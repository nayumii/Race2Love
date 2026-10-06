//! Native desktop views. Render code only reads channel snapshots and sends
//! controls; telemetry reads and device communication stay in core workers.

use std::{path::PathBuf, time::Duration};

use eframe::egui;
use race2love_core::{
    config::Config,
    effects::ResponseCurve,
    runtime::{RuntimeControl, RuntimeSnapshot, StopReason},
    unit,
};

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
        }
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

    fn header(&mut self, ui: &mut egui::Ui, snapshot: &RuntimeSnapshot) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Race2Love");
            ui.label("Phase 1 · Demo");
            ui.separator();
            for (page, title) in [
                (Page::Dashboard, "Dashboard"),
                (Page::Effects, "Effects"),
                (Page::Settings, "Devices / Settings"),
            ] {
                ui.selectable_value(&mut self.page, page, title);
            }
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("EMERGENCY STOP · Esc").color(egui::Color32::WHITE),
                    )
                    .fill(egui::Color32::from_rgb(160, 35, 45)),
                )
                .clicked()
            {
                self.control.emergency_stop();
            }
            if snapshot.controls.emergency_stopped && ui.button("Resume output").clicked() {
                self.control.resume();
            }
            ui.label(snapshot.effects.reason.label());
        });
        ui.separator();
    }

    fn dashboard(&mut self, ui: &mut egui::Ui, snapshot: &RuntimeSnapshot) {
        ui.heading("Telemetry & output");
        ui.label("Tune racing effects with synthetic Demo telemetry.");
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            indicator(
                ui,
                snapshot.telemetry.connected,
                &format!("{} telemetry", snapshot.telemetry.source_name),
            );
            indicator(ui, snapshot.device.connected, "Mock device");
            indicator(ui, false, "LMU · Phase 3 / 4");
            indicator(ui, false, "Lovense · Phase 2");
        });
        let mut enabled = snapshot.controls.source_enabled;
        if ui.checkbox(&mut enabled, "Run Demo telemetry").changed() {
            self.control.set_source_enabled(enabled);
        }
        if let Some(error) = &snapshot.telemetry.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Some(error) = &snapshot.device.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        ui.separator();
        if let Some(frame) = &snapshot.telemetry.frame {
            egui::Grid::new("telemetry_values")
                .num_columns(2)
                .spacing([24.0, 10.0])
                .show(ui, |ui| {
                    value_row(ui, "Session", frame.session.as_deref().unwrap_or("Unknown"));
                    value_row(ui, "Car", frame.car.as_deref().unwrap_or("Unknown"));
                    value_row(
                        ui,
                        "Speed",
                        &format!(
                            "{:.0} km/h  ({:.1} m/s)",
                            frame.speed_mps * 3.6,
                            frame.speed_mps
                        ),
                    );
                    value_row(
                        ui,
                        "Engine",
                        &format!("{:.0} / {:.0} RPM", frame.engine_rpm, frame.engine_max_rpm),
                    );
                    let gear = match frame.gear {
                        -1 => "R".into(),
                        0 => "N".into(),
                        gear => gear.to_string(),
                    };
                    value_row(ui, "Gear", &gear);
                });
            ui.add_space(8.0);
            let ratio = if frame.engine_max_rpm > 0.0 {
                frame.engine_rpm / frame.engine_max_rpm
            } else {
                0.0
            };
            meter(ui, "RPM", ratio);
            meter(ui, "Throttle", frame.throttle);
            meter(ui, "Brake", frame.brake);
        } else {
            ui.label("Demo is paused or waiting for its first sample.");
        }
        ui.add_space(12.0);
        meter(ui, "Mixed effects", snapshot.effects.mixed);
        meter(ui, "Scaled target", snapshot.effects.intensity);
        meter(ui, "Applied mock output", snapshot.device.intensity);
        ui.small("Demo output is an in-memory value. It does not command a physical device.");
        ui.separator();
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
        if self.config.ui.show_debug {
            ui.separator();
            ui.label("Normalized signals · synthetic Demo data");
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
                ui.monospace(format!("Explicit synthetic impact: {:?}", frame.impact));
            }
        }
    }

    fn effects(&mut self, ui: &mut egui::Ui) {
        ui.heading("Engine RPM");
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
        ui.separator();
        ui.heading("Gear shift");
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
        ui.small("Adjacent forward gears generate pulses. Neutral, reverse and reconnects do not.");
        ui.separator();
        ui.heading("Further effects · Phase 6");
        ui.label(
            "Wheel slip, kerbs and collisions need verified LMU signals before they drive output.",
        );
        ui.label("Demo already exposes synthetic versions of these signals in the debug display.");
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
        ui.small("Valid changes apply immediately. Stop remains latched until Resume output.");
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Lovense · Phase 2");
        ui.label(
            "Connection preferences can be saved now. The local API adapter is the next phase.",
        );
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
            ui.small("0 = unset; use the port shown by Remote");
        });
        ui.checkbox(
            &mut self.config.lovense.automatic_reconnect,
            "Reconnect automatically (Phase 2)",
        );
        ui.add(
            egui::Slider::new(&mut self.config.lovense.request_timeout_ms, 100..=5_000)
                .text("API timeout (ms)"),
        );
        ui.add_enabled(
            false,
            egui::Button::new("Discover / Connect / Test vibration · Phase 2"),
        );
        ui.label("Detected toys: none · mock device active");
        ui.separator();
        ui.heading("Application");
        ui.checkbox(
            &mut self.config.ui.start_minimized,
            "Start minimized on the next launch",
        );
        ui.checkbox(
            &mut self.config.ui.show_debug,
            "Show telemetry debug values on Dashboard",
        );
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
        ui.small("Minimize-to-tray and automatic launch are not implemented in Phase 1.");
        ui.separator();
        ui.label("TOML configuration");
        if let Some(path) = &self.config_path {
            ui.monospace(path.display().to_string());
        }
        if ui.button("Save settings").clicked() {
            self.save_settings();
        }
        ui.small("Changed settings are also saved on normal exit. Runtime state is never saved.");
    }
}

