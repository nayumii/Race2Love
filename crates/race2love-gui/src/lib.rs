//! Native desktop views. Render code only reads channel snapshots and sends
//! controls; telemetry reads and device communication stay in core workers.

use std::{path::PathBuf, sync::Arc, time::Duration};

use eframe::egui;
use race2love_core::{
    config::{Config, LocalProtocol},
    devices::HapticDevice,
    effects::ResponseCurve,
    runtime::{RuntimeControl, RuntimeSnapshot, StopReason, telemetry_is_fresh},
    telemetry::DemoSource,
    unit,
};
use race2love_lovense::{ConnectionState, LovenseControl};
mod graphs;
mod theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Dashboard,
    Effects,
    Settings,
}

pub struct Race2LoveApp {
    control: RuntimeControl,
    config: Config,
    config_path: Option<PathBuf>,
    page: Page,
    dirty: bool,
    message: Option<String>,
    config_error: Option<String>,
    lovense: Option<LovenseControl>,
    devices: Option<(Arc<dyn HapticDevice>, Arc<dyn HapticDevice>)>,
    history: graphs::History,
    profile_name: String,
    styled: bool,
}

impl Race2LoveApp {
    pub fn new(
        control: RuntimeControl,
        config: Config,
        config_path: Option<PathBuf>,
        startup_message: Option<String>,
    ) -> Self {
        Self {
            control,
            config,
            config_path,
            page: Page::Dashboard,
            dirty: false,
            message: startup_message,
            config_error: None,
            lovense: None,
            devices: None,
            history: graphs::History::default(),
            profile_name: "My effects".into(),
            styled: false,
        }
    }

    pub fn with_lovense(
        mut self,
        control: LovenseControl,
        device: Arc<dyn HapticDevice>,
        demo: Arc<dyn HapticDevice>,
    ) -> Self {
        self.lovense = Some(control);
        self.devices = Some((device, demo));
        self
    }

    fn save_settings(&mut self) {
        let Some(path) = &self.config_path else {
            self.message = Some("Set RACE2LOVE_CONFIG to choose a settings file.".into());
            return;
        };
        match self.config.save(path) {
            Ok(()) => {
                self.dirty = false;
                self.message = Some("Settings saved.".into());
            }
            Err(error) => {
                tracing::error!(%error, ?path, "Could not save settings");
                self.message = Some("Settings could not be saved. Details are in the logs.".into());
            }
        }
    }

    fn navigation(&mut self, ui: &mut egui::Ui, vertical: bool) {
        for (page, title, subtitle) in [
            (Page::Dashboard, "Dashboard", "Feedback & output"),
            (Page::Effects, "Effects", "Shape your feedback"),
            (Page::Settings, "Devices / Settings", "Connect & configure"),
        ] {
            let selected = self.page == page;
            let button = egui::Button::selectable(selected, title)
                .min_size(egui::vec2(if vertical { 174.0 } else { 0.0 }, 38.0));
            if ui.add(button).clicked() {
                self.page = page;
            }
            if vertical {
                ui.label(egui::RichText::new(subtitle).size(11.0).color(theme::MUTED));
                ui.add_space(16.0);
            }
        }
    }

