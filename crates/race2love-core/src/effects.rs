//! Independent generators operating solely on normalized telemetry.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{
    config::{EffectsConfig, EngineConfig},
    mixer::{ContinuousEffect, EffectMixer, HapticEffect},
    telemetry::TelemetryFrame,
    unit,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseCurve {
    #[default]
    Linear,
    Exponential,
    Logarithmic,
}

impl ResponseCurve {
    pub const ALL: [Self; 3] = [Self::Linear, Self::Exponential, Self::Logarithmic];
    pub fn name(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Exponential => "Exponential",
            Self::Logarithmic => "Logarithmic",
        }
    }
    /// Normalized endpoints; logarithmic curve is log10(1+9x).
    pub fn map(self, value: f32) -> f32 {
        let x = unit(value);
        match self {
            Self::Linear => x,
            Self::Exponential => x * x,
            Self::Logarithmic => (1.0 + 9.0 * x).log10(),
        }
    }
}

pub fn engine_intensity(frame: &TelemetryFrame, config: &EngineConfig) -> f32 {
    if !config.enabled
        || !frame.engine_rpm.is_finite()
        || !frame.engine_max_rpm.is_finite()
        || frame.engine_max_rpm <= 0.0
        || !config.start_ratio.is_finite()
        || !config.end_ratio.is_finite()
        || config.end_ratio <= config.start_ratio
    {
        return 0.0;
    }
    let ratio = frame.engine_rpm / frame.engine_max_rpm;
    if ratio < config.start_ratio {
        return 0.0;
    }
    let progress = unit((ratio - config.start_ratio) / (config.end_ratio - config.start_ratio));
    let minimum = unit(config.min_intensity);
    let maximum = unit(config.max_intensity).max(minimum);
    unit(minimum + (maximum - minimum) * config.curve.map(progress))
}

/// Brief neutral samples can occur during a forward shift. Keep the last forward
/// gear for at most 250 ms; reverse, long neutral and telemetry gaps reset it.
#[derive(Default)]
pub struct GearShiftDetector {
    previous: Option<i8>,
    neutral_since: Option<Instant>,
    last_sample: Option<Instant>,
}

