//! Independent telemetry, effect, and device tasks connected with latest-value
//! watch channels. Slow adapters cannot build an unbounded queue of old frames.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::{
    sync::watch,
    task::JoinHandle,
    time::{MissedTickBehavior, interval, timeout},
};

use crate::{
    config::{Config, ConfigError},
    devices::{DeviceError, HapticDevice},
    effects::EffectEngine,
    scale_output,
    telemetry::{TelemetryFrame, TelemetrySource},
    unit,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controls {
    pub source_enabled: bool,
    pub emergency_stopped: bool,
    pub test_pulse: Option<TestPulse>,
    pub resume_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TestPulse {
    pub until: Instant,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            source_enabled: true,
            emergency_stopped: false,
            test_pulse: None,
            resume_epoch: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TelemetrySnapshot {
    pub source_name: &'static str,
    pub connected: bool,
    pub frame: Option<TelemetryFrame>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    Running,
    EmergencyStop,
    SourceDisabled,
    NoTelemetry,
    StaleTelemetry,
    Shutdown,
}

impl StopReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::EmergencyStop => "Emergency stop latched",
            Self::SourceDisabled => "Telemetry paused",
            Self::NoTelemetry => "Waiting for telemetry",
            Self::StaleTelemetry => "Telemetry timed out",
            Self::Shutdown => "Shutting down",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EffectSnapshot {
    pub mixed: f32,
    pub intensity: f32,
    pub reason: StopReason,
    /// Independent output watchdog detects a stalled effects worker.
    pub heartbeat: Instant,
}

#[derive(Clone, Debug)]
pub struct DeviceSnapshot {
    pub name: String,
    pub connected: bool,
    /// Last successfully applied intensity, not merely a desired value.
    pub intensity: f32,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RuntimeSnapshot {
    pub telemetry: TelemetrySnapshot,
    pub effects: EffectSnapshot,
    pub device: DeviceSnapshot,
    pub controls: Controls,
}

#[derive(Clone)]
pub struct RuntimeControl {
    config: watch::Sender<Arc<Config>>,
    controls: watch::Sender<Controls>,
    shutdown: watch::Sender<bool>,
    telemetry: watch::Receiver<TelemetrySnapshot>,
    effects: watch::Receiver<EffectSnapshot>,
    device: watch::Receiver<DeviceSnapshot>,
    selected_device: watch::Sender<Arc<dyn HapticDevice>>,
}

impl RuntimeControl {
    pub fn snapshot(&self) -> RuntimeSnapshot {
        RuntimeSnapshot {
            telemetry: self.telemetry.borrow().clone(),
            effects: *self.effects.borrow(),
            device: self.device.borrow().clone(),
            controls: *self.controls.borrow(),
        }
    }
    pub fn update_config(&self, config: Config) -> Result<(), ConfigError> {
        config.validate()?;
        self.config.send_replace(Arc::new(config));
        Ok(())
    }
    pub fn set_source_enabled(&self, enabled: bool) {
        self.controls.send_modify(|controls| {
            controls.source_enabled = enabled;
            controls.test_pulse = None;
        });
    }
    /// Latched until an explicit resume; settings changes cannot undo it.
    pub fn emergency_stop(&self) {
        self.controls.send_modify(|controls| {
            controls.emergency_stopped = true;
            controls.test_pulse = None;
        });
    }
    pub fn resume(&self) {
        self.controls.send_modify(|controls| {
            controls.emergency_stopped = false;
            controls.test_pulse = None;
            controls.resume_epoch = controls.resume_epoch.wrapping_add(1);
        });
    }
    /// Latest selection replaces old requests; the output worker stops the old device.
    pub fn select_device(&self, device: Arc<dyn HapticDevice>) {
        self.emergency_stop();
        self.selected_device.send_replace(device);
    }
    /// Deliberate manual output independent of game telemetry. Demo stays paused
    /// after the pulse; emergency stop must be explicitly resumed beforehand.
    pub fn test_vibration(&self) -> bool {
        let mut accepted = false;
        self.controls.send_modify(|controls| {
            if !controls.emergency_stopped {
                controls.source_enabled = false;
                controls.test_pulse = Some(TestPulse {
                    until: Instant::now() + Duration::from_secs(1),
                });
                accepted = true;
            }
        });
        accepted
    }
    pub fn request_shutdown(&self) {
        self.emergency_stop();
        self.shutdown.send_replace(true);
    }
}

/// Owns tasks. Call shutdown().await after the native window exits. Drop also
/// signals cancellation, but awaiting is required to confirm the final stop.
pub struct RaceRuntime {
    pub control: RuntimeControl,
    tasks: Vec<JoinHandle<()>>,
}

impl RaceRuntime {
    /// Must be called from a Tokio runtime. Phase 1 supplies Demo + MockDevice.
    pub fn spawn(
        source: Box<dyn TelemetrySource>,
        device: Arc<dyn HapticDevice>,
        config: Config,
    ) -> Result<Self, ConfigError> {
        config.validate()?;
        let (config_tx, config_rx) = watch::channel(Arc::new(config));
        let (controls_tx, controls_rx) = watch::channel(Controls::default());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (telemetry_tx, telemetry_rx) = watch::channel(TelemetrySnapshot {
            source_name: source.name(),
            connected: false,
            frame: None,
            error: None,
        });
        let (effects_tx, effects_rx) = watch::channel(EffectSnapshot {
            mixed: 0.0,
            intensity: 0.0,
            reason: StopReason::NoTelemetry,
            heartbeat: Instant::now(),
        });
        let (device_tx, device_rx) = watch::channel(DeviceSnapshot {
            name: device.name().into(),
            connected: false,
            intensity: 0.0,
            error: None,
        });
        let (selected_device_tx, selected_device_rx) = watch::channel(device.clone());
        let tasks = vec![
            tokio::spawn(telemetry_task(
                source,
                telemetry_tx,
                config_rx.clone(),
                controls_rx.clone(),
                shutdown_rx.clone(),
            )),
            tokio::spawn(effects_task(
                telemetry_rx.clone(),
                effects_tx,
                config_rx.clone(),
                controls_rx.clone(),
                shutdown_rx.clone(),
            )),
            tokio::spawn(output_task(
                device,
                effects_rx.clone(),
                telemetry_rx.clone(),
                device_tx,
                config_rx,
                controls_rx,
                shutdown_rx,
                selected_device_rx,
                controls_tx.clone(),
            )),
        ];
        Ok(Self {
            control: RuntimeControl {
                config: config_tx,
                controls: controls_tx,
                shutdown: shutdown_tx,
                telemetry: telemetry_rx,
                effects: effects_rx,
                device: device_rx,
                selected_device: selected_device_tx,
            },
            tasks,
        })
    }

    pub async fn shutdown(mut self) {
        self.control.request_shutdown();
        for task in self.tasks.drain(..) {
            if let Err(error) = task.await {
                tracing::error!(%error, "Background task failed during shutdown");
            }
        }
    }
}

impl Drop for RaceRuntime {
    fn drop(&mut self) {
        self.control.request_shutdown();
    }
}

fn ticker(hz: u32) -> tokio::time::Interval {
    let mut ticker = interval(Duration::from_secs_f64(1.0 / f64::from(hz.clamp(1, 120))));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    ticker
}

/// Future-dated and missing samples fail closed.
pub fn telemetry_is_fresh(
    frame: Option<&TelemetryFrame>,
    now: Instant,
    maximum_age: Duration,
) -> bool {
    frame
        .and_then(|frame| now.checked_duration_since(frame.timestamp))
        .is_some_and(|age| age < maximum_age)
}

async fn telemetry_task(
    mut source: Box<dyn TelemetrySource>,
    state: watch::Sender<TelemetrySnapshot>,
    mut config: watch::Receiver<Arc<Config>>,
    mut controls: watch::Receiver<Controls>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut clock = ticker(config.borrow().output.telemetry_hz);
    let mut clock_hz = config.borrow().output.telemetry_hz;
    let mut connection_attempted = false;
    let mut snapshot = state.borrow().clone();
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            result = controls.changed() => {
                if result.is_err() { break; }
                if !controls.borrow().source_enabled { connection_attempted = false; }
            }
            result = config.changed() => {
                if result.is_err() { break; }
                clock_hz = config.borrow().output.telemetry_hz;
                clock = ticker(clock_hz);
            }
            _ = clock.tick() => {}
        }
        if *shutdown.borrow() {
            break;
        }
        if !controls.borrow().source_enabled {
            if source.is_connected() {
                source.disconnect();
                tracing::info!(source = source.name(), "Telemetry disconnected");
            }
            connection_attempted = false;
            snapshot.connected = false;
            snapshot.frame = None;
            snapshot.error = None;
        } else {
            if !source.is_connected() && !connection_attempted {
                connection_attempted = true;
                match source.connect() {
                    Ok(()) => {
                        tracing::info!(source = source.name(), "Telemetry connected");
                        snapshot.error = None;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Telemetry connection failed");
                        snapshot.error = Some(error.to_string());
                    }
                }
            }
            if source.is_connected() {
                match source.read_frame() {
                    Ok(Some(frame)) => snapshot.frame = Some(frame),
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(%error, "Telemetry read failed");
                        snapshot.error = Some(error.to_string());
                        source.disconnect();
                        snapshot.frame = None;
                    }
                }
            }
            snapshot.connected = source.is_connected();
        }
        let desired_hz = if controls.borrow().source_enabled {
            config.borrow().output.telemetry_hz
        } else {
            1
        };
        if desired_hz != clock_hz {
            clock = ticker(desired_hz);
            clock_hz = desired_hz;
        }
        state.send_replace(snapshot.clone());
    }
    source.disconnect();
    snapshot.connected = false;
    snapshot.frame = None;
    state.send_replace(snapshot);
    tracing::info!(source = source.name(), "Telemetry worker stopped");
}

async fn effects_task(
    mut telemetry: watch::Receiver<TelemetrySnapshot>,
    state: watch::Sender<EffectSnapshot>,
    mut config: watch::Receiver<Arc<Config>>,
    mut controls: watch::Receiver<Controls>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut engine = EffectEngine::default();
    let mut clock = ticker(config.borrow().output.effects_hz);
    let mut clock_hz = config.borrow().output.effects_hz;
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            result = controls.changed() => { if result.is_err() { break; } }
            result = config.changed() => {
                if result.is_err() { break; }
                clock_hz = config.borrow().output.effects_hz;
                clock = ticker(clock_hz);
            }
            result = telemetry.changed() => { if result.is_err() { break; } }
            _ = clock.tick() => {}
        }
        if *shutdown.borrow() {
            break;
        }
        let now = Instant::now();
        let telemetry = telemetry.borrow().clone();
        let controls = *controls.borrow();
        let config = config.borrow().clone();
        let reason = if controls.emergency_stopped {
            StopReason::EmergencyStop
        } else if !controls.source_enabled {
            StopReason::SourceDisabled
        } else if !telemetry.connected || telemetry.frame.is_none() {
            StopReason::NoTelemetry
        } else if !telemetry_is_fresh(
            telemetry.frame.as_ref(),
            now,
            Duration::from_millis(config.output.telemetry_timeout_ms),
        ) {
            StopReason::StaleTelemetry
        } else {
            StopReason::Running
        };
        let mixed = if reason == StopReason::Running {
            telemetry
                .frame
                .as_ref()
                .map(|frame| engine.update(frame, &config.effects, now))
                .unwrap_or(0.0)
        } else {
            engine.reset();
            0.0
        };
        let intensity = scale_output(
            mixed,
            config.output.global_intensity,
            config.output.max_intensity,
        );
        let desired_hz = if reason == StopReason::Running {
            config.output.effects_hz
        } else {
            2
        };
        if desired_hz != clock_hz {
            clock = ticker(desired_hz);
            clock_hz = desired_hz;
        }
        state.send_replace(EffectSnapshot {
            mixed,
            intensity,
            reason,
            heartbeat: now,
        });
    }
    state.send_replace(EffectSnapshot {
        mixed: 0.0,
        intensity: 0.0,
        reason: StopReason::Shutdown,
        heartbeat: Instant::now(),
    });
}

