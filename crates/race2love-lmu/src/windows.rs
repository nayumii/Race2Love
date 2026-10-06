//! The only unsafe module: Win32 handles, bounded mapped views and the SDK's
//! shared interlocked lock. Nothing is injected into the game. The telemetry
//! mapping is read-only; only the eight-byte synchronization mapping is writable.
#![allow(unsafe_code)]

use std::{
    mem::size_of,
    sync::atomic::{AtomicI32, Ordering},
};

use race2love_core::telemetry::TelemetryError;
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Memory::{
                FILE_MAP, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
                OpenFileMappingW, UnmapViewOfFile,
            },
            Threading::{
                EVENT_MODIFY_STATE, OpenEventW, OpenProcess, PROCESS_SYNCHRONIZE,
                SYNCHRONIZATION_SYNCHRONIZE, SetEvent, WaitForSingleObject,
            },
        },
    },
    core::{PCWSTR, w},
};

use crate::{SnapshotReader, layout::PAYLOAD_SIZE};

fn unavailable(context: &str, error: impl std::fmt::Display) -> TelemetryError {
    TelemetryError::Unavailable(format!("{context}: {error}"))
}

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by a successful Win32 open, and this
        // object uniquely owns it. Views are unmapped before their handle closes.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct Mapping {
    address: MEMORY_MAPPED_VIEW_ADDRESS,
    length: usize,
    _handle: OwnedHandle,
}

// SAFETY: ownership can move between Tokio threads; a mapped view is process-wide
// and not thread-affine. It is never shared concurrently. SDK atomics guard copies.
unsafe impl Send for Mapping {}
// SAFETY: Win32 handles are process-wide. There is a single owning reader.
unsafe impl Send for OwnedHandle {}

impl Mapping {
    fn open(name: PCWSTR, access: FILE_MAP, length: usize) -> Result<Self, TelemetryError> {
        // SAFETY: names are valid terminated UTF-16 strings for this call. We open existing
        // objects only, never fabricate a telemetry or synchronization producer.
        let handle = OwnedHandle(unsafe { OpenFileMappingW(access.0, false, name) }.map_err(
            |error| {
                unavailable(
                    "Cannot open LMU shared memory; enable plugins and start LMU",
                    error,
                )
            },
        )?);
        // SAFETY: offset zero, explicit bounded length; failure is checked before
        // any dereference. MapViewOfFile refuses an insufficient backing section.
        let address = unsafe { MapViewOfFile(handle.0, access, 0, 0, length) };
        if address.Value.is_null() {
            return Err(unavailable(
                "Cannot map LMU shared memory",
                windows::core::Error::from_thread(),
            ));
        }
        Ok(Self {
            address,
            length,
            _handle: handle,
        })
    }

    fn copy_to(&self, destination: &mut [u8]) {
        assert_eq!(destination.len(), self.length);
        // SAFETY: the view remains valid until Drop, the destination has exactly
        // its validated length, and the caller holds the SDK lock. No reference
        // to mutable external telemetry is created or retained.
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.address.Value.cast::<u8>(),
                destination.as_mut_ptr(),
                self.length,
            );
        }
    }

    fn atomic(&self, offset: usize) -> &AtomicI32 {
        assert!(offset + size_of::<AtomicI32>() <= self.length);
        // SAFETY: Windows views are page aligned; offset is 0 or 4, so the
        // SDK's 32-bit interlocked integers have the required alignment. Only
        // atomic access is used, as in the producer's interlocked protocol.
        unsafe {
            &*self
                .address
                .Value
                .cast::<u8>()
                .add(offset)
                .cast::<AtomicI32>()
        }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: uniquely owned, successful mapped view; no slices outlive it.
        unsafe {
            let _ = UnmapViewOfFile(self.address);
        }
    }
}

struct Connection {
    process: OwnedHandle,
    data: Mapping,
    lock: Mapping,
    wake: OwnedHandle,
    hold_gate: OwnedHandle,
    data_gate: OwnedHandle,
}

/// Unlike the SDK's blocking client, this consumer never waits or spins. A busy
/// lock skips one sample; the normal watchdog stops output if contention persists.
struct LockGuard<'a> {
    lock: &'a Mapping,
    wake: &'a OwnedHandle,
}
impl Drop for LockGuard<'_> {
    fn drop(&mut self) {
        self.lock.atomic(4).store(0, Ordering::SeqCst);
        if self.lock.atomic(0).load(Ordering::SeqCst) > 0 {
            // SAFETY: the existing auto-reset SDK event is valid for this guard.
            unsafe {
                let _ = SetEvent(self.wake.0);
            }
        }
    }
}