    fn header(&mut self, ui: &mut egui::Ui, snapshot: &RuntimeSnapshot) {
        ui.horizontal_wrapped(|ui| {
            theme::brand(ui);
            ui.add_space(12.0);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("EMERGENCY STOP · Esc")
                            .color(theme::RED)
                            .strong(),
                    )
                    .fill(egui::Color32::from_rgb(64, 29, 42))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(112, 49, 64)))
                    .min_size(egui::vec2(0.0, 38.0)),
                )
                .clicked()
            {
                self.control.emergency_stop();
            }
            if snapshot.controls.emergency_stopped
                && ui
                    .add(
                        egui::Button::new("Resume output")
                            .fill(egui::Color32::from_rgb(35, 78, 74)),
                    )
                    .clicked()
            {
                self.control.resume();
            }
            let testing = snapshot
                .controls
                .test_pulse
                .is_some_and(|pulse| std::time::Instant::now() < pulse.until);
            indicator(
                ui,
                snapshot.effects.reason == StopReason::Running || testing,
                if testing {
                    "Testing vibration"
                } else {
                    match snapshot.effects.reason {
                        StopReason::Running => "Running",
                        StopReason::EmergencyStop => "Output stopped",
                        StopReason::SourceDisabled => "Game input paused",
                        StopReason::NoTelemetry => "Waiting for game",
                        StopReason::StaleTelemetry => "Game data lost · output stopped",
                        StopReason::Shutdown => "Shutting down",
                    }
                },
            );
        });
        if snapshot.controls.emergency_stopped {
            ui.label(
                egui::RichText::new("Output paused. Resume when ready to feel the drive.")
                    .size(12.0)
                    .color(theme::MUTED),
            );
        }
        ui.add_space(4.0);
    }

    fn dashboard(&mut self, ui: &mut egui::Ui, snapshot: &RuntimeSnapshot) {
        theme::page_title(
            ui,
            "Your feedback",
            "Control the strength of your feedback and see what reaches your device.",
        );
        let fresh = snapshot.telemetry.connected
            && telemetry_is_fresh(
                snapshot.telemetry.frame.as_ref(),
                std::time::Instant::now(),
                Duration::from_millis(self.config.output.telemetry_timeout_ms),
            );
        theme::card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label("Game:");
                if ui
                    .selectable_label(snapshot.telemetry.source_name == "Demo", "Demo")
                    .clicked()
                {
                    self.control
                        .select_source(|| Box::new(DemoSource::default()));
                }
                if ui
                    .add_enabled(
                        cfg!(any(windows, target_os = "linux")),
                        egui::Button::selectable(
                            snapshot.telemetry.source_name == "Le Mans Ultimate",
                            "Le Mans Ultimate",
                        ),
                    )
                    .clicked()
                {
                    self.control.select_source(race2love_lmu::native_source);
                }
                let mut enabled = snapshot.controls.source_enabled;
                if ui.checkbox(&mut enabled, "Enable game input").changed() {
                    self.control.set_source_enabled(enabled);
                }
            });
            if !cfg!(any(windows, target_os = "linux")) {
                ui.small(
                "Native LMU telemetry supports Windows and Linux/Proton. Demo remains available.",
            );
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                indicator(
                    ui,
                    fresh,
                    if fresh {
                        "Game ready"
                    } else {
                        "Waiting for game"
                    },
                );
                indicator(
                    ui,
                    snapshot.device.connected,
                    snapshot
                        .device
                        .name
                        .strip_suffix(" (mock device)")
                        .unwrap_or(&snapshot.device.name),
                );
            });
            if let Some(error) = &snapshot.telemetry.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            if let Some(error) = &snapshot.device.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        });
        ui.add_space(6.0);
        Self::output_card(ui, snapshot, self.config.ui.show_debug);
        let levels = snapshot.effects.levels;
        theme::card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            theme::section(
                ui,
                "Effect activity",
                "Which effects are contributing to your feedback",
            );
            let signals = [
                ("Engine", levels.engine),
                ("Gear pulse", levels.gear_shift),
                ("Wheel slip", levels.wheel_slip),
                ("Kerbs / road", levels.road),
                ("Impact", levels.impact),
            ];
            if ui.available_width() >= 720.0 {
                ui.columns(5, |cols| {
                    for (column, (name, value)) in cols.iter_mut().zip(signals) {
                        meter(column, name, value);
                    }
                });
            } else {
                for (name, value) in signals {
                    meter(ui, name, value);
                }
            }
        });
        if self
            .devices
            .as_ref()
            .is_some_and(|(physical, _)| physical.name() == snapshot.device.name)
        {
            ui.small("Feedback is routed to your selected device.");
        } else {
            ui.small(
                "Demo output is active. Connect a device in Devices / Settings to feel feedback.",
            );
        }
        theme::card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            theme::section(
                ui,
                "Output limits",
                "Tune the overall strength of your feedback.",
            );
            percentage_slider(
                ui,
                &mut self.config.output.global_intensity,
                "Global intensity",
                0.0..=1.0,
            );
            percentage_slider(
                ui,
                &mut self.config.output.max_intensity,
                "Maximum output",
                0.0..=1.0,
            );
        });
        if self.config.ui.show_debug {
            ui.separator();
            theme::section(
                ui,
                "Debug diagnostics",
                "Live game data and detection details",
            );
            let frame = snapshot.telemetry.frame.as_ref().filter(|_| fresh);
            ui.columns(3, |cols| {
                theme::metric(
                    &mut cols[0],
                    "SPEED",
                    &frame.map_or("—".into(), |f| format!("{:.0}", f.speed_mps * 3.6)),
                    "km/h",
                    theme::TEXT,
                );
                theme::metric(
                    &mut cols[1],
                    "ENGINE",
                    &frame.map_or("—".into(), |f| format!("{:.0}", f.engine_rpm)),
                    "RPM",
                    theme::ACCENT,
                );
                let gear = frame.map_or("—".into(), |f| match f.gear {
                    -1 => "R".into(),
                    0 => "N".into(),
                    n => n.to_string(),
                });
                theme::metric(&mut cols[2], "GEAR", &gear, "Current gear", theme::VIOLET);
            });
            ui.add_space(6.0);
            Self::driving_card(ui, frame);
            let levels = snapshot.effects.levels;
            ui.horizontal(|ui| {
                indicator(
                    ui,
                    levels
                        .last_shift
                        .is_some_and(|at| at.elapsed() < Duration::from_millis(500)),
                    "Shift detected",
                );
                ui.label(format!("{} shifts since effects reset", levels.shift_count));
            });
            ui.separator();
            ui.label("Normalized signals · optional values remain unavailable unless verified");
            if let Some(frame) = &snapshot.telemetry.frame {
                ui.monospace(format!(
                    "Sample age: {} ms",
                    frame.timestamp.elapsed().as_millis()
                ));
                ui.monospace(format!("Wheel slip: {:?}", frame.wheel_slip));
                ui.monospace(format!(
                    "Suspension velocity (m/s): {:?}",
                    frame.suspension_velocity
                ));
                ui.monospace(format!(
                    "Vertical acceleration (m/s²): {:?}",
                    frame.vertical_acceleration
                ));
                ui.monospace(format!(
                    "Impact severity / event: {:?} / {:?}",
                    frame.impact, frame.impact_id
                ));
                ui.monospace(format!("Rumble-strip contact: {:?}", frame.kerb_contact));
                ui.monospace(format!("Tyre terrain: {:?}", frame.wheel_terrain));
                ui.monospace(format!(
                    "Road vibration (m/s²): {:?}",
                    snapshot.effects.levels.road_vibration
                ));
            }
        }
        ui.checkbox(&mut self.config.ui.show_graphs, "Show output history");
        if self.config.ui.show_graphs {
            self.history
                .show(ui, std::time::Instant::now(), self.config.ui.show_debug);
        }
    }

    fn driving_card(ui: &mut egui::Ui, frame: Option<&race2love_core::telemetry::TelemetryFrame>) {
        theme::card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            theme::section(ui, "On track", "");
            if let Some(frame) = frame {
                ui.label(
                    egui::RichText::new(frame.car.as_deref().unwrap_or("Unknown car")).strong(),
                );
                ui.label(
                    egui::RichText::new(frame.session.as_deref().unwrap_or("Unknown session"))
                        .color(theme::MUTED),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "{:.0} / {:.0} RPM",
                        frame.engine_rpm, frame.engine_max_rpm
                    ))
                    .size(12.0)
                    .color(theme::MUTED),
                );
                let ratio = if frame.engine_max_rpm > 0.0 {
                    frame.engine_rpm / frame.engine_max_rpm
                } else {
                    0.0
                };
                meter(ui, "RPM", ratio);
                theme::bar(ui, "Throttle", frame.throttle, theme::ACCENT);
                theme::bar(ui, "Brake", frame.brake, theme::RED);
            } else {
                ui.label("Waiting for a driving session");
                ui.label(
                    egui::RichText::new(
                        "Start Demo, or enter the car in LMU. Live values will appear here.",
                    )
                    .color(theme::MUTED),
                );
                meter(ui, "RPM", 0.0);
                theme::bar(ui, "Throttle", 0.0, theme::ACCENT);
                theme::bar(ui, "Brake", 0.0, theme::RED);
            }
        });
    }

    fn output_card(ui: &mut egui::Ui, snapshot: &RuntimeSnapshot, debug: bool) {
        theme::card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            theme::section(
                ui,
                "Mixer output",
                "Combined strength of your active effects",
            );
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{:.0}%", unit(snapshot.effects.mixed) * 100.0))
                        .size(42.0)
                        .color(theme::VIOLET)
                        .strong(),
                );
                ui.label(egui::RichText::new("Before global intensity").color(theme::MUTED));
            });
            if debug {
                meter(ui, "Scaled target", snapshot.effects.intensity);
            }
            theme::bar(
                ui,
                "Device output",
                snapshot.device.intensity,
                theme::VIOLET,
            );
        });
    }

    fn effects(&mut self, ui: &mut egui::Ui) {
        theme::page_title(
            ui,
            "Make it feel like you.",
            "Tune each effect independently. Changes apply as you drive.",
        );
        theme::card().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            theme::section(ui, "Engine RPM", "A continuous connection to the engine.");
            let engine = &mut self.config.effects.engine;
            ui.checkbox(&mut engine.enabled, "Enable engine vibration");
            ui.add_enabled_ui(engine.enabled, |ui| {
                percentage_slider(
                    ui,
                    &mut engine.start_ratio,
                    "Start at % of max RPM",
                    0.0..=(engine.end_ratio - 0.01).max(0.0),
                );
                percentage_slider(
                    ui,
                    &mut engine.end_ratio,
                    "Full effect at % of max RPM",
                    (engine.start_ratio + 0.01).min(1.0)..=1.0,
                );
                percentage_slider(
                    ui,
                    &mut engine.min_intensity,
                    "Minimum vibration",
                    0.0..=engine.max_intensity,
                );
                percentage_slider(
                    ui,
                    &mut engine.max_intensity,
                    "Maximum vibration",
                    engine.min_intensity..=1.0,
                );
                egui::ComboBox::from_id_salt("rpm_curve")
                    .selected_text(engine.curve.name())
                    .show_ui(ui, |ui| {
                        for curve in ResponseCurve::ALL {
                            ui.selectable_value(&mut engine.curve, curve, curve.name());
                        }
                    });
            });
            theme::engine_preview(ui, engine);
        });
        theme::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        theme::section(ui, "Gear shift", "A precise pulse with every forward shift.");
        let shift = &mut self.config.effects.gear_shift;
        ui.checkbox(&mut shift.enabled, "Enable shift pulses");
        ui.add_enabled_ui(shift.enabled, |ui| {
            percentage_slider(ui, &mut shift.intensity, "Pulse intensity", 0.0..=1.0);
            ui.add(egui::Slider::new(&mut shift.hold_ms, 0..=500).text("Hold (ms)"));
            ui.add(egui::Slider::new(&mut shift.attack_ms, 0..=200).text("Attack (ms)"));
            ui.add(egui::Slider::new(&mut shift.release_ms, 0..=500).text("Release (ms)"));
            ui.label(format!(
                "Total pulse duration: {} ms",
                shift.attack_ms + shift.hold_ms + shift.release_ms
            ));
        });
        if self.config.ui.show_debug { ui.small("Adjacent forward shifts count, including a brief neutral transition (up to 250 ms). Long neutral, reverse and reconnects reset detection."); }
        if self.config.ui.show_debug { ui.small(format!(
            "Output updates at {} Hz. Pulses shorter than {:.0} ms may be missed.",
            self.config.output.update_hz,
            1000.0 / f64::from(self.config.output.update_hz)
        )); }
        });
        effect_card(ui, "Wheel slip", |ui| {
            let slip = &mut self.config.effects.wheel_slip;
            ui.checkbox(&mut slip.enabled, "Enable wheel slip");
            percentage_slider(
                ui,
                &mut slip.threshold,
                "Sliding contact threshold",
                0.0..=1.0,
            );
            ui.add(egui::Slider::new(&mut slip.gain, 0.0..=10.0).text("Slip gain"));
            percentage_slider(
                ui,
                &mut slip.max_intensity,
                "Slip maximum intensity",
                0.0..=1.0,
            );
            if self.config.ui.show_debug {
                ui.small("LMU: maximum loaded-wheel sliding contact fraction (not longitudinal slip ratio). Active above 3 m/s.");
            }
        });
        effect_card(ui, "Kerbs / road", |ui| {
            let road = &mut self.config.effects.road;
            ui.checkbox(&mut road.enabled, "Enable kerbs / road");
            ui.add(
                egui::Slider::new(&mut road.threshold_mps, 0.0..=5.0)
                    .text("Travel speed threshold (m/s)"),
            );
            ui.add(egui::Slider::new(&mut road.gain, 0.0..=10.0).text("Road gain"));
            percentage_slider(
                ui,
                &mut road.kerb_intensity,
                "Kerb contact intensity",
                0.0..=1.0,
            );
            percentage_slider(
                ui,
                &mut road.max_intensity,
                "Road maximum intensity",
                0.0..=1.0,
            );
            ui.add(
                egui::Slider::new(&mut road.acceleration_threshold_mps2, 0.0..=10.0)
                    .text("Vertical vibration threshold (m/s²)"),
            );
            ui.add(
                egui::Slider::new(&mut road.acceleration_gain, 0.0..=2.0)
                    .text("Vertical vibration gain"),
            );
            ui.small("Lower thresholds for more sensitive road feedback. Bumps and grass can also trigger this effect.");
        });
        effect_card(ui, "Collision / impact", |ui| {
            let impact = &mut self.config.effects.impact;
            ui.checkbox(&mut impact.enabled, "Enable impacts");
            percentage_slider(
                ui,
                &mut impact.threshold,
                "Impact severity threshold",
                0.0..=1.0,
            );
            percentage_slider(
                ui,
                &mut impact.intensity,
                "Impact pulse intensity",
                0.0..=1.0,
            );
            if self.config.ui.show_debug {
                ui.small("LMU: a new explicit impact event, with acceleration / 100 m/s² as estimated severity. One pulse per event; braking alone cannot trigger it.");
            }
        });
        effect_card(ui, "Effect profiles", |ui| {
            ui.small(
                "Save and reuse your effect settings. Profiles keep your output limits unchanged.",
            );
            egui::ComboBox::from_id_salt("effect_profile")
                .selected_text(&self.profile_name)
                .show_ui(ui, |ui| {
                    for name in self.config.effect_profiles.keys() {
                        ui.selectable_value(&mut self.profile_name, name.clone(), name);
                    }
                });
            ui.add(egui::TextEdit::singleline(&mut self.profile_name).char_limit(48));
            ui.horizontal(|ui| {
                let name = self.profile_name.trim().to_owned();
                let valid =
                    !name.is_empty() && name.len() <= 48 && !name.chars().any(char::is_control);
                let room = self.config.effect_profiles.contains_key(&name)
                    || self.config.effect_profiles.len() < 16;
                if ui
                    .add_enabled(valid && room, egui::Button::new("Store profile"))
                    .clicked()
                {
                    self.config
                        .effect_profiles
                        .insert(name.clone(), self.config.effects.clone());
                }
                if ui
                    .add_enabled(
                        self.config.effect_profiles.contains_key(&name),
                        egui::Button::new("Apply profile"),
                    )
                    .clicked()
                {
                    self.config.effects = self.config.effect_profiles[&name].clone();
                }
                if ui.button("Delete profile").clicked() {
                    self.config.effect_profiles.remove(&name);
                }
            });
        });
        ui.add_space(12.0);
        percentage_slider(
            ui,
            &mut self.config.output.global_intensity,
            "Global intensity",
            0.0..=1.0,
        );
        percentage_slider(
            ui,
            &mut self.config.output.max_intensity,
            "Maximum output",
            0.0..=1.0,
        );
        ui.small("Changes apply immediately.");
    }

    fn settings(&mut self, ui: &mut egui::Ui, snapshot: &RuntimeSnapshot) {
        theme::page_title(
            ui,
            "Connect to the drive.",
            "Your devices, connection preferences and application settings.",
        );
        theme::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        theme::section(ui, "Lovense Remote / Game Mode", "Local control. Your chosen device.");
        ui.label("Enable LAN in Remote, then enter its address and port. Connect discovers toys.");
        ui.horizontal(|ui| {
            ui.label("Remote host / IP");
            ui.add(egui::TextEdit::singleline(&mut self.config.lovense.host).char_limit(253));
        });
        ui.horizontal(|ui| {
            ui.label("Game Mode port");
            let mut port = self.config.lovense.port.unwrap_or(0);
            if ui
                .add(egui::DragValue::new(&mut port).range(0..=u16::MAX))
                .changed()
            {
                self.config.lovense.port = (port != 0).then_some(port);
            }
            ui.small("0 = unset; use Remote's port");
        });
        ui.checkbox(
            &mut self.config.lovense.automatic_reconnect,
            "Reconnect automatically (up to 5 attempts)",
        );
        egui::CollapsingHeader::new("Connection options").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label("Protocol");
            ui.selectable_value(
                &mut self.config.lovense.protocol,
                LocalProtocol::Http,
                "HTTP",
            );
            ui.selectable_value(
                &mut self.config.lovense.protocol,
                LocalProtocol::Https,
                "HTTPS",
            );
            if ui.button("Local HTTP preset").clicked() {
                self.config.lovense.host = "127.0.0.1".into();
                self.config.lovense.port = Some(20010);
                self.config.lovense.protocol = LocalProtocol::Http;
            }
        });
        ui.add(
            egui::Slider::new(&mut self.config.lovense.request_timeout_ms, 100..=5_000)
                .text("API timeout (ms)"),
        );
        egui::ComboBox::from_id_salt("lovense_output_mode")
            .selected_text(self.config.lovense.output_mode.label())
            .show_ui(ui, |ui| {
                for mode in race2love_core::config::LovenseOutputMode::ALL {
                    ui.selectable_value(&mut self.config.lovense.output_mode, mode, mode.label());
                }
            });
        ui.small("Compare modes with the same effects/intensity. Connect, reselect the toy and Resume to apply.");
        ui.small("Direct Vibrate is the tested default. Pattern modes are experimental; switch back if they cycle or feel irregular.");
        });
        if let (Some(lovense), Some((device, demo))) = (&self.lovense, &self.devices) {
            let remote = lovense.snapshot();
            if remote.using_vibrate_fallback {
                ui.small("Remote rejected Pattern; using direct Vibrate until reconnect.");
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button("Connect / Discover toys").clicked()
                    && self.control.update_config(self.config.clone()).is_ok()
                {
                    self.control.select_device(device.clone());
                    lovense.connect(self.config.lovense.clone());
                }
                if ui.button("Disconnect / Use Demo output").clicked() {
                    self.control.emergency_stop();
                    lovense.disconnect();
                    self.control.select_device(demo.clone());
                }
                if ui
                    .add_enabled(
                        remote.selected_ready
                            && snapshot.device.connected
                            && snapshot.device.name == device.name()
                            && !snapshot.controls.emergency_stopped,
                        egui::Button::new("Test vibration · 1 second"),
                    )
                    .clicked()
                {
                    self.control.test_vibration();
                }
            });
            indicator(
                ui,
                remote.state == ConnectionState::Connected,
                remote.state.label(),
            );
            if self.config.ui.show_debug && let Some(endpoint) = &remote.endpoint {
                ui.small(format!("Active endpoint: {endpoint}"));
            }
            ui.small("After selecting a device, Resume output to enable feedback or test vibration.");
            ui.small("Testing pauses the game feed and respects your output limits.");
            if let Some(error) = &remote.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            ui.label("Select a device:");
            for toy in &remote.toys {
                ui.horizontal_wrapped(|ui| {
                    let allowed = toy.connected && toy.vibration != Some(false);
                    if ui
                        .add_enabled(
                            allowed,
                            egui::Button::selectable(
                                remote.selected.as_ref() == Some(&toy.id),
                                toy.label(),
                            ),
                        )
                        .clicked()
                    {
                        self.control.select_device(device.clone());
                        lovense.select_toy(toy.id.clone());
                    }
                    ui.small(if toy.connected {
                        "Connected"
                    } else {
                        "Disconnected"
                    });
                    if let Some(battery) = toy.battery {
                        ui.small(format!("Battery {battery}%"));
                    }
                    match toy.vibration {
                        Some(false) => {
                            ui.small("Vibration unsupported");
                        }
                        None => {
                            ui.small("Capabilities unavailable; verify vibration before testing");
                        }
                        _ => {}
                    }
                });
            }
            if remote.toys.is_empty() {
                ui.small("No toys detected. Pair a toy in Remote and click Connect.");
            }
        } else {
            ui.label("Device connection is unavailable.");
        }

        });
        theme::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        theme::section(ui, "Application", "Make Race2Love fit your setup.");
        ui.checkbox(
            &mut self.config.ui.start_minimized,
            "Start minimized on the next launch",
        );
        ui.checkbox(
            &mut self.config.ui.show_debug,
            "Debug mode",
        );
        if self.config.ui.show_debug {
        ui.add(egui::Slider::new(&mut self.config.ui.refresh_hz, 1..=60).text("UI refresh (Hz)"));
        ui.add(
            egui::Slider::new(&mut self.config.output.telemetry_hz, 1..=120)
                .text("Telemetry refresh (Hz)"),
        );
        ui.add(
            egui::Slider::new(&mut self.config.output.effects_hz, 1..=120)
                .text("Effects refresh (Hz)"),
        );
        ui.add(
            egui::Slider::new(&mut self.config.output.update_hz, 1..=60).text("Device output (Hz)"),
        );
        ui.add(
            egui::Slider::new(&mut self.config.output.telemetry_timeout_ms, 100..=2_000)
                .text("Telemetry timeout (ms)"),
        );
        ui.small("Defaults: telemetry/effects 60 Hz, output 25 Hz, UI 30 Hz. Higher rates consume more CPU.");
        ui.separator();
        ui.label("TOML configuration");
        if let Some(path) = &self.config_path {
            ui.monospace(path.display().to_string());
        }
        }
        if ui.button("Save settings").clicked() {
            self.save_settings();
        }
        ui.small("Changes are also saved when you close the app.");
        });
    }
}

