use std::{sync::Arc, time::Duration};

use race2love_core::{
    config::{LovenseConfig, LovenseOutputMode},
    devices::{DeviceError, DeviceFuture, HapticDevice},
    unit,
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, sleep_until},
};

use crate::smoothing::{Cadence, Output, Smoother};
use crate::{RENEWAL, RemoteClient, Toy, vibration_step};

const POLL: Duration = Duration::from_secs(2);
const RETRIES: [u64; 5] = [1, 2, 4, 8, 16];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Exhausted,
}

impl ConnectionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting",
            Self::Connected => "Remote connected",
            Self::Reconnecting => "Reconnecting",
            Self::Exhausted => "Reconnect attempts exhausted; click Connect",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LovenseSnapshot {
    pub state: ConnectionState,
    pub endpoint: Option<String>,
    pub toys: Vec<Toy>,
    pub selected: Option<String>,
    pub selected_ready: bool,
    pub epoch: u64,
    pub retries: usize,
    pub error: Option<String>,
    pub output_mode: LovenseOutputMode,
    pub using_vibrate_fallback: bool,
    cadence: Option<Cadence>,
}

#[derive(Clone, Default)]
struct Desired {
    config: LovenseConfig,
    enabled: bool,
    selected: Option<String>,
    revision: u64,
}

#[derive(Clone)]
pub struct LovenseControl {
    desired: watch::Sender<Desired>,
    state: watch::Receiver<LovenseSnapshot>,
}

impl LovenseControl {
    pub fn snapshot(&self) -> LovenseSnapshot {
        self.state.borrow().clone()
    }
    /// Connect/refresh is a deliberate request. Nothing connects on app startup.
    pub fn connect(&self, config: LovenseConfig) {
        self.desired.send_modify(|desired| {
            if desired.config != config {
                desired.selected = None;
            }
            desired.config = config;
            desired.enabled = true;
            desired.revision = desired.revision.wrapping_add(1);
        });
    }
    pub fn disconnect(&self) {
        self.desired.send_modify(|desired| {
            desired.enabled = false;
            desired.selected = None;
            desired.revision = desired.revision.wrapping_add(1);
        });
    }
    pub fn select_toy(&self, id: String) {
        self.desired.send_modify(|desired| {
            desired.selected = Some(id);
            desired.revision = desired.revision.wrapping_add(1);
        });
    }
}

struct Command {
    intensity: Option<f32>,
    ceiling: f32,
    epoch: u64,
    reply: oneshot::Sender<Result<(), DeviceError>>,
}

pub struct LovenseDevice {
    commands: mpsc::Sender<Command>,
    state: watch::Receiver<LovenseSnapshot>,
}

impl LovenseDevice {
    async fn command(&self, intensity: Option<f32>, ceiling: f32) -> Result<(), DeviceError> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Command {
                intensity,
                ceiling,
                epoch: self.connection_epoch(),
                reply,
            })
            .await
            .map_err(|_| DeviceError::Disconnected)?;
        result.await.map_err(|_| DeviceError::Disconnected)?
    }
}

impl HapticDevice for LovenseDevice {
    fn name(&self) -> &str {
        "Lovense Remote"
    }
    fn is_connected(&self) -> bool {
        self.state.borrow().selected_ready
    }
    fn set_vibration(&self, intensity: f32) -> DeviceFuture<'_> {
        Box::pin(self.command(Some(intensity), 1.0))
    }
    fn set_vibration_with_limit(&self, intensity: f32, ceiling: f32) -> DeviceFuture<'_> {
        Box::pin(self.command(Some(intensity), ceiling))
    }
    fn stop(&self) -> DeviceFuture<'_> {
        Box::pin(self.command(None, 0.0))
    }
    fn refresh_interval(&self) -> Option<Duration> {
        Some(RENEWAL)
    }
    fn next_update_at(&self, intensity: f32, ceiling: f32) -> Option<std::time::Instant> {
        let state = self.state.borrow();
        if state.output_mode == LovenseOutputMode::Vibrate || state.using_vibrate_fallback {
            None
        } else {
            state
                .cadence
                .and_then(|cadence| cadence.next_update_at(intensity, ceiling))
        }
    }
    fn quantize(&self, intensity: f32) -> f32 {
        let state = self.state.borrow();
        if state.output_mode == LovenseOutputMode::Vibrate || state.using_vibrate_fallback {
            f32::from(vibration_step(intensity)) / 20.0
        } else {
            // Preserve fractions to 0.01 native level, suppressing insignificant jitter.
            (unit(intensity) * 2000.0).floor() / 2000.0
        }
    }
    fn connection_epoch(&self) -> u64 {
        self.state.borrow().epoch
    }
    fn requires_resume_on_connect(&self) -> bool {
        false
    }
}