/// Quantized at 1/100 internally until a backend supplies its command resolution.
/// Stops bypass both duplicate suppression and the positive-output rate limit.
#[derive(Default)]
pub struct OutputGate {
    last_value: Option<u32>,
}

impl OutputGate {
    pub fn next(&mut self, intensity: f32) -> Option<f32> {
        // Round down so quantization can never exceed an intensity ceiling.
        self.next_quantized((unit(intensity) * 100.0).floor() / 100.0)
    }
    fn next_quantized(&mut self, intensity: f32) -> Option<f32> {
        let value = unit(intensity);
        if self.last_value == Some(value.to_bits()) {
            return None;
        }
        self.last_value = Some(value.to_bits());
        Some(value)
    }
    pub fn reset(&mut self) {
        self.last_value = None;
    }
}

async fn timed_stop(
    device: &dyn HapticDevice,
    request_timeout: Duration,
) -> Result<(), DeviceError> {
    timeout(request_timeout, device.stop())
        .await
        .map_err(|_| DeviceError::Timeout)?
}

// Channel arguments express task inputs directly, avoiding shared mutable state.
#[allow(clippy::too_many_arguments)]
async fn output_task(
    mut device: Arc<dyn HapticDevice>,
    mut effects: watch::Receiver<EffectSnapshot>,
    telemetry: watch::Receiver<TelemetrySnapshot>,
    state: watch::Sender<DeviceSnapshot>,
    mut config: watch::Receiver<Arc<Config>>,
    mut controls: watch::Receiver<Controls>,
    mut shutdown: watch::Receiver<bool>,
    mut selected_device: watch::Receiver<Arc<dyn HapticDevice>>,
    controls_tx: watch::Sender<Controls>,
) {
    let mut clock = ticker(config.borrow().output.update_hz);
    let mut gate = OutputGate::default();
    let mut snapshot = state.borrow().clone();
    let mut was_connected = false;
    let mut device_epoch = device.connection_epoch();
    let mut resume_epoch = controls.borrow().resume_epoch;
    let mut last_request = None;
    // A communication failure latches output until an explicit Stop + Resume.
    let mut fault_latched = false;
    let initial_timeout = Duration::from_millis(config.borrow().lovense.request_timeout_ms);
    if let Err(error) = timed_stop(device.as_ref(), initial_timeout).await {
        tracing::warn!(%error, "Initial device stop failed");
        snapshot.error = Some(error.to_string());
        fault_latched = true;
    }
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            result = controls.changed() => {
                if result.is_err() { break; }
                let latest = *controls.borrow();
                // A deliberate Stop must retry once even if zero was already
                // consumed after a fault. Timers cannot trigger that retry.
                if latest.emergency_stopped { gate.reset(); }
                if latest.resume_epoch != resume_epoch {
                    resume_epoch = latest.resume_epoch;
                    fault_latched = false;
                    gate.reset();
                }
            }
            result = selected_device.changed() => { if result.is_err() { break; } }
            result = config.changed() => {
                if result.is_err() { break; }
                clock = ticker(config.borrow().output.update_hz);
            }
            result = effects.changed() => { if result.is_err() { break; } }
            _ = clock.tick() => {}
        }
        if *shutdown.borrow() {
            break;
        }
        let now = Instant::now();
        let settings = config.borrow().clone();
        let latest_controls = *controls.borrow();
        let effect = *effects.borrow();
        let sample = telemetry.borrow().clone();
        let deadline = Duration::from_millis(settings.output.telemetry_timeout_ms);
        let request_timeout = Duration::from_millis(settings.lovense.request_timeout_ms);
        let selected = selected_device.borrow().clone();
        if !Arc::ptr_eq(&selected, &device) {
            if let Err(error) = timed_stop(device.as_ref(), request_timeout).await {
                tracing::warn!(%error, "Previous device stop failed during switch");
            }
            device = selected;
            device_epoch = device.connection_epoch();
            was_connected = false;
            gate.reset();
            last_request = None;
            fault_latched = false;
            snapshot.name = device.name().into();
            snapshot.error = None;
            snapshot.intensity = 0.0;
        }
        let connected = device.is_connected();
        snapshot.connected = connected;
        let epoch = device.connection_epoch();
        if connected != was_connected || epoch != device_epoch {
            tracing::info!(
                connected,
                device = device.name(),
                "Device connection changed"
            );
            if let Err(error) = timed_stop(device.as_ref(), request_timeout).await {
                tracing::warn!(%error, "Device stop after connection change failed");
                snapshot.error = Some(error.to_string());
                fault_latched = true;
            }
            snapshot.intensity = 0.0;
            if device.requires_resume_on_connect() {
                controls_tx.send_modify(|control| {
                    control.emergency_stopped = true;
                    control.test_pulse = None;
                });
            }
            gate.reset();
            was_connected = connected;
            device_epoch = epoch;
            // Give reconnect a zero-output cycle before allowing fresh effects.
            state.send_replace(snapshot.clone());
            continue;
        }
        let common_safe = connected && !fault_latched && !latest_controls.emergency_stopped;
        let testing = latest_controls
            .test_pulse
            .is_some_and(|pulse| now < pulse.until);
        let safe = common_safe
            && latest_controls.source_enabled
            && sample.connected
            && telemetry_is_fresh(sample.frame.as_ref(), now, deadline)
            && now
                .checked_duration_since(effect.heartbeat)
                .is_some_and(|age| age < deadline)
            && effect.reason == StopReason::Running;
        let desired = if common_safe && testing {
            scale_output(
                0.4,
                settings.output.global_intensity,
                settings.output.max_intensity,
            )
        } else if safe {
            unit(effect.intensity)
                .min(unit(settings.output.global_intensity))
                .min(unit(settings.output.max_intensity))
        } else {
            0.0
        };
        let period = Duration::from_secs_f64(1.0 / f64::from(settings.output.update_hz));
        if desired > 0.0
            && last_request.is_some_and(|last| now.saturating_duration_since(last) < period)
        {
            continue;
        }
        let quantized = unit(device.quantize(desired)).min(desired);
        let renewal_due = quantized > 0.0
            && device.refresh_interval().is_some_and(|renewal| {
                last_request.is_some_and(|last| now.saturating_duration_since(last) >= renewal)
            });
        if renewal_due {
            gate.reset();
        }
        if let Some(intensity) = gate.next_quantized(quantized) {
            let (applied, result) = if intensity == 0.0 {
                (0.0, timed_stop(device.as_ref(), request_timeout).await)
            } else {
                let safety_deadline = if testing {
                    latest_controls.test_pulse.unwrap().until
                } else {
                    let telemetry_deadline = sample
                        .frame
                        .as_ref()
                        .map_or(now, |frame| frame.timestamp + deadline);
                    telemetry_deadline.min(effect.heartbeat + deadline)
                };
                // Cancel an in-flight command on stop/shutdown, then send stop.
                tokio::select! {
                    biased;
                    _ = shutdown.changed() => break,
                    _ = controls.changed() => {
                        gate.reset();
                        let result = timed_stop(device.as_ref(), request_timeout).await;
                        snapshot.intensity = 0.0;
                        (0.0, result)
                    }
                    _ = selected_device.changed() => {
                        gate.reset();
                        (0.0, timed_stop(device.as_ref(), request_timeout).await)
                    }
                    _ = config.changed() => {
                        gate.reset();
                        clock = ticker(config.borrow().output.update_hz);
                        (0.0, timed_stop(device.as_ref(), request_timeout).await)
                    }
                    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(safety_deadline)) => {
                        gate.reset();
                        (0.0, timed_stop(device.as_ref(), request_timeout).await)
                    }
                    _ = async {
                        let _ = effects.wait_for(|effect| effect.reason != StopReason::Running || effect.intensity == 0.0).await;
                    }, if !testing => {
                        gate.reset();
                        (0.0, timed_stop(device.as_ref(), request_timeout).await)
                    }
                    result = timeout(request_timeout, device.set_vibration(intensity)) => {
                        (intensity, result.map_err(|_| DeviceError::Timeout).and_then(|result| result))
                    }
                }
            };
            last_request = Some(now);
            match result {
                Ok(()) => {
                    // Control changes may have canceled this command in-flight.
                    if controls.borrow().emergency_stopped {
                        snapshot.intensity = 0.0;
                    } else {
                        snapshot.intensity = applied;
                    }
                    snapshot.error = None;
                }
                Err(error) => {
                    tracing::warn!(%error, "Device output failed; output latched off");
                    snapshot.error = Some(error.to_string());
                    snapshot.intensity = 0.0;
                    fault_latched = true;
                    gate.reset();
                    // Mark zero as consumed before the one best-effort stop.
                    // A failed stop must not start an endless retry loop.
                    let _ = gate.next(0.0);
                    if let Err(stop_error) = timed_stop(device.as_ref(), request_timeout).await {
                        tracing::warn!(%stop_error, "Best-effort device stop failed");
                    }
                }
            }
        }
        state.send_replace(snapshot.clone());
    }
    let request_timeout = Duration::from_millis(config.borrow().lovense.request_timeout_ms);
    if let Err(error) = timed_stop(device.as_ref(), request_timeout).await {
        tracing::error!(%error, "Final device stop failed");
    }
    snapshot.connected = false;
    snapshot.intensity = 0.0;
    state.send_replace(snapshot);
    tracing::info!(device = device.name(), "Output worker stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        devices::{DeviceFuture, MockDevice},
        telemetry::{DemoSource, MockTelemetrySource},
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn telemetry_timeout_including_boundary_and_future() {
        let now = Instant::now();
        let mut frame = TelemetryFrame {
            timestamp: now,
            ..TelemetryFrame::default()
        };
        let limit = Duration::from_millis(250);
        assert!(telemetry_is_fresh(
            Some(&frame),
            now + Duration::from_millis(249),
            limit
        ));
        assert!(!telemetry_is_fresh(Some(&frame), now + limit, limit));
        assert!(!telemetry_is_fresh(None, now, limit));
        frame.timestamp = now + Duration::from_secs(1);
        assert!(!telemetry_is_fresh(Some(&frame), now, limit));
    }

    #[test]
    fn duplicate_output_is_suppressed_and_stop_is_immediate() {
        let mut gate = OutputGate::default();
        assert_eq!(gate.next(0.501), Some(0.5));
        assert_eq!(gate.next(0.502), None);
        assert_eq!(gate.next(0.51), Some(0.51));
        assert_eq!(gate.next(0.0), Some(0.0));
        assert_eq!(gate.next(0.0), None);
        assert_eq!(gate.next(f32::NAN), None);
        assert_eq!(gate.next(0.755), Some(0.75));
    }

    async fn wait_for(mut predicate: impl FnMut() -> bool) {
        timeout(Duration::from_secs(2), async {
            while !predicate() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("pipeline state did not arrive");
    }

    #[tokio::test]
    async fn full_demo_pipeline_stop_resume_pause_reconnect_and_shutdown() {
        let device = Arc::new(MockDevice::default());
        let runtime = RaceRuntime::spawn(
            Box::new(DemoSource::default()),
            device.clone(),
            Config::default(),
        )
        .unwrap();
        wait_for(|| device.intensity() > 0.0).await;
        runtime.control.emergency_stop();
        wait_for(|| device.intensity() == 0.0).await;
        let count = device.command_count();
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(device.command_count(), count);
        runtime.control.resume();
        wait_for(|| device.intensity() > 0.0).await;
        runtime.control.set_source_enabled(false);
        wait_for(|| device.intensity() == 0.0).await;
        runtime.control.set_source_enabled(true);
        wait_for(|| device.intensity() > 0.0).await;
        device.disconnect();
        wait_for(|| !runtime.control.snapshot().device.connected).await;
        assert_eq!(device.intensity(), 0.0);
        device.reconnect();
        wait_for(|| device.intensity() > 0.0).await;
        runtime.shutdown().await;
        assert_eq!(device.intensity(), 0.0);
        assert!(device.stop_count() >= 5);
    }

    #[tokio::test]
    async fn mock_telemetry_goes_stale_and_clears_output() {
        let source = MockTelemetrySource::with_frame(TelemetryFrame {
            engine_rpm: 7_000.0,
            engine_max_rpm: 9_000.0,
            gear: 3,
            ..TelemetryFrame::default()
        });
        let device = Arc::new(MockDevice::default());
        let runtime =
            RaceRuntime::spawn(Box::new(source), device.clone(), Config::default()).unwrap();
        wait_for(|| device.intensity() > 0.0).await;
        wait_for(|| {
            runtime.control.snapshot().effects.reason == StopReason::StaleTelemetry
                && device.intensity() == 0.0
        })
        .await;
        runtime.shutdown().await;
        assert_eq!(device.intensity(), 0.0);
    }

    #[tokio::test]
    async fn configuration_changes_apply_without_restarting_workers() {
        let device = Arc::new(MockDevice::default());
        let runtime = RaceRuntime::spawn(
            Box::new(DemoSource::default()),
            device.clone(),
            Config::default(),
        )
        .unwrap();
        wait_for(|| device.intensity() > 0.0).await;
        let mut config = Config::default();
        config.output.global_intensity = 0.0;
        runtime.control.update_config(config).unwrap();
        wait_for(|| device.intensity() == 0.0).await;
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn selecting_a_backend_stops_old_output_and_requires_explicit_resume() {
        let first = Arc::new(MockDevice::default());
        let second = Arc::new(MockDevice::default());
        let runtime = RaceRuntime::spawn(
            Box::new(DemoSource::default()),
            first.clone(),
            Config::default(),
        )
        .unwrap();
        wait_for(|| first.intensity() > 0.0).await;
        runtime.control.select_device(second.clone());
        wait_for(|| first.intensity() == 0.0 && second.stop_count() > 0).await;
        assert!(runtime.control.snapshot().controls.emergency_stopped);
        assert_eq!(second.intensity(), 0.0);
        runtime.control.resume();
        wait_for(|| second.intensity() > 0.0).await;
        assert_eq!(first.intensity(), 0.0);
        runtime.shutdown().await;
        assert_eq!(second.intensity(), 0.0);
    }

    struct FailingDevice {
        commands: AtomicU64,
        stops: AtomicU64,
    }

    impl HapticDevice for FailingDevice {
        fn name(&self) -> &str {
            "Failing test device"
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn set_vibration(&self, _intensity: f32) -> DeviceFuture<'_> {
            self.commands.fetch_add(1, Ordering::Relaxed);
            Box::pin(async { Err(DeviceError::Communication("simulated LAN failure".into())) })
        }
        fn stop(&self) -> DeviceFuture<'_> {
            let previous = self.stops.fetch_add(1, Ordering::Relaxed);
            Box::pin(async move {
                if previous < 2 {
                    Ok(())
                } else {
                    Err(DeviceError::Communication("simulated stop failure".into()))
                }
            })
        }
    }

    #[tokio::test]
    async fn network_failures_latch_and_do_not_retry_forever() {
        let device = Arc::new(FailingDevice {
            commands: AtomicU64::new(0),
            stops: AtomicU64::new(0),
        });
        let runtime = RaceRuntime::spawn(
            Box::new(DemoSource::default()),
            device.clone(),
            Config::default(),
        )
        .unwrap();
        wait_for(|| runtime.control.snapshot().device.error.is_some()).await;
        let commands = device.commands.load(Ordering::Relaxed);
        let stops = device.stops.load(Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(device.commands.load(Ordering::Relaxed), commands);
        assert_eq!(device.stops.load(Ordering::Relaxed), stops);
        assert_eq!(runtime.control.snapshot().device.intensity, 0.0);
        runtime.shutdown().await;
    }

    struct SlowDevice {
        mock: MockDevice,
        started: AtomicU64,
    }

    impl HapticDevice for SlowDevice {
        fn name(&self) -> &str {
            "Slow test device"
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn set_vibration(&self, intensity: f32) -> DeviceFuture<'_> {
            Box::pin(async move {
                self.started.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_secs(5)).await;
                self.mock.set_vibration(intensity).await
            })
        }
        fn stop(&self) -> DeviceFuture<'_> {
            self.mock.stop()
        }
    }

    #[tokio::test]
    async fn emergency_stop_cancels_in_flight_output_without_waiting_for_timeout() {
        let device = Arc::new(SlowDevice {
            mock: MockDevice::default(),
            started: AtomicU64::new(0),
        });
        let runtime = RaceRuntime::spawn(
            Box::new(DemoSource::default()),
            device.clone(),
            Config::default(),
        )
        .unwrap();
        wait_for(|| device.started.load(Ordering::Relaxed) > 0).await;
        let stops = device.mock.stop_count();
        runtime.control.emergency_stop();
        timeout(Duration::from_millis(200), async {
            while device.mock.stop_count() == stops {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("emergency stop waited for a network command");
        assert_eq!(device.mock.intensity(), 0.0);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn effect_worker_failure_stops_output_on_channel_closure() {
        let device = Arc::new(MockDevice::default());
        let mut runtime = RaceRuntime::spawn(
            Box::new(DemoSource::default()),
            device.clone(),
            Config::default(),
        )
        .unwrap();
        wait_for(|| device.intensity() > 0.0).await;
        let aborted = runtime.tasks.remove(1);
        aborted.abort();
        let _ = aborted.await;
        wait_for(|| device.intensity() == 0.0).await;
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn output_watchdog_stops_when_effect_heartbeat_stalls_with_live_telemetry() {
        let device = Arc::new(MockDevice::default());
        let (config_tx, config_rx) = watch::channel(Arc::new(Config::default()));
        let (controls_tx, controls_rx) = watch::channel(Controls::default());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (telemetry_tx, telemetry_rx) = watch::channel(TelemetrySnapshot {
            source_name: "Demo",
            connected: true,
            frame: Some(DemoSource::frame_at(1.0, Instant::now())),
            error: None,
        });
        // Keep this sender alive but deliberately do not refresh its heartbeat.
        let (effects_tx, effects_rx) = watch::channel(EffectSnapshot {
            mixed: 0.5,
            intensity: 0.25,
            reason: StopReason::Running,
            heartbeat: Instant::now(),
        });
        let (state_tx, state_rx) = watch::channel(DeviceSnapshot {
            name: device.name().into(),
            connected: false,
            intensity: 0.0,
            error: None,
        });
        let (_selection_tx, selection_rx) = watch::channel(device.clone() as Arc<dyn HapticDevice>);
        let task = tokio::spawn(output_task(
            device.clone(),
            effects_rx,
            telemetry_rx,
            state_tx,
            config_rx,
            controls_rx,
            shutdown_rx,
            selection_rx,
            controls_tx.clone(),
        ));
        wait_for(|| device.intensity() > 0.0).await;
        let telemetry_worker = tokio::spawn(async move {
            let mut clock = ticker(60);
            loop {
                clock.tick().await;
                telemetry_tx.send_modify(|snapshot| {
                    snapshot.frame = Some(DemoSource::frame_at(1.0, Instant::now()))
                });
            }
        });
        wait_for(|| device.intensity() == 0.0).await;
        assert!(state_rx.borrow().connected);
        shutdown_tx.send_replace(true);
        task.await.unwrap();
        telemetry_worker.abort();
        let _ = telemetry_worker.await;
        drop((config_tx, controls_tx, effects_tx));
    }
}