impl eframe::App for Race2LoveApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.styled {
            theme::apply(ui.ctx());
            self.styled = true;
        }
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.control.emergency_stop();
        }
        let snapshot = self.control.snapshot();
        if self.config.ui.show_graphs {
            self.history.capture(
                &snapshot,
                std::time::Instant::now(),
                Duration::from_millis(self.config.output.telemetry_timeout_ms),
            );
        } else {
            self.history.clear();
        }
        let before = self.config.clone();
        egui::Frame::central_panel(ui.style())
            .inner_margin(18)
            .show(ui, |ui| {
                self.header(ui, &snapshot);
                ui.separator();
                if ui.available_width() >= 1000.0 {
                    egui::Panel::left("navigation")
                        .exact_size(198.0)
                        .resizable(false)
                        .frame(
                            egui::Frame::new()
                                .fill(theme::BG)
                                .inner_margin(egui::Margin {
                                    left: 0,
                                    right: 18,
                                    top: 18,
                                    bottom: 0,
                                }),
                        )
                        .show_inside(ui, |ui| {
                            ui.label(
                                egui::RichText::new("WORKSPACE")
                                    .size(10.0)
                                    .color(theme::MUTED),
                            );
                            self.navigation(ui, true);
                            ui.separator();
                            if ui.button("Save settings").clicked() {
                                self.save_settings();
                            }
                            ui.label(
                                egui::RichText::new(if self.dirty {
                                    "Unsaved changes"
                                } else {
                                    "Settings up to date"
                                })
                                .size(12.0)
                                .color(theme::MUTED),
                            );
                        });
                } else {
                    ui.horizontal_wrapped(|ui| self.navigation(ui, false));
                    ui.add_space(4.0);
                }
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: if ui.available_width() >= 720.0 { 12 } else { 0 },
                        right: 2,
                        top: 0,
                        bottom: 0,
                    })
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt(match self.page {
                                Page::Dashboard => "dashboard_scroll",
                                Page::Effects => "effects_scroll",
                                Page::Settings => "settings_scroll",
                            })
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add_space(8.0);
                                match self.page {
                                    Page::Dashboard => self.dashboard(ui, &snapshot),
                                    Page::Effects => self.effects(ui),
                                    Page::Settings => self.settings(ui, &snapshot),
                                }
                                if let Some(error) = &self.config_error {
                                    ui.colored_label(theme::RED, error);
                                }
                                if let Some(message) = &self.message {
                                    ui.separator();
                                    ui.label(message);
                                }
                                if self.dirty {
                                    ui.label(
                                        egui::RichText::new(
                                            "Unsaved changes · saved on normal exit",
                                        )
                                        .size(12.0)
                                        .color(theme::MUTED),
                                    );
                                }
                                ui.add_space(16.0);
                            });
                    });
            });
        if self.config != before {
            self.dirty = true;
            if self.config.lovense != before.lovense && self.lovense.is_some() {
                self.control.emergency_stop();
            }
            match self.control.update_config(self.config.clone()) {
                Ok(()) => self.config_error = None,
                Err(error) => self.config_error = Some(error.to_string()),
            }
        }
        let rate = if snapshot.effects.reason == StopReason::Running
            || snapshot
                .controls
                .test_pulse
                .is_some_and(|pulse| std::time::Instant::now() < pulse.until)
        {
            self.config.ui.refresh_hz.max(1)
        } else {
            2
        };
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f64(1.0 / f64::from(rate)));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.control.request_shutdown();
        if self.dirty {
            self.save_settings();
        }
    }
}

