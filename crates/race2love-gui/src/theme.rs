//! Shared native UI styling and small drawing primitives. No image assets required.
use eframe::egui::{self, Color32, RichText};

pub const BG: Color32 = Color32::from_rgb(11, 15, 23);
pub const CARD: Color32 = Color32::from_rgb(19, 25, 37);
pub const BORDER: Color32 = Color32::from_rgb(41, 51, 68);
pub const TEXT: Color32 = Color32::from_rgb(231, 237, 247);
pub const MUTED: Color32 = Color32::from_rgb(153, 168, 190);
pub const ACCENT: Color32 = Color32::from_rgb(94, 226, 192);
pub const VIOLET: Color32 = Color32::from_rgb(170, 157, 255);
pub const RED: Color32 = Color32::from_rgb(255, 119, 141);

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    style.visuals = egui::Visuals::dark();
    let v = &mut style.visuals;
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = BG;
    v.faint_bg_color = CARD;
    v.selection.bg_fill = Color32::from_rgb(35, 78, 74);
    v.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = Color32::from_rgb(246, 201, 119);
    v.error_fg_color = RED;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, MUTED);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(30, 39, 54);
    v.widgets.inactive.bg_fill = Color32::from_rgb(38, 52, 66);
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(40, 58, 72);
    v.widgets.hovered.bg_fill = Color32::from_rgb(60, 139, 121);
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.weak_bg_fill = Color32::from_rgb(35, 78, 74);
    for widget in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
    ] {
        widget.corner_radius = egui::CornerRadius::same(6);
        widget.fg_stroke = egui::Stroke::new(1.0, TEXT);
    }
    style.spacing.item_spacing = egui::vec2(12.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 9.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.slider_width = 180.0;
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(23.0));
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    style.animation_time = 0.1;
    ctx.set_global_style(style);
}

pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(12)
        .inner_margin(18)
}

pub fn section(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(RichText::new(title).size(18.0).strong());
    if !description.is_empty() {
        ui.label(RichText::new(description).color(MUTED));
    }
    ui.add_space(3.0);
}

pub fn page_title(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(RichText::new(title).size(28.0).strong());
    ui.label(RichText::new(description).color(MUTED));
    ui.add_space(8.0);
}

pub fn brand(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.add(
            egui::Image::from_bytes(
                "bytes://race2love-icon.png",
                include_bytes!("../../../assets/race2love-icon.png"),
            )
            .fit_to_exact_size(egui::vec2(34.0, 34.0))
            .corner_radius(9.0),
        );
        ui.label(RichText::new("Race2Love").size(21.0).strong());
    });
}

pub fn metric(ui: &mut egui::Ui, title: &str, value: &str, detail: &str, color: Color32) {
    card().show(ui, |ui| {
        ui.set_min_width((ui.available_width()).max(0.0));
        ui.label(RichText::new(title).size(12.0).color(MUTED));
        ui.label(RichText::new(value).size(42.0).color(color).strong());
        ui.label(RichText::new(detail).size(12.0).color(MUTED));
    });
}

pub fn bar(ui: &mut egui::Ui, label: &str, value: f32, color: Color32) {
    let value = race2love_core::unit(value);
    ui.label(RichText::new(format!("{label} · {:.0}%", value * 100.0)).size(13.0));
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 3.0, BG);
    if value > 0.0 {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * value, rect.height())),
            3.0,
            color,
        );
    }
}

/// Draw the actual core RPM response so tuning stays consistent with output.
pub fn engine_preview(ui: &mut egui::Ui, config: &race2love_core::config::EngineConfig) {
    ui.label(
        RichText::new("RPM response · before global intensity")
            .size(12.0)
            .color(MUTED),
    );
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 92.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, BG);
    let plot = rect.shrink2(egui::vec2(12.0, 14.0));
    for fraction in [0.25, 0.5, 0.75] {
        let x = plot.left() + plot.width() * fraction;
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            egui::Stroke::new(1.0, BORDER),
        );
    }
    let points = (0..=80)
        .map(|step| {
            let ratio = step as f32 / 80.0;
            let frame = race2love_core::telemetry::TelemetryFrame {
                engine_rpm: ratio * 100.0,
                engine_max_rpm: 100.0,
                ..Default::default()
            };
            let level = race2love_core::effects::engine_intensity(&frame, config);
            egui::pos2(
                plot.left() + ratio * plot.width(),
                plot.bottom() - level * plot.height(),
            )
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(2.0, if config.enabled { ACCENT } else { MUTED }),
    ));
    response.on_hover_text("Horizontal: 0–100% of maximum RPM. Vertical: 0–100% vibration. The preview uses the same response function as the effect engine.");
}
