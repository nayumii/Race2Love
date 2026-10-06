//! Direct LMU adapter. Platform access supplies owned byte snapshots; one safe
//! parser normalizes them for every platform. See docs/LMU.md for the contract.

pub mod decoder;
pub mod layout;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

use std::time::{Duration, Instant};

use race2love_core::telemetry::{TelemetryError, TelemetryFrame, TelemetrySource};

/// Acquisition boundary shared by Windows, Proton and fixture tests.
/// Snapshots are bounded and owned. Windows uses the SDK lock; Proton performs
/// repeated read-only consistency checks (see docs/LMU.md for their limitations).
pub trait SnapshotReader: Send {
    fn connect(&mut self) -> Result<(), TelemetryError>;
    fn disconnect(&mut self);
    fn is_connected(&self) -> bool;
    /// `false` means temporarily busy. Producer exit must return an error.
    fn snapshot(&mut self, destination: &mut [u8]) -> Result<bool, TelemetryError>;
}

pub struct LmuSource<R: SnapshotReader> {
    reader: R,
    bytes: Vec<u8>,
    last_tick: Option<(i32, u64)>,
    last_progress: Instant,
    logged_version: Option<i32>,
    previous_suspension: Option<(f64, [f32; 4])>,
}

impl<R: SnapshotReader> LmuSource<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            bytes: vec![0; layout::PAYLOAD_SIZE],
            last_tick: None,
            last_progress: Instant::now(),
            logged_version: None,
            previous_suspension: None,
        }
    }
}

impl<R: SnapshotReader> TelemetrySource for LmuSource<R> {
    fn name(&self) -> &'static str {
        "Le Mans Ultimate"
    }
    fn connect(&mut self) -> Result<(), TelemetryError> {
        self.disconnect();
        self.reader.connect()?;
        self.last_progress = Instant::now();
        Ok(())
    }
    fn disconnect(&mut self) {
        self.reader.disconnect();
        self.last_tick = None;
        self.logged_version = None;
        self.previous_suspension = None;
    }
    fn is_connected(&self) -> bool {
        self.reader.is_connected()
    }
    fn read_frame(&mut self) -> Result<Option<TelemetryFrame>, TelemetryError> {
        if !self.is_connected() {
            return Err(TelemetryError::Disconnected);
        }
        let now = Instant::now();
        let result = self.read_at(now);
        if result
            .as_ref()
            .is_err_and(|error| !matches!(error, TelemetryError::Waiting(_)))
        {
            self.disconnect();
        }
        result
    }
}

impl<R: SnapshotReader> LmuSource<R> {
    fn read_at(&mut self, now: Instant) -> Result<Option<TelemetryFrame>, TelemetryError> {
        if now.saturating_duration_since(self.last_progress) >= Duration::from_secs(2) {
            return Err(TelemetryError::Unavailable(
                "LMU telemetry froze; waiting for a fresh mapping".into(),
            ));
        }
        if !self.reader.snapshot(&mut self.bytes)? {
            return Ok(None);
        }
        let decoded = decoder::decode(&self.bytes, now)
            .map_err(|error| TelemetryError::Unavailable(error.to_string()))?;
        let Some(mut decoded) = decoded else {
            self.last_tick = None;
            self.previous_suspension = None;
            self.last_progress = now;
            return Err(TelemetryError::Waiting(
                "LMU is not publishing a live player car. Enter the car in a driving session."
                    .into(),
            ));
        };
        if self.logged_version != Some(decoded.game_version) {
            tracing::info!(game_version = decoded.game_version, "LMU layout accepted");
            self.logged_version = Some(decoded.game_version);
        }
        let tick = (decoded.vehicle_id, decoded.elapsed_seconds.to_bits());
        if self.last_tick == Some(tick) {
            return Ok(None);
        }
        let discontinuity = self.last_tick.is_some_and(|(id, time)| {
            id != decoded.vehicle_id || decoded.elapsed_seconds < f64::from_bits(time)
        });
        if discontinuity {
            self.previous_suspension = None;
        }
        if let Some(current) = decoded.suspension_deflection {
            if let Some((time, previous)) = self.previous_suspension {
                let dt = decoded.elapsed_seconds - time;
                if (1.0 / 240.0..=0.25).contains(&dt) {
                    let velocity =
                        std::array::from_fn(|index| (current[index] - previous[index]) / dt as f32);
                    if velocity.iter().all(|v| v.is_finite() && v.abs() <= 20.0) {
                        decoded.frame.suspension_velocity = Some(velocity);
                    }
                }
            }
            self.previous_suspension = Some((decoded.elapsed_seconds, current));
        } else {
            self.previous_suspension = None;
        }
        let first = self.last_tick.replace(tick).is_none();
        self.last_progress = now;
        if discontinuity {
            return Err(TelemetryError::Waiting(
                "LMU player/session changed; waiting for fresh telemetry".into(),
            ));
        }
        // Require observed progress after every open. A retained frozen mapping
        // must not become fresh again simply because we reconnected to it.
        Ok((!first).then_some(decoded.frame))
    }
}

/// Creates the native source, including a human-readable unavailable source on
/// platforms whose acquisition adapter has not been implemented yet.
pub fn native_source() -> Box<dyn TelemetrySource> {
    #[cfg(windows)]
    {
        Box::new(LmuSource::new(windows::WindowsReader::default()))
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(LmuSource::new(linux::LinuxReader::default()))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Box::new(UnsupportedSource)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
struct UnsupportedSource;
#[cfg(not(any(windows, target_os = "linux")))]
impl TelemetrySource for UnsupportedSource {
    fn name(&self) -> &'static str {
        "Le Mans Ultimate"
    }
    fn connect(&mut self) -> Result<(), TelemetryError> {
        Err(TelemetryError::Unavailable(
            "LMU telemetry is supported on Windows and Linux/Proton. Choose Demo on this platform."
                .into(),
        ))
    }
    fn disconnect(&mut self) {}
    fn is_connected(&self) -> bool {
        false
    }
    fn read_frame(&mut self) -> Result<Option<TelemetryFrame>, TelemetryError> {
        Err(TelemetryError::Disconnected)
    }
}

#[cfg(test)]
mod tests;