impl GearShiftDetector {
    pub fn update(&mut self, gear: i8) -> bool {
        self.update_at(gear, Instant::now())
    }
    pub fn update_at(&mut self, gear: i8, now: Instant) -> bool {
        let grace = Duration::from_millis(250);
        if self
            .last_sample
            .is_some_and(|last| now < last || now.duration_since(last) > grace)
        {
            self.reset();
        }
        self.last_sample = Some(now);
        if gear == 0 {
            let since = *self.neutral_since.get_or_insert(now);
            if now.duration_since(since) > grace {
                self.previous = None;
            }
            return false;
        }
        if !(1..=32).contains(&gear) {
            self.reset();
            return false;
        }
        if self
            .neutral_since
            .take()
            .is_some_and(|since| now.duration_since(since) > grace)
        {
            self.previous = None;
        }
        let previous = self.previous.replace(gear);
        previous.is_some_and(|previous| {
            previous > 0 && gear > 0 && (i16::from(gear) - i16::from(previous)).abs() == 1
        })
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Per-effect values before global scaling; copied into the existing GUI snapshot.
#[derive(Clone, Copy, Debug, Default)]
pub struct EffectLevels {
    pub engine: f32,
    pub gear_shift: f32,
    pub wheel_slip: f32,
    pub road: f32,
    /// Absolute high-pass vertical acceleration (m/s²), before sensitivity/scaling.
    pub road_vibration: Option<f32>,
    pub impact: f32,
    pub shift_count: u64,
    pub last_shift: Option<Instant>,
}

#[derive(Default)]
pub struct EffectEngine {
    gear: GearShiftDetector,
    mixer: EffectMixer,
    last_frame: Option<Instant>,
    vertical_baseline: Option<f32>,
    last_update: Option<Instant>,
    last_impact_id: Option<u64>,
    impact_above: bool,
    last_impact_pulse: Option<Instant>,
    levels: EffectLevels,
}

impl EffectEngine {
    pub fn reset(&mut self) {
        self.gear.reset();
        self.mixer.clear();
        self.last_frame = None;
        self.vertical_baseline = None;
        self.last_update = None;
        self.last_impact_id = None;
        self.impact_above = false;
        self.last_impact_pulse = None;
        self.levels = EffectLevels::default();
    }
    pub fn levels(&self) -> EffectLevels {
        self.levels
    }

    pub fn update(&mut self, frame: &TelemetryFrame, config: &EffectsConfig, now: Instant) -> f32 {
        if self.last_frame != Some(frame.timestamp) {
            let sample_dt = self
                .last_frame
                .and_then(|last| frame.timestamp.checked_duration_since(last));
            let vertical = frame
                .vertical_acceleration
                .filter(|value| value.is_finite());
            self.levels.road_vibration = if frame.speed_mps.is_finite() && frame.speed_mps >= 3.0 {
                vertical.map(|value| {
                    let baseline = match (self.vertical_baseline, sample_dt) {
                        (Some(previous), Some(dt)) if dt <= Duration::from_millis(250) => {
                            previous + (value - previous) * (1.0 - (-dt.as_secs_f32() / 0.2).exp())
                        }
                        _ => value, // Establish a baseline; never pulse on connect/resume.
                    };
                    self.vertical_baseline = Some(baseline);
                    (value - baseline).abs()
                })
            } else {
                self.vertical_baseline = None;
                None
            };
            if vertical.is_none() {
                self.vertical_baseline = None;
            }
            let first = self.last_frame.is_none();
            let shifted = self.gear.update_at(frame.gear, frame.timestamp);
            if shifted {
                self.levels.shift_count = self.levels.shift_count.saturating_add(1);
                self.levels.last_shift = Some(now);
                if config.gear_shift.enabled {
                    let shift = &config.gear_shift;
                    self.mixer.push(
                        HapticEffect {
                            intensity: shift.intensity,
                            priority: 160,
                            attack: Duration::from_millis(shift.attack_ms),
                            hold: Duration::from_millis(shift.hold_ms),
                            release: Duration::from_millis(shift.release_ms),
                        },
                        now,
                    );
                }
            }
            let above = frame.impact.is_some_and(|value| {
                value.is_finite() && value > 0.0 && value >= config.impact.threshold
            });
            let new_impact = frame
                .impact_id
                .map_or(!self.impact_above, |id| self.last_impact_id != Some(id));
            if let Some(id) = frame.impact_id {
                self.last_impact_id = Some(id);
            }
            if !first
                && above
                && new_impact
                && config.impact.enabled
                && self.last_impact_pulse.is_none_or(|last| {
                    now.saturating_duration_since(last) >= Duration::from_millis(300)
                })
            {
                self.mixer.push(
                    HapticEffect {
                        intensity: config.impact.intensity,
                        priority: 220,
                        attack: Duration::from_millis(5),
                        hold: Duration::from_millis(80),
                        release: Duration::from_millis(180),
                    },
                    now,
                );
                self.last_impact_pulse = Some(now);
            }
            self.impact_above = above;
            self.last_frame = Some(frame.timestamp);
        }
        if !config.gear_shift.enabled {
            self.mixer.clear_priority(160);
        }
        if !config.impact.enabled {
            self.mixer.clear_priority(220);
        }
        let dt = self.last_update.replace(now).map_or(0.0, |last| {
            now.saturating_duration_since(last).as_secs_f32().min(0.25)
        });
        let moving = frame.speed_mps.is_finite() && frame.speed_mps >= 3.0;
        let slip = max_signal(frame.wheel_slip);
        let road = max_signal(frame.suspension_velocity);
        let kerb = frame
            .kerb_contact
            .is_some_and(|wheels| wheels.into_iter().any(|contact| contact));
        let slip_target = if config.wheel_slip.enabled && moving {
            slip.map_or(0.0, |value| {
                unit((value - config.wheel_slip.threshold).max(0.0) * config.wheel_slip.gain)
                    .min(unit(config.wheel_slip.max_intensity))
            })
        } else {
            0.0
        };
        let road_target = if config.road.enabled && moving {
            let suspension = road.map_or(0.0, |value| {
                unit((value - config.road.threshold_mps).max(0.0) * config.road.gain)
                    .min(unit(config.road.max_intensity))
            });
            let vibration = self.levels.road_vibration.map_or(0.0, |value| {
                unit(
                    (value - config.road.acceleration_threshold_mps2).max(0.0)
                        * config.road.acceleration_gain,
                )
            });
            suspension
                .max(vibration)
                .max(if kerb {
                    unit(config.road.kerb_intensity)
                } else {
                    0.0
                })
                .min(unit(config.road.max_intensity))
        } else {
            0.0
        };
        self.levels.engine = engine_intensity(frame, &config.engine);
        self.levels.wheel_slip = if config.wheel_slip.enabled && slip.is_some() && moving {
            follow(self.levels.wheel_slip, slip_target, dt, 0.03, 0.10)
                .min(unit(config.wheel_slip.max_intensity))
        } else {
            0.0
        };
        self.levels.road = if config.road.enabled
            && (road.is_some()
                || self.levels.road_vibration.is_some()
                || frame.kerb_contact.is_some())
            && moving
        {
            follow(self.levels.road, road_target, dt, 0.01, 0.08)
                .min(unit(config.road.max_intensity))
        } else {
            0.0
        };
        self.levels.gear_shift = self.mixer.level_at_priority(160, now);
        self.levels.impact = self.mixer.level_at_priority(220, now);
        self.mixer.sample(
            &[
                ContinuousEffect {
                    intensity: self.levels.engine,
                    priority: 20,
                },
                ContinuousEffect {
                    intensity: self.levels.wheel_slip,
                    priority: 20,
                },
                ContinuousEffect {
                    intensity: self.levels.road,
                    priority: 20,
                },
            ],
            now,
        )
    }
}

/// Reject a malformed optional wheel array as unavailable, rather than allowing
/// one NaN/inf to turn a sensor into sustained full output.
pub fn max_signal(values: Option<[f32; 4]>) -> Option<f32> {
    let values = values?;
    values
        .iter()
        .all(|v| v.is_finite())
        .then(|| values.into_iter().map(f32::abs).fold(0.0, f32::max))
}

fn follow(previous: f32, target: f32, dt: f32, attack: f32, release: f32) -> f32 {
    let seconds = if target > previous { attack } else { release };
    let value = unit(previous + (target - previous) * (1.0 - (-dt / seconds).exp()));
    if value < 0.0001 { 0.0 } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpm_thresholds_and_all_curves() {
        let mut frame = TelemetryFrame {
            engine_rpm: 1_000.0,
            engine_max_rpm: 10_000.0,
            ..TelemetryFrame::default()
        };
        let mut config = EngineConfig {
            start_ratio: 0.2,
            end_ratio: 0.8,
            min_intensity: 0.1,
            max_intensity: 0.9,
            ..EngineConfig::default()
        };
        assert_eq!(engine_intensity(&frame, &config), 0.0);
        frame.engine_rpm = 2_000.0;
        assert!((engine_intensity(&frame, &config) - 0.1).abs() < 0.0001);
        frame.engine_rpm = 5_000.0;
        assert!((engine_intensity(&frame, &config) - 0.5).abs() < 0.0001);
        config.curve = ResponseCurve::Exponential;
        assert!((engine_intensity(&frame, &config) - 0.3).abs() < 0.0001);
        config.curve = ResponseCurve::Logarithmic;
        assert!(engine_intensity(&frame, &config) > 0.5);
        frame.engine_rpm = 20_000.0;
        assert!((engine_intensity(&frame, &config) - 0.9).abs() < 0.0001);
        for curve in ResponseCurve::ALL {
            assert_eq!(curve.map(0.0), 0.0);
            assert_eq!(curve.map(1.0), 1.0);
            assert_eq!(curve.map(f32::NAN), 0.0);
        }
        frame.engine_max_rpm = 0.0;
        assert_eq!(engine_intensity(&frame, &config), 0.0);
    }

    #[test]
    fn only_adjacent_forward_gear_changes_count() {
        let mut detector = GearShiftDetector::default();
        assert!(!detector.update(1));
        assert!(!detector.update(1));
        assert!(detector.update(2));
        assert!(detector.update(1));
        assert!(!detector.update(0));
        assert!(!detector.update(1));
        assert!(!detector.update(-1));
        assert!(!detector.update(3));
        assert!(!detector.update(6));
        detector.reset();
        assert!(!detector.update(2));
    }

    #[test]
    fn brief_neutral_bridges_forward_shifts_but_long_neutral_and_gaps_do_not() {
        let start = Instant::now();
        let mut detector = GearShiftDetector::default();
        for (ms, gear, shifted) in [
            (0, 3, false),
            (20, 0, false),
            (40, 4, true),
            (60, 4, false),
            (80, 0, false),
            (100, 3, true),
            (120, 0, false),
            (300, 0, false),
            (400, 4, false),
            (800, 5, false),
            (820, -1, false),
            (840, 6, false),
        ] {
            assert_eq!(
                detector.update_at(gear, start + Duration::from_millis(ms)),
                shifted,
                "gear {gear} at {ms}ms"
            );
        }
    }

    #[test]
    fn slip_and_road_are_independent_smoothed_bounded_and_require_valid_moving_telemetry() {
        let start = Instant::now();
        let mut engine = EffectEngine::default();
        let mut config = EffectsConfig::default();
        config.engine.enabled = false;
        config.gear_shift.enabled = false;
        config.wheel_slip.enabled = true;
        config.road.enabled = true;
        let mut frame = TelemetryFrame {
            timestamp: start,
            speed_mps: 30.0,
            wheel_slip: Some([0.9; 4]),
            suspension_velocity: Some([1.0; 4]),
            ..Default::default()
        };
        assert_eq!(engine.update(&frame, &config, start), 0.0);
        for ms in 1..=200 {
            frame.timestamp = start + Duration::from_millis(ms);
            assert!((0.0..=1.0).contains(&engine.update(&frame, &config, frame.timestamp)));
        }
        assert!(engine.levels().wheel_slip > 0.5 && engine.levels().wheel_slip <= 0.6);
        assert!(engine.levels().road > 0.4 && engine.levels().road <= 0.5);
        config.wheel_slip.enabled = false;
        engine.update(&frame, &config, frame.timestamp);
        assert_eq!(engine.levels().wheel_slip, 0.0);
        assert!(engine.levels().road > 0.4);
        config.road.max_intensity = 0.1;
        engine.update(&frame, &config, frame.timestamp);
        assert!(engine.levels().road <= 0.1);
        frame.suspension_velocity = Some([f32::NAN; 4]);
        assert_eq!(engine.update(&frame, &config, frame.timestamp), 0.0);
        frame.suspension_velocity = Some([1.0; 4]);
        frame.speed_mps = 0.0;
        assert_eq!(
            engine.update(&frame, &config, start + Duration::from_secs(1)),
            0.0
        );
    }

    #[test]
    fn impact_requires_new_event_never_replays_and_is_independent_of_shift_toggle() {
        let start = Instant::now();
        let mut engine = EffectEngine::default();
        let mut config = EffectsConfig::default();
        config.engine.enabled = false;
        config.gear_shift.enabled = false;
        config.impact.enabled = true;
        let mut frame = TelemetryFrame {
            timestamp: start,
            impact: Some(0.8),
            impact_id: Some(1),
            ..Default::default()
        };
        assert_eq!(engine.update(&frame, &config, start), 0.0); // Old event at connect.
        frame.timestamp = start + Duration::from_millis(20);
        frame.impact_id = Some(2);
        engine.update(&frame, &config, frame.timestamp);
        frame.timestamp = start + Duration::from_millis(40);
        assert!(engine.update(&frame, &config, frame.timestamp) > 0.7);
        assert!(engine.levels().impact > 0.7); // Shift disabled must not clear it.
        frame.timestamp = start + Duration::from_millis(400);
        assert_eq!(engine.update(&frame, &config, frame.timestamp), 0.0); // Same retained ID.
        frame.impact_id = Some(3);
        frame.timestamp += Duration::from_millis(20);
        engine.update(&frame, &config, frame.timestamp);
        config.impact.enabled = false;
        assert_eq!(
            engine.update(&frame, &config, frame.timestamp + Duration::from_millis(20)),
            0.0
        );
        config.impact.enabled = true;
        assert_eq!(
            engine.update(&frame, &config, frame.timestamp + Duration::from_millis(40)),
            0.0
        );
        engine.reset();
        assert_eq!(
            engine.update(&frame, &config, frame.timestamp + Duration::from_secs(1)),
            0.0
        );
    }

    #[test]
    fn duplicate_samples_do_not_retrigger_shift_and_disable_clears_pulse() {
        let now = Instant::now();
        let mut config = EffectsConfig::default();
        config.engine.enabled = false;
        let mut engine = EffectEngine::default();
        let mut frame = TelemetryFrame {
            timestamp: now,
            gear: 1,
            ..TelemetryFrame::default()
        };
        assert_eq!(engine.update(&frame, &config, now), 0.0);
        frame.timestamp = now + Duration::from_millis(20);
        frame.gear = 2;
        engine.update(&frame, &config, frame.timestamp);
        assert!(engine.update(&frame, &config, now + Duration::from_millis(50)) > 0.0);
        assert_eq!(
            engine.update(&frame, &config, now + Duration::from_millis(200)),
            0.0
        );
        frame.timestamp = now + Duration::from_millis(220);
        frame.gear = 3;
        engine.update(&frame, &config, frame.timestamp);
        config.gear_shift.enabled = false;
        assert_eq!(
            engine.update(&frame, &config, now + Duration::from_millis(250)),
            0.0
        );
    }
    #[test]
    fn road_vibration_works_without_kerb_flags_and_rejects_static_bias() {
        let start = Instant::now();
        let mut engine = EffectEngine::default();
        let mut config = EffectsConfig::default();
        config.engine.enabled = false;
        config.road.enabled = true;
        let mut frame = TelemetryFrame {
            timestamp: start,
            speed_mps: 30.0,
            vertical_acceleration: Some(9.81),
            kerb_contact: Some([false; 4]),
            suspension_velocity: Some([0.0; 4]),
            ..Default::default()
        };
        for tick in 0..20 {
            frame.timestamp = start + Duration::from_millis(tick * 20);
            assert_eq!(engine.update(&frame, &config, frame.timestamp), 0.0);
        }
        for tick in 20..30 {
            frame.timestamp = start + Duration::from_millis(tick * 20);
            frame.vertical_acceleration = Some(if tick % 2 == 0 { 18.0 } else { 2.0 });
            let output = engine.update(&frame, &config, frame.timestamp);
            assert!(output > 0.4 && output <= config.road.max_intensity);
        }
        let vibration = engine.levels().road_vibration;
        engine.update(&frame, &config, frame.timestamp + Duration::from_millis(5));
        assert_eq!(engine.levels().road_vibration, vibration); // Duplicates do not advance filter.
        frame.timestamp += Duration::from_secs(1);
        frame.vertical_acceleration = Some(9.81);
        engine.update(&frame, &config, frame.timestamp);
        assert_eq!(engine.levels().road_vibration, Some(0.0)); // Gap re-establishes baseline.
        frame.timestamp += Duration::from_millis(20);
        frame.speed_mps = 0.0;
        assert_eq!(engine.update(&frame, &config, frame.timestamp), 0.0);
        frame.timestamp += Duration::from_millis(20);
        frame.speed_mps = 30.0;
        frame.vertical_acceleration = Some(f32::NAN);
        engine.update(&frame, &config, frame.timestamp);
        assert_eq!(engine.levels().road_vibration, None);
        engine.reset();
        assert_eq!(engine.levels().road_vibration, None);
    }

    #[test]
    fn explicit_kerb_adds_to_engine_even_without_suspension_movement() {
        let start = Instant::now();
        let mut engine = EffectEngine::default();
        let mut config = EffectsConfig::default();
        config.road.enabled = true;
        let mut frame = TelemetryFrame {
            timestamp: start,
            speed_mps: 30.0,
            engine_rpm: 8_000.0,
            engine_max_rpm: 8_000.0,
            kerb_contact: Some([true, false, false, false]),
            ..Default::default()
        };
        engine.update(&frame, &config, start);
        frame.timestamp += Duration::from_millis(20);
        let mixed = engine.update(&frame, &config, frame.timestamp);
        assert!(engine.levels().road > 0.4);
        assert!(mixed > engine.levels().engine && mixed <= 1.0);
    }
}
