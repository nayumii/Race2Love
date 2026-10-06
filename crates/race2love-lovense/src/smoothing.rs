//! Fractional native-level shaping; no timers, threads, or telemetry knowledge.
//!
//! Pattern slots are 110 ms: the API table requires >100 ms, although an example
//! uses 100. All slots explicitly cover the existing two-second finite lease.
//! Source: https://developer.lovense.com/docs/standard-solutions/standard-api

use std::time::{Duration, Instant};

use race2love_core::{config::LovenseOutputMode, unit};

use crate::{LEASE, RENEWAL};

pub(crate) const INTERVAL: Duration = Duration::from_millis(110);
pub(crate) const SAMPLES: usize = 19;
const RAMP: Duration = Duration::from_millis(60);
const FAST_CHANGE: f32 = 2.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Output {
    Vibrate(u8),
    Pattern([u8; SAMPLES]),
}

impl Output {
    fn maximum(&self) -> u8 {
        match self {
            Self::Vibrate(level) => *level,
            Self::Pattern(levels) => levels.iter().copied().max().unwrap_or(0),
        }
    }
    fn level(&self, elapsed: Duration) -> u8 {
        match self {
            Self::Vibrate(level) => *level,
            Self::Pattern(levels) => {
                let index = (elapsed.as_millis() / INTERVAL.as_millis()) as usize;
                levels[index.min(SAMPLES - 1)]
            }
        }
    }
    fn equivalent(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Vibrate(level), Self::Pattern(levels))
            | (Self::Pattern(levels), Self::Vibrate(level)) => {
                levels.iter().all(|sample| sample == level)
            }
            _ => self == other,
        }
    }
}

#[derive(Clone, Copy)]
struct Ramp {
    from: f32,
    target: f32,
    started: Instant,
}

impl Ramp {
    fn value(self, now: Instant) -> f32 {
        let progress = (now.saturating_duration_since(self.started).as_secs_f32()
            / RAMP.as_secs_f32())
        .min(1.0);
        self.from + (self.target - self.from) * progress
    }
}

struct Playing {
    output: Output,
    ideals: [f32; SAMPLES],
    error: f32,
    started: Instant,
}

impl Playing {
    /// Account only for slots (or parts of slots) actually played. Counting a
    /// whole future pattern would bias dithering when new targets replace it.
    fn error_at(&self, now: Instant) -> f32 {
        if !matches!(self.output, Output::Pattern(_)) {
            return 0.5;
        }
        let elapsed = now.saturating_duration_since(self.started).min(LEASE);
        let mut error = self.error;
        for (index, ideal) in self.ideals.iter().enumerate() {
            let start = INTERVAL * index as u32;
            let played = elapsed.saturating_sub(start).min(INTERVAL);
            if played.is_zero() {
                break;
            }
            error += (ideal - f32::from(self.output.level(start)))
                * (played.as_secs_f32() / INTERVAL.as_secs_f32());
        }
        error.clamp(0.0, 1.0)
    }
}

#[derive(Default)]
pub(crate) struct Smoother {
    ramp: Option<Ramp>,
    playing: Option<Playing>,
    mode: Option<LovenseOutputMode>,
}

