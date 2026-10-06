//! Adapter boundary: normalized SI values and optional, explicitly sourced signals.

mod demo;
pub use demo::DemoSource;

use std::time::Instant;

use thiserror::Error;

#[derive(Clone, Debug)]
pub struct TelemetryFrame {
    /// Time the source observed this sample; re-reading it must not refresh this.
    pub timestamp: Instant,
    pub speed_mps: f32,
    pub engine_rpm: f32,
    pub engine_max_rpm: f32,
    /// -1 reverse, 0 neutral, positive values forward gears.
    pub gear: i8,
    pub throttle: f32,
    pub brake: f32,
    /// Dimensionless slip ratio, ordered front-left, front-right, rear-left, rear-right.
    pub wheel_slip: Option<[f32; 4]>,
    /// Suspension travel velocity in m/s, in the same wheel order.
    pub suspension_velocity: Option<[f32; 4]>,
    /// Vertical acceleration in m/s², excluding gravity where the source permits.
    pub vertical_acceleration: Option<f32>,
    /// Explicit impact severity in 0..=1; None unless the adapter has a reliable signal.
    pub impact: Option<f32>,
    pub session: Option<String>,
    pub car: Option<String>,
}

impl Default for TelemetryFrame {
    fn default() -> Self {
        Self {
            timestamp: Instant::now(),
            speed_mps: 0.0,
            engine_rpm: 0.0,
            engine_max_rpm: 0.0,
            gear: 0,
            throttle: 0.0,
            brake: 0.0,
            wheel_slip: None,
            suspension_velocity: None,
            vertical_acceleration: None,
            impact: None,
            session: None,
            car: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum TelemetryError {
    #[error("Telemetry source is not connected")]
    Disconnected,
    #[error("Telemetry is unavailable: {0}")]
    Unavailable(String),
}

/// Reads run on a background task, never in the GUI. Implementations must perform
/// bounded, quick reads; expensive process discovery belongs in a blocking worker.
/// None means no fresh sample. Game closure must be reported as disconnection.
pub trait TelemetrySource: Send {
    fn name(&self) -> &'static str;
    fn connect(&mut self) -> Result<(), TelemetryError>;
    fn disconnect(&mut self);
    fn is_connected(&self) -> bool;
    fn read_frame(&mut self) -> Result<Option<TelemetryFrame>, TelemetryError>;
}

/// Controllable source for pipeline tests and downstream adapter development.
#[derive(Default)]
pub struct MockTelemetrySource {
    connected: bool,
    pub frame: Option<TelemetryFrame>,
}

impl MockTelemetrySource {
    pub fn with_frame(frame: TelemetryFrame) -> Self {
        Self {
            frame: Some(frame),
            ..Self::default()
        }
    }
}

impl TelemetrySource for MockTelemetrySource {
    fn name(&self) -> &'static str {
        "Mock telemetry"
    }
    fn connect(&mut self) -> Result<(), TelemetryError> {
        self.connected = true;
        Ok(())
    }
    fn disconnect(&mut self) {
        self.connected = false;
    }
    fn is_connected(&self) -> bool {
        self.connected
    }
    fn read_frame(&mut self) -> Result<Option<TelemetryFrame>, TelemetryError> {
        if !self.connected {
            return Err(TelemetryError::Disconnected);
        }
        Ok(self.frame.take())
    }
}