/// Owns the HTTP worker. Await shutdown after the core output task has stopped.
pub struct LovenseService {
    pub control: LovenseControl,
    pub device: Arc<LovenseDevice>,
    shutdown: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl LovenseService {
    pub fn spawn() -> Self {
        let (desired_tx, desired_rx) = watch::channel(Desired::default());
        let (state_tx, state_rx) = watch::channel(LovenseSnapshot::default());
        let (commands_tx, commands_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(
            Worker {
                desired: desired_rx,
                state: state_tx,
                commands: commands_rx,
                shutdown: shutdown_rx,
                client: None,
                selected: None,
                applied: Desired::default(),
                snapshot: LovenseSnapshot::default(),
                due: None,
                smoother: Smoother::default(),
            }
            .run(),
        );
        Self {
            control: LovenseControl {
                desired: desired_tx,
                state: state_rx.clone(),
            },
            device: Arc::new(LovenseDevice {
                commands: commands_tx,
                state: state_rx,
            }),
            shutdown: shutdown_tx,
            task: Some(task),
        }
    }
    pub async fn shutdown(mut self) {
        self.shutdown.send_replace(true);
        if let Some(task) = self.task.take()
            && let Err(error) = task.await
        {
            tracing::error!(%error, "Lovense worker failed during shutdown");
        }
    }
}

impl Drop for LovenseService {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
    }
}

struct Worker {
    desired: watch::Receiver<Desired>,
    state: watch::Sender<LovenseSnapshot>,
    commands: mpsc::Receiver<Command>,
    shutdown: watch::Receiver<bool>,
    client: Option<RemoteClient>,
    selected: Option<String>,
    applied: Desired,
    snapshot: LovenseSnapshot,
    due: Option<Instant>,
    smoother: Smoother,
}

impl Worker {
    fn publish(&self) {
        self.state.send_replace(self.snapshot.clone());
    }
    fn invalidate(&mut self) {
        self.smoother.reset();
        self.snapshot.cadence = None;
        self.snapshot.using_vibrate_fallback = false;
        self.snapshot.selected_ready = false;
        self.snapshot.epoch = self.snapshot.epoch.wrapping_add(1);
        self.publish();
    }
    async fn stop_selected(&self) -> Result<(), DeviceError> {
        if let (Some(client), Some(toy)) = (&self.client, &self.selected) {
            client.stop(toy).await
        } else {
            Ok(())
        }
    }
    fn failure(&mut self, error: &DeviceError) {
        tracing::warn!(%error, "Lovense connection failed; output stopped");
        if self.snapshot.selected_ready {
            self.invalidate();
        }
        self.snapshot.error = Some(error.to_string());
        if self.applied.enabled
            && self.applied.config.automatic_reconnect
            && self.snapshot.retries < RETRIES.len()
        {
            self.due = Some(Instant::now() + Duration::from_secs(RETRIES[self.snapshot.retries]));
            self.snapshot.retries += 1;
            self.snapshot.state = ConnectionState::Reconnecting;
        } else {
            self.due = None;
            self.snapshot.state = ConnectionState::Exhausted;
        }
        self.publish();
    }

    async fn apply_desired(&mut self, desired: Desired) {
        self.invalidate();
        if let Err(error) = self.stop_selected().await {
            tracing::warn!(%error, "Stop before Lovense selection change failed; lease bounds old output");
        }
        let endpoint_changed = self.applied.config != desired.config || !self.applied.enabled;
        self.applied = desired;
        self.snapshot.output_mode = self.applied.config.output_mode;
        self.snapshot.error = None;
        self.snapshot.retries = 0;
        if !self.applied.enabled {
            self.client = None;
            self.selected = None;
            self.snapshot = LovenseSnapshot {
                epoch: self.snapshot.epoch,
                ..Default::default()
            };
            self.due = None;
            tracing::info!("Lovense Remote disconnected");
        } else {
            if endpoint_changed {
                self.snapshot.toys.clear();
            }
            self.selected = None;
            self.snapshot.selected = None;
            match RemoteClient::new(&self.applied.config) {
                Ok(client) => {
                    self.snapshot.endpoint = Some(client.endpoint().into());
                    self.client = Some(client);
                    self.snapshot.state = ConnectionState::Connecting;
                    self.due = Some(Instant::now());
                }
                Err(error) => {
                    self.snapshot.endpoint = None;
                    self.client = None;
                    self.snapshot.state = ConnectionState::Exhausted;
                    self.snapshot.error = Some(error.to_string());
                    self.due = None;
                }
            }
        }
        self.publish();
    }