#[derive(Default)]
pub struct WindowsReader {
    connection: Option<Connection>,
}

impl SnapshotReader for WindowsReader {
    fn connect(&mut self) -> Result<(), TelemetryError> {
        self.disconnect();
        let (pid, process) = find_game()?;
        let data = Mapping::open(w!("LMU_Data"), FILE_MAP_READ, PAYLOAD_SIZE)?;
        let lock = Mapping::open(
            w!("LMU_SharedMemoryLockData"),
            FILE_MAP_READ | FILE_MAP_WRITE,
            8,
        )?;
        // SAFETY: only signaling access to an existing SDK event is requested.
        let wake = OwnedHandle(
            unsafe { OpenEventW(EVENT_MODIFY_STATE, false, w!("LMU_SharedMemoryLockEvent")) }
                .map_err(|error| unavailable("Cannot open LMU synchronization event", error))?,
        );
        // Current shipped SDK requires checking Hold before Data. Read-only
        // synchronization rights; these events are never created or signaled.
        let hold_gate = open_gate(w!("LMU_Data_HoldEvent"))?;
        let data_gate = open_gate(w!("LMU_Data_DataEvent"))?;
        tracing::info!(
            pid,
            mapping = "LMU_Data",
            bytes = PAYLOAD_SIZE,
            "LMU process and SDK mappings found"
        );
        self.connection = Some(Connection {
            process,
            data,
            lock,
            wake,
            hold_gate,
            data_gate,
        });
        Ok(())
    }
    fn disconnect(&mut self) {
        self.connection = None;
    }
    fn is_connected(&self) -> bool {
        self.connection.is_some()
    }
    fn snapshot(&mut self, destination: &mut [u8]) -> Result<bool, TelemetryError> {
        let connection = self
            .connection
            .as_ref()
            .ok_or(TelemetryError::Disconnected)?;
        // SAFETY: process handle has SYNCHRONIZE access; zero is a nonblocking poll.
        match unsafe { WaitForSingleObject(connection.process.0, 0) } {
            WAIT_TIMEOUT => {}
            WAIT_OBJECT_0 => return Err(TelemetryError::Disconnected),
            _ => {
                return Err(unavailable(
                    "Cannot check LMU process lifetime",
                    windows::core::Error::from_thread(),
                ));
            }
        }
        // Preserve the SDK gate order even when polling without waiting. Reading
        // Data first could consume a notification before Hold permits the copy.
        for gate in [&connection.hold_gate, &connection.data_gate] {
            // SAFETY: owned synchronization event handle, zero timeout.
            match unsafe { WaitForSingleObject(gate.0, 0) } {
                WAIT_OBJECT_0 => {}
                WAIT_TIMEOUT => return Ok(false),
                _ => {
                    return Err(unavailable(
                        "Cannot check LMU update gate",
                        windows::core::Error::from_thread(),
                    ));
                }
            }
        }
        if connection
            .lock
            .atomic(4)
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(false);
        }
        let _guard = LockGuard {
            lock: &connection.lock,
            wake: &connection.wake,
        };
        connection.data.copy_to(destination);
        Ok(true)
    }
}

fn open_gate(name: PCWSTR) -> Result<OwnedHandle, TelemetryError> {
    // SAFETY: valid terminated name, synchronization-only access to an existing
    // event. Missing gates indicate a stopped/older/incompatible game producer.
    unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, name) }
        .map(OwnedHandle)
        .map_err(|error| {
            unavailable(
                "Cannot open LMU update gate; this reader requires the current shared-memory SDK",
                error,
            )
        })
}

