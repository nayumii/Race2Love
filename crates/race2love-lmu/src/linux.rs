//! Read-only Proton acquisition through /proc/PID/fd. Positioned reads avoid
//! mmap's SIGBUS risk if a producer truncates its file. Wine's anonymous backing
//! does not expose the Windows object name or SDK lock: verify layout and repeat
//! the relevant reads, then let the shared decoder/freshness watchdog fail closed.

mod proc;
#[cfg(test)]
mod tests;

use std::{
    collections::HashSet,
    fs::{self, File},
    io,
    os::unix::fs::{FileExt, MetadataExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use race2love_core::telemetry::TelemetryError;

use crate::{SnapshotReader, decoder, layout::*};
use proc::Identity;

// Wine rounds anonymous sections to 4 KiB; no arbitrary large-file scanning.
const MAX_BACKING_SIZE: u64 = ALLOCATION_SIZE.div_ceil(4096) as u64 * 4096;
const SCORING_PREFIX: usize = 116;
const VEHICLE_PREFIX: usize = MAX_RPM + 8;
const MAX_CANDIDATES: usize = 16;

pub(crate) struct LinuxReader {
    root: PathBuf,
    mapping: Option<Mapping>,
}

impl Default for LinuxReader {
    fn default() -> Self {
        Self {
            root: PathBuf::from("/proc"),
            mapping: None,
        }
    }
}

impl SnapshotReader for LinuxReader {
    fn connect(&mut self) -> Result<(), TelemetryError> {
        self.disconnect();
        let processes = proc::discover(&self.root).map_err(unavailable)?.ok_or_else(|| {
            TelemetryError::Unavailable("LMU is not running or its /proc entries are inaccessible. Start LMU through Proton as the same user.".into())
        })?;
        tracing::debug!(pid = processes.game.pid, "Detected LMU under Proton");
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        let mut access_denied = false;
        for owner in processes.owners {
            let paths = match proc::fd_paths(&self.root, owner) {
                Ok(paths) => paths,
                Err(error) => {
                    access_denied |= error.kind() == io::ErrorKind::PermissionDenied;
                    continue;
                }
            };
            for path in paths {
                let file = match File::open(&path) {
                    Ok(file) => file,
                    Err(error) => {
                        access_denied |= error.kind() == io::ErrorKind::PermissionDenied;
                        continue;
                    }
                };
                let Ok(meta) = file.metadata() else { continue };
                if !meta.is_file()
                    || !(ALLOCATION_SIZE as u64..=MAX_BACKING_SIZE).contains(&meta.len())
                {
                    continue;
                }
                let identity = (meta.dev(), meta.ino());
                if !seen.insert(identity) {
                    continue;
                }
                let mapping = Mapping {
                    file,
                    path,
                    owner,
                    game: processes.game,
                    identity,
                };
                let mut bytes = vec![0; PAYLOAD_SIZE];
                if mapping.check(&self.root).is_err() {
                    continue;
                }
                if let Ok(true) = snapshot(&mapping.file, &mut bytes)
                    && let Ok(decoded) = decoder::decode(&bytes, Instant::now())
                {
                    let tick =
                        decoded.map(|frame| (frame.vehicle_id, frame.elapsed_seconds.to_bits()));
                    candidates.push((mapping, tick));
                }
                if candidates.len() >= MAX_CANDIDATES {
                    break;
                }
            }
            if candidates.len() >= MAX_CANDIDATES {
                break;
            }
        }
        let mapping = choose_candidate(candidates).ok_or_else(|| TelemetryError::Unavailable(
            if access_denied {
                "Cannot read LMU's /proc file descriptors. Run as the same user and check ptrace/hidepid or sandbox restrictions."
            } else {
                "No supported LMU telemetry mapping found. Enable Plugins in LMU, restart it, and enter a driving session. This Proton build must expose Wine mapping file descriptors."
            }.into()
        ))?;
        mapping.check(&self.root)?;
        tracing::info!(game_pid = mapping.game.pid, owner_pid = mapping.owner.pid,
            fd = %mapping.path.display(), "Discovered LMU Wine shared-memory backing");
        self.mapping = Some(mapping);
        Ok(())
    }

    fn disconnect(&mut self) {
        self.mapping = None;
    }
    fn is_connected(&self) -> bool {
        self.mapping.is_some()
    }

    fn snapshot(&mut self, destination: &mut [u8]) -> Result<bool, TelemetryError> {
        let mapping = self.mapping.as_ref().ok_or(TelemetryError::Disconnected)?;
        mapping.check(&self.root)?;
        let accepted = snapshot(&mapping.file, destination).map_err(unavailable)?;
        // Detect exit/replacement during the copy too; owned handles can otherwise
        // keep dead telemetry alive. Core separately rejects unchanged clocks.
        mapping.check(&self.root)?;
        Ok(accepted)
    }
}

struct Mapping {
    file: File,
    path: PathBuf,
    owner: Identity,
    game: Identity,
    identity: (u64, u64),
}

impl Mapping {
    fn check(&self, root: &Path) -> Result<(), TelemetryError> {
        if !self.game.is_alive(root) || (self.owner != self.game && !self.owner.is_alive(root)) {
            return Err(TelemetryError::Disconnected);
        }
        let current = fs::metadata(&self.path).map_err(unavailable)?;
        if (current.dev(), current.ino()) != self.identity {
            return Err(TelemetryError::Unavailable(
                "LMU mapping was replaced; rediscovering".into(),
            ));
        }
        if !(ALLOCATION_SIZE as u64..=MAX_BACKING_SIZE).contains(&current.len()) {
            return Err(TelemetryError::Unavailable(
                "LMU mapping size changed; rediscovering".into(),
            ));
        }
        Ok(())
    }
}

/// Disambiguate retained copies by simulation-clock progress, not RPM changes:
/// a stationary/idling car is still live. Only discovery sleeps, on its isolated
/// worker, for at most 150 ms; steady-state reads never wait or spin.
fn choose_candidate(mut candidates: Vec<(Mapping, Option<(i32, u64)>)>) -> Option<Mapping> {
    if candidates.len() > 1 && candidates.iter().any(|(_, tick)| tick.is_some()) {
        let started = Instant::now();
        let mut bytes = vec![0; PAYLOAD_SIZE];
        while started.elapsed() < Duration::from_millis(150) {
            std::thread::sleep(Duration::from_millis(10));
            for index in 0..candidates.len() {
                let (mapping, previous) = &candidates[index];
                if snapshot(&mapping.file, &mut bytes).unwrap_or(false)
                    && let Ok(Some(frame)) = decoder::decode(&bytes, Instant::now())
                {
                    let tick = (frame.vehicle_id, frame.elapsed_seconds.to_bits());
                    if previous.is_some_and(|previous| previous != tick) {
                        return Some(candidates.swap_remove(index).0);
                    }
                }
            }
        }
    }
    // Menu/frozen mappings can connect, but never create fresh haptic frames.
    candidates.into_iter().next().map(|(mapping, _)| mapping)
}

#[derive(PartialEq, Eq)]
struct Sample {
    version: [u8; 4],
    scoring: [u8; SCORING_PREFIX],
    header: [u8; 3],
    vehicle: [u8; VEHICLE_PREFIX],
}

impl Sample {
    fn read(mut read: impl FnMut(&mut [u8], u64) -> io::Result<()>) -> io::Result<Self> {
        let mut sample = Self {
            version: [0; 4],
            scoring: [0; SCORING_PREFIX],
            header: [0; 3],
            vehicle: [0; VEHICLE_PREFIX],
        };
        read(&mut sample.version, GAME_VERSION as u64)?;
        read(&mut sample.scoring, SCORING as u64)?;
        read(&mut sample.header, TELEMETRY as u64)?;
        let player = usize::from(sample.header[1]);
        // Invalid header still reaches the decoder, without unsafe indexing/read.
        if player < MAX_VEHICLES && sample.header[2] == 1 {
            read(
                &mut sample.vehicle,
                (VEHICLES + player * VEHICLE_SIZE) as u64,
            )?;
        }
        Ok(sample)
    }

    fn copy_to(&self, bytes: &mut [u8]) {
        bytes[GAME_VERSION..GAME_VERSION + 4].copy_from_slice(&self.version);
        bytes[SCORING..SCORING + SCORING_PREFIX].copy_from_slice(&self.scoring);
        bytes[TELEMETRY..TELEMETRY + 3].copy_from_slice(&self.header);
        let player = usize::from(self.header[1]);
        if player < MAX_VEHICLES && self.header[2] == 1 {
            let base = VEHICLES + player * VEHICLE_SIZE;
            bytes[base..base + VEHICLE_PREFIX].copy_from_slice(&self.vehicle);
        }
    }
}

fn snapshot(file: &File, destination: &mut [u8]) -> io::Result<bool> {
    consistent_snapshot(
        |bytes, offset| file.read_exact_at(bytes, offset),
        destination,
    )
}

fn consistent_snapshot(
    mut read: impl FnMut(&mut [u8], u64) -> io::Result<()>,
    destination: &mut [u8],
) -> io::Result<bool> {
    if destination.len() < PAYLOAD_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "snapshot buffer is too small",
        ));
    }
    let first = Sample::read(&mut read)?;
    let second = Sample::read(read)?;
    if first != second {
        return Ok(false);
    }
    second.copy_to(destination);
    Ok(true)
}

fn unavailable(error: io::Error) -> TelemetryError {
    tracing::debug!(%error, "Proton telemetry acquisition failed");
    TelemetryError::Unavailable(format!("Cannot read LMU Proton telemetry: {error}"))
}
