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

/// First sample, neutral, reverse, duplicate frames and non-adjacent changes do
/// not count as normal shifts. Reset on telemetry loss to avoid reconnect pulses.
#[derive(Default)]
pub struct GearShiftDetector {
    previous: Option<i8>,
}

impl GearShiftDetector {
    pub fn update(&mut self, gear: i8) -> bool {
        let previous = self.previous.replace(gear);
        previous.is_some_and(|previous| {
            previous > 0 && gear > 0 && (i16::from(gear) - i16::from(previous)).abs() == 1
        })
    }
    pub fn reset(&mut self) {
        self.previous = None;
    }
}

#[derive(Default)]
pub struct EffectEngine {
    gear: GearShiftDetector,
    mixer: EffectMixer,
    last_frame: Option<Instant>,
}

impl EffectEngine {
    pub fn reset(&mut self) {
        self.gear.reset();
        self.mixer.clear();
        self.last_frame = None;
    }

    pub fn update(&mut self, frame: &TelemetryFrame, config: &EffectsConfig, now: Instant) -> f32 {
        if self.last_frame != Some(frame.timestamp) {
            self.last_frame = Some(frame.timestamp);
            let shifted = self.gear.update(frame.gear);
            if shifted && config.gear_shift.enabled {
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
        if !config.gear_shift.enabled {
            self.mixer.clear();
        }
        self.mixer.sample(
            &[ContinuousEffect {
                intensity: engine_intensity(frame, &config.engine),
                priority: 20,
            }],
            now,
        )
    }
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
}