fn find_game() -> Result<(u32, OwnedHandle), TelemetryError> {
    // SAFETY: documented process enumeration API, no process-memory permissions.
    let snapshot = OwnedHandle(
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|error| unavailable("Cannot discover LMU process", error))?,
    );
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: initialized struct with correct dwSize, owned snapshot handle.
    unsafe { Process32FirstW(snapshot.0, &mut entry) }
        .map_err(|error| unavailable("Cannot enumerate game processes", error))?;
    loop {
        let end = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        if String::from_utf16_lossy(&entry.szExeFile[..end])
            .eq_ignore_ascii_case("Le Mans Ultimate.exe")
        {
            // SAFETY: requesting only termination notification, no injection,
            // memory reads or elevation. PID comes from the live snapshot.
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, entry.th32ProcessID) }
                .map_err(|error| unavailable("Cannot monitor LMU process", error))?;
            return Ok((entry.th32ProcessID, OwnedHandle(handle)));
        }
        // SAFETY: same valid handle and struct as Process32FirstW above.
        if unsafe { Process32NextW(snapshot.0, &mut entry) }.is_err() {
            break;
        }
    }
    Err(TelemetryError::Unavailable(
        "LMU is not running. Start Le Mans Ultimate and enable plugins in Gameplay settings."
            .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::decode;
    use std::{
        process::{Command, Stdio},
        time::{Instant, SystemTime, UNIX_EPOCH},
    };
    use windows::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        System::{
            Memory::{
                CreateFileMappingW, MEMORY_BASIC_INFORMATION, PAGE_READONLY, PAGE_READWRITE,
                VirtualQuery,
            },
            Threading::{CreateEventW, GetCurrentProcessId},
        },
    };

    fn name(suffix: &str) -> Vec<u16> {
        format!(
            "Local\\Race2LoveFixture_{}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            suffix
        )
        .encode_utf16()
        .chain(Some(0))
        .collect()
    }

    fn create(name: &[u16], length: usize) -> Mapping {
        // SAFETY: terminated name, page-file backed fixture owned only by this
        // test. No production LMU names are created or written by tests.
        let handle = OwnedHandle(
            unsafe {
                CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    None,
                    PAGE_READWRITE,
                    0,
                    length as u32,
                    PCWSTR(name.as_ptr()),
                )
            }
            .unwrap(),
        );
        // SAFETY: explicit fixture size and successful handle, checked result.
        let address =
            unsafe { MapViewOfFile(handle.0, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, length) };
        assert!(!address.Value.is_null());
        Mapping {
            address,
            length,
            _handle: handle,
        }
    }

    fn gates() -> (OwnedHandle, OwnedHandle) {
        let hold_name = name("HoldGate");
        let data_name = name("DataGate");
        // SAFETY: isolated fixture names; Hold is a level gate, Data an auto-reset
        // notification so tests detect accidental notification consumption.
        let hold = OwnedHandle(
            unsafe { CreateEventW(None, true, false, PCWSTR(hold_name.as_ptr())) }.unwrap(),
        );
        let data = OwnedHandle(
            unsafe { CreateEventW(None, false, true, PCWSTR(data_name.as_ptr())) }.unwrap(),
        );
        (hold, data)
    }

    #[test]
    fn named_mapping_is_read_only_busy_lock_skips_and_release_wakes_writer() {
        let data_name = name("Data");
        let lock_name = name("Lock");
        let event_name = name("Event");
        let producer = create(&data_name, PAYLOAD_SIZE);
        let producer_lock = create(&lock_name, 8);
        // SAFETY: new test-specific auto-reset event with a terminated name.
        let event = OwnedHandle(
            unsafe { CreateEventW(None, false, false, PCWSTR(event_name.as_ptr())) }.unwrap(),
        );
        let fixture = crate::tests::fixture(5, 42.0);
        // SAFETY: producer owns a writable mapping, sizes agree, no reader exists
        // yet, and the fixture bytes contain no real game data.
        unsafe {
            std::ptr::copy_nonoverlapping(
                fixture.as_ptr(),
                producer.address.Value.cast(),
                PAYLOAD_SIZE,
            );
        }
        // SAFETY: synchronize-only handle to the current live test process.
        let process = OwnedHandle(
            unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, GetCurrentProcessId()) }.unwrap(),
        );
        let data = Mapping::open(PCWSTR(data_name.as_ptr()), FILE_MAP_READ, PAYLOAD_SIZE).unwrap();
        let mut info = MEMORY_BASIC_INFORMATION::default();
        // SAFETY: live view and valid initialized output struct.
        assert_ne!(
            unsafe {
                VirtualQuery(
                    Some(data.address.Value),
                    &mut info,
                    size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            },
            0
        );
        assert_eq!(info.Protect, PAGE_READONLY);
        let lock = Mapping::open(
            PCWSTR(lock_name.as_ptr()),
            FILE_MAP_READ | FILE_MAP_WRITE,
            8,
        )
        .unwrap();
        // SAFETY: signaling-only handle to the test's existing event.
        let wake = OwnedHandle(
            unsafe { OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(event_name.as_ptr())) }.unwrap(),
        );
        let (hold_gate, data_gate) = gates();
        let hold_handle = hold_gate.0;
        let data_handle = data_gate.0;
        let mut reader = WindowsReader {
            connection: Some(Connection {
                process,
                data,
                lock,
                wake,
                hold_gate,
                data_gate,
            }),
        };
        let mut bytes = vec![0; PAYLOAD_SIZE];
        assert!(!reader.snapshot(&mut bytes).unwrap());
        // SAFETY: owned fixture handle. Data remains signaled because a closed
        // Hold gate must be checked first; opening Hold now permits the copy.
        unsafe {
            SetEvent(hold_handle).unwrap();
        }
        assert!(reader.snapshot(&mut bytes).unwrap());
        assert!(!reader.snapshot(&mut bytes).unwrap()); // notification consumed
        // SAFETY: fixture producer publishes another notification.
        unsafe {
            SetEvent(data_handle).unwrap();
        }
        producer_lock.atomic(4).store(1, Ordering::SeqCst);
        assert!(!reader.snapshot(&mut bytes).unwrap());
        producer_lock.atomic(4).store(0, Ordering::SeqCst);
        producer_lock.atomic(0).store(1, Ordering::SeqCst);
        // SAFETY: fixture producer publishes the next update after contention.
        unsafe {
            SetEvent(data_handle).unwrap();
        }
        assert!(reader.snapshot(&mut bytes).unwrap());
        assert_eq!(producer_lock.atomic(4).load(Ordering::SeqCst), 0);
        // SAFETY: valid event handle; this checks that the reader woke a waiter.
        assert_eq!(unsafe { WaitForSingleObject(event.0, 0) }, WAIT_OBJECT_0);
        let frame = decode(&bytes, Instant::now()).unwrap().unwrap();
        assert_eq!(frame.vehicle_id, 5);
        assert_eq!(frame.frame.engine_rpm, 7_000.0);
        reader.disconnect();
        assert!(!reader.is_connected());
        assert!(reader.snapshot(&mut bytes).is_err());
    }

    #[test]
    fn missing_and_undersized_sections_fail_before_copying() {
        let missing = name("Missing");
        assert!(Mapping::open(PCWSTR(missing.as_ptr()), FILE_MAP_READ, PAYLOAD_SIZE).is_err());
        let small_name = name("Small");
        let _small = create(&small_name, 8);
        assert!(Mapping::open(PCWSTR(small_name.as_ptr()), FILE_MAP_READ, PAYLOAD_SIZE).is_err());
    }

    #[test]
    fn exited_producer_is_detected_even_while_the_mapping_is_retained() {
        // This command blocks on its piped stdin and never launches LMU or sends
        // network traffic. The child exists solely to exercise a process handle.
        let mut child = Command::new("cmd.exe")
            .args(["/D", "/Q", "/C", "set /p race2love_fixture="])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // SAFETY: child PID and only termination-notification rights.
        let process =
            OwnedHandle(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, child.id()) }.unwrap());
        let data_name = name("RetainedData");
        let lock_name = name("RetainedLock");
        let event_name = name("RetainedEvent");
        let _producer = create(&data_name, PAYLOAD_SIZE);
        let _producer_lock = create(&lock_name, 8);
        // SAFETY: unique terminated fixture event name.
        let event = OwnedHandle(
            unsafe { CreateEventW(None, false, false, PCWSTR(event_name.as_ptr())) }.unwrap(),
        );
        let (hold_gate, data_gate) = gates();
        let mut reader = WindowsReader {
            connection: Some(Connection {
                process,
                data: Mapping::open(PCWSTR(data_name.as_ptr()), FILE_MAP_READ, PAYLOAD_SIZE)
                    .unwrap(),
                lock: Mapping::open(
                    PCWSTR(lock_name.as_ptr()),
                    FILE_MAP_READ | FILE_MAP_WRITE,
                    8,
                )
                .unwrap(),
                wake: event,
                hold_gate,
                data_gate,
            }),
        };
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(matches!(
            reader.snapshot(&mut vec![0; PAYLOAD_SIZE]),
            Err(TelemetryError::Disconnected)
        ));
        reader.disconnect();
    }
}
