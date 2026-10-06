//! Direct LMU adapter. Platform access supplies owned byte snapshots; one safe
//! parser normalizes them for every platform. See docs/LMU.md for the contract.

pub mod decoder;
pub mod layout;
#[cfg(windows)]
mod windows;

use std::time::{Duration, Instant};

use race2love_core::telemetry::{TelemetryError, TelemetryFrame, TelemetrySource};

/// Acquisition boundary reused by the future Proton adapter and fixture tests.
/// A successful snapshot must be bounded, owned and synchronized with its writer.
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
}

impl<R: SnapshotReader> LmuSource<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            bytes: vec![0; layout::PAYLOAD_SIZE],
            last_tick: None,
            last_progress: Instant::now(),
            logged_version: None,
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
        let Some(decoded) = decoded else {
            self.last_tick = None;
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
        let first = self.last_tick.replace(tick).is_none();
        self.last_progress = now;
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
    #[cfg(not(windows))]
    {
        Box::new(UnsupportedSource)
    }
}

#[cfg(not(windows))]
struct UnsupportedSource;
#[cfg(not(windows))]
impl TelemetrySource for UnsupportedSource {
    fn name(&self) -> &'static str {
        "Le Mans Ultimate"
    }
    fn connect(&mut self) -> Result<(), TelemetryError> {
        Err(TelemetryError::Unavailable(
            "LMU under Linux/Proton is planned for Phase 4. Choose Demo on this build.".into(),
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