fn indicator(ui: &mut egui::Ui, connected: bool, label: &str) {
    let color = if connected {
        theme::ACCENT
    } else {
        theme::MUTED
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.0, color);
        ui.colored_label(color, label);
    });
}

fn effect_card(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    theme::card().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        egui::CollapsingHeader::new(egui::RichText::new(title).size(18.0).strong())
            .default_open(false)
            .show(ui, contents);
    });
}

fn meter(ui: &mut egui::Ui, label: &str, value: f32) {
    theme::bar(ui, label, value, theme::ACCENT);
}

fn percentage_slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    label: &str,
    range: std::ops::RangeInclusive<f32>,
) {
    ui.add(
        egui::Slider::new(value, range)
            .text(label)
            .custom_formatter(|value, _| format!("{:.0}%", value * 100.0))
            .custom_parser(|text| {
                text.trim()
                    .trim_end_matches('%')
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .map(|percent| percent / 100.0)
            }),
    );
}

pub fn run(
    control: RuntimeControl,
    config: Config,
    path: Option<PathBuf>,
    startup_message: Option<String>,
    lovense: LovenseControl,
    device: Arc<dyn HapticDevice>,
    demo: Arc<dyn HapticDevice>,
) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 860.0])
            .with_min_inner_size([620.0, 540.0])
            .with_app_id("race2love")
            .with_icon(egui::IconData::default()),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Race2Love",
        options,
        Box::new(move |creation| {
            if config.ui.start_minimized {
                creation
                    .egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            Ok(Box::new(
                Race2LoveApp::new(control, config, path, startup_message)
                    .with_lovense(lovense, device, demo),
            ))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;
    use race2love_core::{devices::MockDevice, runtime::RaceRuntime, telemetry::DemoSource};
    use std::{sync::Arc, time::Instant};

    struct TestUi {
        context: egui::Context,
        frame: eframe::Frame,
        time: f64,
        size: egui::Vec2,
    }

    impl TestUi {
        fn new() -> Self {
            Self {
                context: egui::Context::default(),
                frame: eframe::Frame::_new_kittest(),
                time: 0.0,
                size: egui::vec2(880.0, 1400.0),
            }
        }

        fn render(&mut self, app: &mut Race2LoveApp, events: Vec<egui::Event>) -> egui::FullOutput {
            self.time += 1.0 / 30.0;
            self.context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                },
                |ui| app.ui(ui, &mut self.frame),
            )
        }

        fn click(&mut self, app: &mut Race2LoveApp, label: &str) {
            let output = self.render(app, vec![]);
            let position = output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::epaint::Shape::Text(text) = &shape.shape
                        && text.galley.job.text == label
                    {
                        Some(text.pos + text.galley.size() * 0.5)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("Visible control not found: {label}"));
            self.render(
                app,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            self.render(
                app,
                vec![egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            self.render(app, vec![]);
        }
    }

    async fn wait_for(mut predicate: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !predicate() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("GUI control did not reach the runtime");
    }

    #[tokio::test]
    async fn gui_controls_stop_resume_edit_and_persist_settings() {
        let device = Arc::new(MockDevice::default());
        let runtime = RaceRuntime::spawn(
            Box::new(DemoSource::default()),
            device.clone(),
            Config::default(),
        )
        .unwrap();
        let path =
            std::env::temp_dir().join(format!("race2love-gui-test-{}.toml", std::process::id()));
        let mut app = Race2LoveApp::new(
            runtime.control.clone(),
            Config::default(),
            Some(path.clone()),
            None,
        );
        let mut ui = TestUi::new();
        ui.render(&mut app, vec![]);
        wait_for(|| device.intensity() > 0.0).await;
        ui.click(&mut app, "EMERGENCY STOP · Esc");
        assert!(runtime.control.snapshot().controls.emergency_stopped);
        wait_for(|| device.intensity() == 0.0).await;
        ui.click(&mut app, "Resume output");
        assert!(!runtime.control.snapshot().controls.emergency_stopped);
        wait_for(|| device.intensity() > 0.0).await;
        ui.click(&mut app, "Effects");
        assert!(app.page == Page::Effects);
        ui.click(&mut app, "Enable engine vibration");
        ui.click(&mut app, "Enable shift pulses");
        assert!(!app.config.effects.engine.enabled && !app.config.effects.gear_shift.enabled);
        wait_for(|| device.intensity() == 0.0).await;
        ui.click(&mut app, "Devices / Settings");
        assert!(app.page == Page::Settings);
        ui.click(&mut app, "Debug mode");
        assert!(app.config.ui.show_debug);
        ui.click(&mut app, "Connection options");
        ui.click(&mut app, "Direct Vibrate (previous)");
        ui.click(&mut app, "Pattern smoothing + dithering (experimental)");
        ui.click(&mut app, "Pattern smoothing + dithering (experimental)");
        ui.click(&mut app, "Direct Vibrate (previous)");
        assert_eq!(
            app.config.lovense.output_mode,
            race2love_core::config::LovenseOutputMode::Vibrate
        );
        ui.click(&mut app, "Save settings");
        assert_eq!(Config::load(&path).unwrap(), app.config);
        assert!(!app.dirty);
        ui.click(&mut app, "Dashboard");
        ui.click(&mut app, "Enable game input");
        wait_for(|| !runtime.control.snapshot().telemetry.connected).await;
        assert_eq!(device.intensity(), 0.0);
        ui.render(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: Some(egui::Key::Escape),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(runtime.control.snapshot().controls.emergency_stopped);
        let started = Instant::now();
        app.on_exit(None);
        runtime.shutdown().await;
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(device.intensity(), 0.0);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn gui_connect_reports_missing_port_and_disconnect_returns_to_demo_safely() {
        let demo = Arc::new(MockDevice::default());
        let runtime = RaceRuntime::spawn(
            Box::new(DemoSource::default()),
            demo.clone(),
            Config::default(),
        )
        .unwrap();
        let service = race2love_lovense::LovenseService::spawn();
        let mut app = Race2LoveApp::new(runtime.control.clone(), Config::default(), None, None)
            .with_lovense(
                service.control.clone(),
                service.device.clone(),
                demo.clone(),
            );
        let mut ui = TestUi::new();
        wait_for(|| demo.intensity() > 0.0).await;
        ui.click(&mut app, "Devices / Settings");
        ui.click(&mut app, "Connect / Discover toys");
        wait_for(|| service.control.snapshot().state == ConnectionState::Exhausted).await;
        assert!(service.control.snapshot().error.unwrap().contains("port"));
        assert!(runtime.control.snapshot().controls.emergency_stopped);
        wait_for(|| demo.intensity() == 0.0).await;
        ui.click(&mut app, "Disconnect / Use Demo output");
        wait_for(|| runtime.control.snapshot().device.name == demo.name()).await;
        assert_eq!(demo.intensity(), 0.0);
        ui.click(&mut app, "Resume output");
        wait_for(|| demo.intensity() > 0.0).await;
        runtime.shutdown().await;
        service.shutdown().await;
        assert_eq!(demo.intensity(), 0.0);
    }

    #[tokio::test]
    async fn dashboard_shows_normalized_lmu_values_and_demo_selection_latches_stop() {
        use race2love_core::telemetry::{MockTelemetrySource, TelemetryFrame};
        let device = Arc::new(MockDevice::default());
        let mut config = Config::default();
        config.output.telemetry_timeout_ms = 2_000;
        config.ui.show_graphs = true;
        let runtime = RaceRuntime::spawn(
            Box::new(MockTelemetrySource::with_frame(TelemetryFrame {
                speed_mps: 50.0,
                engine_rpm: 7_000.0,
                engine_max_rpm: 8_000.0,
                gear: 4,
                throttle: 0.75,
                brake: 0.125,
                car: Some("Fixture GT3".into()),
                session: Some("Race · Spa".into()),
                ..TelemetryFrame::default()
            })),
            device.clone(),
            config.clone(),
        )
        .unwrap();
        let mut app = Race2LoveApp::new(runtime.control.clone(), config, None, None);
        let mut ui = TestUi::new();
        wait_for(|| runtime.control.snapshot().telemetry.frame.is_some()).await;
        ui.size = egui::vec2(880.0, 3000.0);
        let normal = ui.render(&mut app, vec![]);
        let text = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::epaint::Shape::Text(text) = &shape.shape {
                        Some(text.galley.job.text.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        let labels = text(&normal);
        assert!(labels.iter().any(|label| label == "Mixer output"));
        for hidden in [
            "SPEED",
            "GEAR",
            "Fixture GT3",
            "Race · Spa",
            "Engine RPM",
            "Shift detected",
            "Body vertical acceleration",
        ] {
            assert!(
                !labels.iter().any(|label| label == hidden),
                "diagnostic visible without debug: {hidden}"
            );
        }
        app.page = Page::Settings;
        let labels = text(&ui.render(&mut app, vec![]));
        for hidden in [
            "UI refresh (Hz)",
            "Telemetry refresh (Hz)",
            "API timeout (ms)",
            "TOML configuration",
        ] {
            assert!(
                !labels.iter().any(|label| label == hidden),
                "advanced control visible by default: {hidden}"
            );
        }
        ui.click(&mut app, "Debug mode");
        assert!(app.config.ui.show_debug);
        let labels = text(&ui.render(&mut app, vec![]));
        assert!(labels.iter().any(|label| label == "Telemetry refresh (Hz)"));
        ui.click(&mut app, "Dashboard");
        let output = ui.render(&mut app, vec![]);
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    Some(text.galley.job.text.clone())
                } else {
                    None
                }
            })
            .collect();
        for expected in [
            "Fixture GT3",
            "Race · Spa",
            "180",
            "7000 / 8000 RPM",
            "4",
            "Throttle · 75%",
            "Brake · 12%",
        ] {
            assert!(
                labels.iter().any(|label| label == expected),
                "missing displayed value: {expected}; {labels:?}"
            );
        }
        ui.click(&mut app, "Demo");
        wait_for(|| runtime.control.snapshot().telemetry.source_name == "Demo").await;
        assert!(runtime.control.snapshot().controls.emergency_stopped);
        wait_for(|| device.intensity() == 0.0).await;
        #[cfg(any(windows, target_os = "linux"))]
        {
            let generation = runtime.control.snapshot().controls.source_generation;
            ui.click(&mut app, "Le Mans Ultimate");
            assert!(runtime.control.snapshot().controls.source_generation > generation);
            assert!(runtime.control.snapshot().controls.emergency_stopped);
            wait_for(|| runtime.control.snapshot().telemetry.source_name == "Le Mans Ultimate")
                .await;
            assert_eq!(device.intensity(), 0.0);
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let generation = runtime.control.snapshot().controls.source_generation;
            ui.click(&mut app, "Le Mans Ultimate");
            assert_eq!(
                runtime.control.snapshot().controls.source_generation,
                generation
            );
        }
        runtime.shutdown().await;
    }
    #[tokio::test]
    async fn safety_controls_remain_visible_on_every_page_after_scrolling() {
        let device = Arc::new(MockDevice::default());
        let runtime = RaceRuntime::spawn(
            Box::new(DemoSource::default()),
            device.clone(),
            Config::default(),
        )
        .unwrap();
        let mut app = Race2LoveApp::new(runtime.control.clone(), Config::default(), None, None);
        let mut ui = TestUi::new();
        wait_for(|| device.intensity() > 0.0).await;
        for size in [egui::vec2(620.0, 540.0), egui::vec2(1200.0, 860.0)] {
            ui.size = size;
            for page in [Page::Dashboard, Page::Effects, Page::Settings] {
                app.page = page;
                ui.render(&mut app, vec![]);
                ui.render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(egui::pos2(size.x - 60.0, size.y - 60.0)),
                        egui::Event::MouseWheel {
                            phase: egui::TouchPhase::Move,
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, -2000.0),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
                let output = ui.render(&mut app, vec![]);
                let stop = output
                    .shapes
                    .iter()
                    .find_map(|shape| {
                        if let egui::epaint::Shape::Text(text) = &shape.shape
                            && text.galley.job.text == "EMERGENCY STOP · Esc"
                        {
                            Some((
                                egui::Rect::from_min_size(text.pos, text.galley.size()),
                                shape.clip_rect,
                            ))
                        } else {
                            None
                        }
                    })
                    .expect("Stop is visible");
                assert!(stop.1.contains_rect(stop.0));
                assert!(stop.0.bottom() < 90.0);
                assert!(stop.0.right() < size.x);
                ui.click(&mut app, "EMERGENCY STOP · Esc");
                wait_for(|| device.intensity() == 0.0).await;
                assert!(runtime.control.snapshot().controls.emergency_stopped);
                ui.click(&mut app, "Resume output");
                assert!(!runtime.control.snapshot().controls.emergency_stopped);
            }
        }
        runtime.shutdown().await;
        assert_eq!(device.intensity(), 0.0);
    }
}
