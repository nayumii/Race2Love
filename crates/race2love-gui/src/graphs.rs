//! Bounded tuning graphs drawn with the existing egui painter (no extra crate).
use eframe::egui;
use race2love_core::{
    effects::max_signal,
    runtime::{RuntimeSnapshot, telemetry_is_fresh},
};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(20);
const PERIOD: Duration = Duration::from_millis(50);
const CAPACITY: usize = 400;

struct Point {
    at: Instant,
    values: [Option<f32>; 5],
}
#[derive(Default)]
pub(crate) struct History {
    points: VecDeque<Point>,
    last_source: Option<u64>,
}
impl History {
    pub fn clear(&mut self) {
        self.points.clear();
        self.last_source = None;
    }
    pub fn capture(&mut self, snapshot: &RuntimeSnapshot, now: Instant, timeout: Duration) {
        let generation = snapshot.controls.source_generation;
        if self.last_source != Some(generation) {
            self.clear();
            self.last_source = Some(generation);
        }
        if self
            .points
            .back()
            .is_some_and(|p| now.saturating_duration_since(p.at) < PERIOD)
        {
            return;
        }
        let frame = snapshot.telemetry.frame.as_ref().filter(|_| {
            snapshot.telemetry.connected
                && snapshot.telemetry.source_generation == generation
                && telemetry_is_fresh(snapshot.telemetry.frame.as_ref(), now, timeout)
        });
        let values = [
            frame.map(|f| f.engine_rpm),
            frame.and_then(|f| max_signal(f.wheel_slip)),
            frame.and_then(|f| max_signal(f.suspension_velocity)),
            frame.and_then(|f| f.vertical_acceleration),
            Some(snapshot.effects.mixed),
        ];
        self.push(now, values);
    }
    fn push(&mut self, now: Instant, values: [Option<f32>; 5]) {
        while self
            .points
            .front()
            .is_some_and(|p| now.saturating_duration_since(p.at) >= WINDOW)
            || self.points.len() >= CAPACITY
        {
            self.points.pop_front();
        }
        self.points.push_back(Point {
            at: now,
            values: values.map(|v| v.filter(|n| n.is_finite())),
        });
    }
    pub fn show(&self, ui: &mut egui::Ui, now: Instant) {
        ui.small(
            "Last 20 seconds · 20 samples/s · hover for values · gaps mean unavailable telemetry",
        );
        for (index, (label, unit)) in [
            ("Engine RPM", "RPM"),
            ("Tyre sliding contact", "fraction"),
            ("Suspension travel speed", "m/s"),
            ("Body vertical acceleration", "m/s²"),
            ("Mixer output", "0–1"),
        ]
        .into_iter()
        .enumerate()
        {
            ui.label(label);
            let (rect, response) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 95.0), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
            let (min, max) = self
                .points
                .iter()
                .filter_map(|p| p.values[index])
                .fold((0.0_f32, 0.0_f32), |(lo, hi), n| (lo.min(n), hi.max(n)));
            let max = if max - min < 0.001 { min + 1.0 } else { max };
            let plot = rect.shrink(6.0);
            let position = |point: &Point, value: f32| {
                egui::pos2(
                    plot.right()
                        - now.saturating_duration_since(point.at).as_secs_f32()
                            / WINDOW.as_secs_f32()
                            * plot.width(),
                    plot.bottom() - (value - min) / (max - min) * plot.height(),
                )
            };
            let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(90, 200, 170));
            let mut segment = Vec::with_capacity(self.points.len());
            for point in &self.points {
                if let Some(value) = point.values[index] {
                    segment.push(position(point, value));
                } else if !segment.is_empty() {
                    painter.add(egui::Shape::line(std::mem::take(&mut segment), stroke));
                }
            }
            if !segment.is_empty() {
                painter.add(egui::Shape::line(segment, stroke));
            }
            painter.text(
                plot.left_top(),
                egui::Align2::LEFT_TOP,
                format!("{max:.2} {unit}"),
                egui::FontId::monospace(10.0),
                ui.visuals().weak_text_color(),
            );
            painter.text(
                plot.left_bottom(),
                egui::Align2::LEFT_BOTTOM,
                format!("{min:.2}"),
                egui::FontId::monospace(10.0),
                ui.visuals().weak_text_color(),
            );
            if let Some(pointer) = response.hover_pos() {
                let ago = ((plot.right() - pointer.x) / plot.width()).clamp(0.0, 1.0)
                    * WINDOW.as_secs_f32();
                if let Some(point) = self.points.iter().min_by(|a, b| {
                    let distance =
                        |p: &Point| (now.saturating_duration_since(p.at).as_secs_f32() - ago).abs();
                    distance(a).total_cmp(&distance(b))
                }) {
                    response.on_hover_text(point.values[index].map_or_else(
                        || "Unavailable".into(),
                        |value| format!("{ago:.2}s ago: {value:.3} {unit}"),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_is_bounded_ages_out_and_preserves_missing_samples() {
        let start = Instant::now();
        let mut history = History::default();
        for sample in 0..10_000 {
            history.push(start + PERIOD * sample, [Some(sample as f32); 5]);
        }
        assert_eq!(history.points.len(), CAPACITY);
        let later = start + Duration::from_secs(600);
        history.push(
            later,
            [
                Some(f32::NAN),
                None,
                Some(0.0),
                Some(f32::INFINITY),
                Some(0.3),
            ],
        );
        assert_eq!(history.points.len(), 1);
        assert_eq!(
            history.points[0].values,
            [None, None, Some(0.0), None, Some(0.3)]
        );
        history.clear();
        assert!(history.points.is_empty());
    }
}