    async fn discovery(&mut self, toys: Vec<Toy>) {
        let candidate = self
            .applied
            .selected
            .as_ref()
            .and_then(|id| toys.iter().find(|toy| &toy.id == id));
        let ready = candidate.is_some_and(|toy| toy.connected && toy.vibration != Some(false));
        let new_selected = candidate.map(|toy| toy.id.clone());
        if new_selected != self.selected || ready != self.snapshot.selected_ready {
            self.invalidate();
            if let Err(error) = self.stop_selected().await {
                self.failure(&error);
                return;
            }
            self.selected = new_selected;
            // Stop the newly selected/reconnected toy before publishing readiness.
            if ready && let Err(error) = self.stop_selected().await {
                self.failure(&error);
                return;
            }
            self.snapshot.epoch = self.snapshot.epoch.wrapping_add(1);
        }
        if self.snapshot.toys != toys {
            tracing::info!(toys = toys.len(), "Lovense toy discovery updated");
        }
        self.snapshot.toys = toys;
        self.snapshot.selected = self.selected.clone();
        self.snapshot.selected_ready = ready;
        if self.snapshot.state != ConnectionState::Connected {
            tracing::info!("Lovense Remote connected");
        }
        self.snapshot.state = ConnectionState::Connected;
        self.snapshot.retries = 0;
        self.snapshot.error = if self.applied.selected.is_some() && !ready {
            Some("Selected toy is unavailable or does not support vibration.".into())
        } else {
            None
        };
        self.due = Some(Instant::now() + POLL);
        self.publish();
    }

    async fn command(&mut self, mut command: Command) {
        if command.reply.is_closed() {
            return;
        }
        let Some(client) = self.client.clone() else {
            let _ = command.reply.send(if command.intensity.is_none() {
                Ok(())
            } else {
                Err(DeviceError::Disconnected)
            });
            return;
        };
        let Some(toy) = self.selected.clone() else {
            let _ = command.reply.send(if command.intensity.is_none() {
                Ok(())
            } else {
                Err(DeviceError::Disconnected)
            });
            return;
        };
        let result = if let Some(intensity) = command.intensity {
            if !self.snapshot.selected_ready || command.epoch != self.snapshot.epoch {
                let _ = command.reply.send(Err(DeviceError::Disconnected));
                return;
            }
            let mode = if self.snapshot.using_vibrate_fallback {
                LovenseOutputMode::Vibrate
            } else {
                self.applied.config.output_mode
            };
            let Some(output) =
                self.smoother
                    .prepare(intensity, command.ceiling, mode, Instant::now().into_std())
            else {
                let _ = command.reply.send(Ok(()));
                return;
            };
            let was_pattern = matches!(output, Output::Pattern(_));
            let request = async {
                match output {
                    Output::Vibrate(level) => {
                        client.vibrate(&toy, f32::from(level) / 20.0).await?;
                        Ok(true)
                    }
                    Output::Pattern(levels) => {
                        // Pattern does not document stopPrevious. Use the existing
                        // Function contract to cancel older schedules without a
                        // zero-strength gap, before installing the new pattern.
                        client.vibrate(&toy, f32::from(levels[0]) / 20.0).await?;
                        if client.pattern(&toy, &levels).await? {
                            Ok(true)
                        } else {
                            // One bounded compatibility fallback, never after an
                            // ambiguous timeout. Clear any previous pattern first.
                            client.stop(&toy).await?;
                            client
                                .vibrate(&toy, unit(intensity).min(unit(command.ceiling)))
                                .await?;
                            Ok(false)
                        }
                    }
                }
            };
            let result = tokio::select! {
                biased;
                _ = self.shutdown.changed() => return,
                _ = self.desired.changed() => return,
                _ = command.reply.closed() => {
                    // The caller canceled an in-flight command (Stop/timeout).
                    // The request may already have arrived: follow it with Stop.
                    self.smoother.reset();
                    client.stop(&toy).await.map(|()| true)
                }
                result = request => result,
            };
            if let Ok(false) = result {
                tracing::warn!(
                    "Remote does not support Pattern; using direct Vibrate until reconnect"
                );
                self.snapshot.using_vibrate_fallback = true;
                self.smoother.reset();
                let _ = self.smoother.prepare(
                    intensity,
                    command.ceiling,
                    LovenseOutputMode::Vibrate,
                    Instant::now().into_std(),
                );
                self.publish();
            }
            if was_pattern
                && result.is_err()
                && let Err(error) = client.stop(&toy).await
            {
                tracing::warn!(%error, "Stop after failed Pattern replacement failed; finite lease bounds output");
            }
            result.map(|_| ())
        } else {
            self.smoother.reset();
            client.stop(&toy).await
        };
        if result.is_err() {
            self.smoother.reset();
        }
        self.snapshot.cadence = self.smoother.acknowledge(Instant::now().into_std());
        self.publish();
        if let Err(error) = &result
            && self.snapshot.state == ConnectionState::Connected
        {
            self.failure(error);
        }
        let _ = command.reply.send(result);
    }