impl eframe::App for Race2LoveApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.control.emergency_stop();
        }
        let snapshot = self.control.snapshot();
        let before = self.config.clone();
        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            self.header(ui, &snapshot);
            egui::ScrollArea::vertical().show(ui, |ui| {
                match self.page {
                    Page::Dashboard => self.dashboard(ui, &snapshot),
                    Page::Effects => self.effects(ui),
                    Page::Settings => self.settings(ui),
                }
                if let Some(error) = &self.config_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                if let Some(message) = &self.message {
                    ui.separator();
                    ui.label(message);
                }
                if self.dirty {
                    ui.small("Settings changed · save now or close normally to save");
                }
            });
        });
        if self.config != before {
            self.dirty = true;
            match self.control.update_config(self.config.clone()) {
                Ok(()) => self.config_error = None,
                Err(error) => self.config_error = Some(error.to_string()),
            }
        }
        let rate = if snapshot.effects.reason == StopReason::Running {
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
        egui::Color32::from_rgb(90, 200, 120)
    } else {
        egui::Color32::GRAY
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.0, color);
        ui.colored_label(color, label);
    });
}

fn value_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(label);
    ui.strong(value);
    ui.end_row();
}

fn meter(ui: &mut egui::Ui, label: &str, value: f32) {
    ui.add(
        egui::ProgressBar::new(unit(value)).text(format!("{label} · {:.0}%", unit(value) * 100.0)),
    );
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
) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([880.0, 720.0])
            .with_min_inner_size([620.0, 480.0])
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
            Ok(Box::new(Race2LoveApp::new(
                control,
                config,
                path,
                startup_message,
            )))
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
    }

    impl TestUi {
        fn new() -> Self {
            Self {
                context: egui::Context::default(),
                frame: eframe::Frame::_new_kittest(),
                time: 0.0,
            }
        }

        fn render(&mut self, app: &mut Race2LoveApp, events: Vec<egui::Event>) -> egui::FullOutput {
            self.time += 1.0 / 30.0;
            self.context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(880.0, 720.0),
                    )),
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
        ui.click(&mut app, "Show telemetry debug values on Dashboard");
        assert!(app.config.ui.show_debug);
        ui.click(&mut app, "Save settings");
        assert_eq!(Config::load(&path).unwrap(), app.config);
        assert!(!app.dirty);
        ui.click(&mut app, "Dashboard");
        ui.click(&mut app, "Run Demo telemetry");
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
}