impl Smoother {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Core intensities stay normalized. Fractional 0..20 values exist only here.
    /// Every integer slot honors the FLOOR of the independent physical ceiling.
    pub fn prepare(
        &mut self,
        intensity: f32,
        ceiling: f32,
        mode: LovenseOutputMode,
        now: Instant,
    ) -> Option<Output> {
        if self
            .playing
            .as_ref()
            .is_some_and(|playing| now.saturating_duration_since(playing.started) >= LEASE)
        {
            self.reset();
        }
        let maximum = (unit(ceiling) * 20.0).floor();
        let target = (unit(intensity) * 20.0).min(maximum);
        let changed = self.ramp.is_some_and(|ramp| ramp.target != target);
        if !changed
            && self.mode == Some(mode)
            && self.playing.as_ref().is_some_and(|playing| {
                playing.output.maximum() <= maximum as u8
                    && now.saturating_duration_since(playing.started) < RENEWAL
            })
        {
            return None;
        }
        self.mode = Some(mode);
        let gradual = mode != LovenseOutputMode::Vibrate
            && target > 0.0
            && changed
            && self
                .ramp
                .is_some_and(|ramp| (ramp.target - target).abs() < FAST_CHANGE);
        let ramp = if gradual {
            let previous = self.ramp.unwrap().value(now).min(maximum);
            Ramp {
                // Move immediately, then settle. Never queue a 110 ms HTTP wait.
                from: previous + (target - previous) * 0.5,
                target,
                started: now,
            }
        } else if changed || self.ramp.is_none() || target == 0.0 {
            // Startup, zero, and large changes (e.g. pulses) have no smoothing lag.
            Ramp {
                from: target,
                target,
                started: now,
            }
        } else {
            self.ramp.unwrap()
        };
        self.ramp = Some(ramp);
        let dither = mode == LovenseOutputMode::PatternDither;
        let fractional = dither && target != target.floor();
        let interpolating =
            ramp.from != target && now.saturating_duration_since(ramp.started) < RAMP;
        let pattern = mode != LovenseOutputMode::Vibrate
            && target > 0.0
            && (changed || fractional || interpolating);
        let mut ideals = [target; SAMPLES];
        let error = self
            .playing
            .as_ref()
            .map_or(0.5, |playing| playing.error_at(now));
        let output = if pattern {
            let mut remaining = error;
            let levels = std::array::from_fn(|index| {
                let ideal = ramp.value(now + INTERVAL * index as u32).min(maximum);
                ideals[index] = ideal;
                let level = if dither {
                    (ideal + remaining).floor().min(maximum)
                } else {
                    ideal.floor()
                };
                if dither {
                    remaining = (remaining + ideal - level).clamp(0.0, 1.0);
                }
                level as u8
            });
            Output::Pattern(levels)
        } else {
            Output::Vibrate(target.floor() as u8)
        };
        if self.playing.as_ref().is_some_and(|playing| {
            playing.output.equivalent(&output)
                && now.saturating_duration_since(playing.started) < RENEWAL
        }) {
            return None;
        }
        self.playing = Some(Playing {
            output: output.clone(),
            ideals,
            error,
            started: now,
        });
        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn average(output: &Output) -> f32 {
        let sum: f32 = (0..400)
            .map(|sample| f32::from(output.level(Duration::from_millis(sample * 5))))
            .sum();
        sum / 400.0
    }

    #[test]
    fn fractional_pattern_and_renewals_preserve_time_average() {
        let start = Instant::now();
        let mut smoother = Smoother::default();
        let output = smoother
            .prepare(10.5 / 20.0, 1.0, LovenseOutputMode::PatternDither, start)
            .unwrap();
        let Output::Pattern(levels) = &output else {
            panic!("fractional targets must use Pattern");
        };
        assert!(levels.iter().all(|level| *level == 10 || *level == 11));
        assert!((average(&output) - 10.5).abs() < 0.02);
        let mut sum = 0.0;
        let mut requests = 0;
        for milliseconds in (0..2_000).step_by(5) {
            let now = start + Duration::from_millis(milliseconds);
            if smoother
                .prepare(10.5 / 20.0, 1.0, LovenseOutputMode::PatternDither, now)
                .is_some()
            {
                requests += 1;
            }
            let playing = smoother.playing.as_ref().unwrap();
            sum += f32::from(playing.output.level(now - playing.started));
        }
        assert_eq!(
            requests, 3,
            "only the three 500 ms lease renewals are needed"
        );
        assert!((sum / 400.0 - 10.5).abs() < 0.05);
    }

    #[test]
    fn every_pattern_slot_obeys_the_ceiling_and_zero_is_immediate() {
        let start = Instant::now();
        let mut smoother = Smoother::default();
        smoother.prepare(0.525, 1.0, LovenseOutputMode::PatternDither, start);
        let output = smoother
            .prepare(
                0.525,
                0.53,
                LovenseOutputMode::PatternDither,
                start + Duration::from_millis(40),
            )
            .unwrap();
        assert_eq!(output.maximum(), 10);
        assert_eq!(average(&output), 10.0);
        let zero = smoother
            .prepare(
                0.0,
                1.0,
                LovenseOutputMode::PatternDither,
                start + Duration::from_millis(50),
            )
            .unwrap();
        assert_eq!(zero, Output::Vibrate(0));
        for invalid in [f32::NAN, f32::INFINITY, -1.0] {
            smoother.reset();
            assert_eq!(
                smoother.prepare(invalid, 1.0, LovenseOutputMode::PatternDither, start),
                Some(Output::Vibrate(0))
            );
        }
    }

    #[test]
    fn interpolate_small_changes_but_preserve_fast_pulse_onset() {
        let start = Instant::now();
        let mut smoother = Smoother::default();
        smoother.prepare(0.5, 1.0, LovenseOutputMode::PatternDither, start);
        smoother.prepare(
            0.55,
            1.0,
            LovenseOutputMode::PatternDither,
            start + Duration::from_millis(40),
        );
        let playing = smoother.playing.as_ref().unwrap();
        assert_eq!(playing.ideals[0], 10.5);
        assert_eq!(playing.ideals[1], 11.0);
        let output = smoother
            .prepare(
                0.8,
                1.0,
                LovenseOutputMode::PatternDither,
                start + Duration::from_millis(80),
            )
            .unwrap();
        assert_eq!(output.level(Duration::ZERO), 16);
        smoother.reset();
        assert_eq!(
            smoother.prepare(0.5, 1.0, LovenseOutputMode::PatternDither, start),
            Some(Output::Vibrate(10))
        );
    }

    #[test]
    fn legacy_mode_is_exact_and_stable_integer_targets_only_renew_leases() {
        let start = Instant::now();
        for mode in LovenseOutputMode::ALL {
            let mut smoother = Smoother::default();
            assert_eq!(
                smoother.prepare(0.5, 1.0, mode, start),
                Some(Output::Vibrate(10))
            );
            assert!(
                smoother
                    .prepare(0.5, 1.0, mode, start + Duration::from_millis(499))
                    .is_none()
            );
            assert_eq!(
                smoother.prepare(0.5, 1.0, mode, start + RENEWAL),
                Some(Output::Vibrate(10))
            );
        }
        let mut smoother = Smoother::default();
        assert_eq!(
            smoother.prepare(0.525, 1.0, LovenseOutputMode::Vibrate, start),
            Some(Output::Vibrate(10))
        );
        assert!(
            smoother
                .prepare(
                    0.549,
                    1.0,
                    LovenseOutputMode::Vibrate,
                    start + Duration::from_millis(40)
                )
                .is_none()
        );
        assert_eq!(
            smoother.prepare(
                0.6,
                1.0,
                LovenseOutputMode::Vibrate,
                start + Duration::from_millis(80)
            ),
            Some(Output::Vibrate(12))
        );
    }

    #[test]
    fn compare_direct_pattern_and_dithered_pattern_tracking() {
        let start = Instant::now();
        let mut absolute_errors = Vec::with_capacity(LovenseOutputMode::ALL.len());
        for mode in LovenseOutputMode::ALL {
            let mut smoother = Smoother::default();
            let mut commands = 0;
            let mut bias = 0.0;
            let mut absolute_error = 0.0;
            for milliseconds in (0..2_000).step_by(5) {
                let now = start + Duration::from_millis(milliseconds);
                let target = 10.0 + 3.0 * milliseconds as f32 / 2_000.0;
                if milliseconds % 40 == 0
                    && smoother.prepare(target / 20.0, 1.0, mode, now).is_some()
                {
                    commands += 1;
                }
                let playing = smoother.playing.as_ref().unwrap();
                let actual = f32::from(playing.output.level(now - playing.started));
                bias += actual - target;
                absolute_error += (actual - target).abs();
            }
            bias /= 400.0;
            absolute_error /= 400.0;
            absolute_errors.push(absolute_error);
            println!(
                "{mode:?}: 10→13 ramp / 2s, commands={commands}, mean error={bias:.3} levels, mean absolute error={absolute_error:.3} levels"
            );
            if mode == LovenseOutputMode::PatternDither {
                assert!(
                    bias.abs() < 0.2,
                    "fractional tracking must avoid the legacy half-level bias"
                );
            }
        }
        assert!(
            absolute_errors[0] < absolute_errors[2],
            "dithering must reduce average quantization error versus direct Vibrate"
        );
    }
}