    async fn run(mut self) {
        loop {
            if *self.shutdown.borrow() {
                break;
            }
            let desired = self.desired.borrow().clone();
            if desired.revision != self.applied.revision {
                self.apply_desired(desired).await;
                continue;
            }
            let due = self
                .due
                .unwrap_or_else(|| Instant::now() + Duration::from_secs(86_400));
            tokio::select! {
                biased;
                _ = self.shutdown.changed() => break,
                result = self.desired.changed() => { if result.is_err() { break; } }
                command = self.commands.recv() => {
                    match command { Some(command) => self.command(command).await, None => break }
                }
                _ = sleep_until(due), if self.due.is_some() => {
                    let Some(client) = self.client.clone() else { self.due = None; continue; };
                    // Keep discovery alive across output commands so a slow Remote
                    // cannot starve toy-status checks. Stop preempts the status wait.
                    let discovery = client.get_toys();
                    tokio::pin!(discovery);
                    loop { tokio::select! {
                        biased;
                        _ = self.shutdown.changed() => break,
                        _ = self.desired.changed() => break,
                        command = self.commands.recv() => {
                            match command { Some(command) => self.command(command).await, None => break }
                            if self.snapshot.state != ConnectionState::Connected { break; }
                        }
                        result = &mut discovery => {
                            match result {
                                Ok(toys) => self.discovery(toys).await,
                                Err(error) => {
                                    self.failure(&error);
                                    if let Err(error) = self.stop_selected().await {
                                        tracing::warn!(%error, "Stop after discovery failure failed");
                                    }
                                }
                            }
                            break;
                        }
                    } }
                }
            }
        }
        self.invalidate();
        if let Err(error) = self.stop_selected().await {
            tracing::warn!(%error, "Final Lovense stop failed");
        }
        self.snapshot.state = ConnectionState::Disconnected;
        self.publish();
        tracing::info!("Lovense worker stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn reconnect_backoff_has_a_fixed_attempt_limit() {
        let (_desired_tx, desired) = watch::channel(Desired::default());
        let (state, _) = watch::channel(LovenseSnapshot::default());
        let (_commands_tx, commands) = mpsc::channel(8);
        let (_shutdown_tx, shutdown) = watch::channel(false);
        let mut worker = Worker {
            desired,
            state,
            commands,
            shutdown,
            client: None,
            selected: None,
            applied: Desired {
                enabled: true,
                ..Default::default()
            },
            snapshot: LovenseSnapshot::default(),
            due: None,
            smoother: Smoother::default(),
        };
        for delay in RETRIES {
            worker.failure(&DeviceError::Timeout);
            assert_eq!(worker.snapshot.state, ConnectionState::Reconnecting);
            assert_eq!(
                worker.due.unwrap() - Instant::now(),
                Duration::from_secs(delay)
            );
        }
        worker.failure(&DeviceError::Timeout);
        assert_eq!(worker.snapshot.retries, 5);
        assert_eq!(worker.snapshot.state, ConnectionState::Exhausted);
        assert!(worker.due.is_none());
    }
}
