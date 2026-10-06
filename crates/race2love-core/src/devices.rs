//! Device boundary: protocols convert normalized intensity only in their adapters.

use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

use thiserror::Error;

use crate::unit;

#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("Haptic device is disconnected")]
    Disconnected,
    #[error("Device request timed out")]
    Timeout,
    #[error("Device communication failed: {0}")]
    Communication(String),
}

/// Boxed futures keep the adapter object-safe without an async-trait dependency.
pub type DeviceFuture<'a> = Pin<Box<dyn Future<Output = Result<(), DeviceError>> + Send + 'a>>;

/// stop() must be idempotent, including after connection loss. Physical backends
/// must additionally use short command leases to bound output after a crash.
pub trait HapticDevice: Send + Sync {
    fn name(&self) -> &str;
    fn is_connected(&self) -> bool;
    fn set_vibration(&self, intensity: f32) -> DeviceFuture<'_>;
    /// Absolute instantaneous ceiling, separate from a possibly fractional target.
    /// Backends using temporal dithering must cap every emitted native level.
    fn set_vibration_with_limit(&self, intensity: f32, ceiling: f32) -> DeviceFuture<'_> {
        self.set_vibration(unit(intensity).min(unit(ceiling)))
    }
    fn stop(&self) -> DeviceFuture<'_>;
    /// Renew unchanged positive output before the device's finite command expires.
    fn refresh_interval(&self) -> Option<std::time::Duration> {
        None
    }
    /// Backends may round DOWN to their command resolution. A backend that
    /// schedules fractional averages may retain finer target precision here.
    fn quantize(&self, intensity: f32) -> f32 {
        (unit(intensity) * 100.0).floor() / 100.0
    }
    /// Changes even if a disconnect/reconnect completes between worker samples.
    fn connection_epoch(&self) -> u64 {
        0
    }
    /// Physical backends require explicit Resume after every new connection.
    fn requires_resume_on_connect(&self) -> bool {
        false
    }
}

/// In-memory device for Demo and tests. It never discovers or commands real toys.
/// Only a current value and counters are retained, so memory usage is constant.
pub struct MockDevice {
    connected: AtomicBool,
    intensity_bits: AtomicU32,
    commands: AtomicU64,
    stops: AtomicU64,
}

impl Default for MockDevice {
    fn default() -> Self {
        Self {
            connected: AtomicBool::new(true),
            intensity_bits: AtomicU32::new(0.0_f32.to_bits()),
            commands: AtomicU64::new(0),
            stops: AtomicU64::new(0),
        }
    }
}

impl MockDevice {
    pub fn intensity(&self) -> f32 {
        f32::from_bits(self.intensity_bits.load(Ordering::Acquire))
    }
    pub fn command_count(&self) -> u64 {
        self.commands.load(Ordering::Relaxed)
    }
    pub fn stop_count(&self) -> u64 {
        self.stops.load(Ordering::Relaxed)
    }
    pub fn disconnect(&self) {
        self.connected.store(false, Ordering::Release);
        self.intensity_bits
            .store(0.0_f32.to_bits(), Ordering::Release);
    }
    pub fn reconnect(&self) {
        self.intensity_bits
            .store(0.0_f32.to_bits(), Ordering::Release);
        self.connected.store(true, Ordering::Release);
    }
}

impl HapticDevice for MockDevice {
    fn name(&self) -> &str {
        "Demo output (mock device)"
    }
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }
    fn set_vibration(&self, intensity: f32) -> DeviceFuture<'_> {
        Box::pin(async move {
            if !self.is_connected() {
                return Err(DeviceError::Disconnected);
            }
            self.intensity_bits
                .store(unit(intensity).to_bits(), Ordering::Release);
            self.commands.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
    }
    fn stop(&self) -> DeviceFuture<'_> {
        Box::pin(async move {
            self.intensity_bits
                .store(0.0_f32.to_bits(), Ordering::Release);
            self.stops.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_clamps_and_stops_even_when_disconnected() {
        let device = MockDevice::default();
        device.set_vibration(2.0).await.unwrap();
        assert_eq!(device.intensity(), 1.0);
        device.set_vibration(f32::NAN).await.unwrap();
        assert_eq!(device.intensity(), 0.0);
        device.disconnect();
        assert!(device.set_vibration(0.5).await.is_err());
        device.stop().await.unwrap();
        device.reconnect();
        assert_eq!(device.intensity(), 0.0);
        assert_eq!(device.command_count(), 2);
    }
}
