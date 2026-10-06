//! Bounded transient envelopes and priority-weighted soft mixing.

use std::time::{Duration, Instant};

use crate::unit;

#[derive(Clone, Copy, Debug)]
pub struct HapticEffect {
    pub intensity: f32,
    pub priority: u8,
    pub attack: Duration,
    pub hold: Duration,
    pub release: Duration,
}

impl HapticEffect {
    pub fn duration(&self) -> Duration {
        self.attack
            .saturating_add(self.hold)
            .saturating_add(self.release)
    }

    /// Linear attack/release; zero-length segments are valid and never divide by zero.
    pub fn sample(&self, elapsed: Duration) -> f32 {
        if elapsed >= self.duration() {
            return 0.0;
        }
        let gain = if elapsed < self.attack {
            elapsed.as_secs_f32() / self.attack.as_secs_f32()
        } else if elapsed < self.attack.saturating_add(self.hold) {
            1.0
        } else if !self.release.is_zero() {
            1.0 - elapsed
                .saturating_sub(self.attack.saturating_add(self.hold))
                .as_secs_f32()
                / self.release.as_secs_f32()
        } else {
            0.0
        };
        unit(self.intensity) * unit(gain)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ContinuousEffect {
    pub intensity: f32,
    pub priority: u8,
}

struct ActiveEffect {
    effect: HapticEffect,
    started: Instant,
}

/// Maximum transient count is fixed. Each priority layer soft-mixes its effects
/// with attenuated lower layers. The largest layer output wins, so a higher
/// priority pulse can add strength without reducing the existing background.
/// No sample-time allocation is needed and output cannot clip.
pub struct EffectMixer {
    active: Vec<ActiveEffect>,
}

impl Default for EffectMixer {
    fn default() -> Self {
        Self {
            active: Vec::with_capacity(Self::MAX_TRANSIENTS),
        }
    }
}

impl EffectMixer {
    pub const MAX_TRANSIENTS: usize = 16;

    pub fn push(&mut self, effect: HapticEffect, now: Instant) {
        self.prune(now);
        if self.active.len() == Self::MAX_TRANSIENTS {
            let weakest = self
                .active
                .iter()
                .enumerate()
                .min_by_key(|(_, active)| active.effect.priority)
                .map(|(index, _)| index);
            if let Some(index) = weakest {
                if self.active[index].effect.priority > effect.priority {
                    return;
                }
                self.active.remove(index);
            }
        }
        self.active.push(ActiveEffect {
            effect,
            started: now,
        });
    }

    pub fn clear(&mut self) {
        self.active.clear();
    }
    /// Effect kinds own distinct priorities; disabling one must preserve others.
    pub fn clear_priority(&mut self, priority: u8) {
        self.active
            .retain(|active| active.effect.priority != priority);
    }
    pub fn level_at_priority(&self, priority: u8, now: Instant) -> f32 {
        self.active
            .iter()
            .filter(|active| active.effect.priority == priority)
            .fold(0.0, |mixed, active| {
                let level = active
                    .effect
                    .sample(now.saturating_duration_since(active.started));
                mixed + (1.0 - mixed) * level
            })
    }
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    fn prune(&mut self, now: Instant) {
        self.active.retain(|active| {
            now.saturating_duration_since(active.started) < active.effect.duration()
        });
    }

    pub fn sample(&mut self, continuous: &[ContinuousEffect], now: Instant) -> f32 {
        self.prune(now);
        let samples = || {
            continuous
                .iter()
                .map(|effect| (unit(effect.intensity), effect.priority))
                .chain(self.active.iter().map(|active| {
                    (
                        active
                            .effect
                            .sample(now.saturating_duration_since(active.started)),
                        active.effect.priority,
                    )
                }))
        };
        samples()
            .filter(|(intensity, _)| *intensity > 0.0)
            .map(|(_, priority)| priority)
            .fold(0.0_f32, |mixed, layer| {
                let combined = samples().filter(|(_, priority)| *priority <= layer).fold(
                    0.0,
                    |combined, (intensity, priority)| {
                        let weight = 1.0 / (1.0 + f32::from(layer - priority) / 64.0);
                        // Equivalent to 1-product(1-intensity*weight), without
                        // subtracting a small single effect from 1 twice. That
                        // cancellation can lose a device step during flooring.
                        combined + (1.0 - combined) * intensity * weight
                    },
                );
                // Preserve lower layers throughout a pulse's attack/release,
                // including their combined strength rather than just one effect.
                mixed.max(unit(combined))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pulse(priority: u8) -> HapticEffect {
        HapticEffect {
            intensity: 0.8,
            priority,
            attack: Duration::from_millis(20),
            hold: Duration::from_millis(40),
            release: Duration::from_millis(40),
        }
    }

    #[test]
    fn envelope_attack_hold_release_and_expiry() {
        let effect = pulse(100);
        assert_eq!(effect.sample(Duration::ZERO), 0.0);
        assert!((effect.sample(Duration::from_millis(10)) - 0.4).abs() < 0.0001);
        assert_eq!(effect.sample(Duration::from_millis(30)), 0.8);
        assert!((effect.sample(Duration::from_millis(80)) - 0.4).abs() < 0.0001);
        assert_eq!(effect.sample(Duration::from_millis(100)), 0.0);
        let instant = HapticEffect {
            attack: Duration::ZERO,
            hold: Duration::ZERO,
            release: Duration::ZERO,
            ..effect
        };
        assert_eq!(instant.sample(Duration::ZERO), 0.0);
        let held = HapticEffect {
            attack: Duration::ZERO,
            release: Duration::ZERO,
            ..effect
        };
        assert_eq!(held.sample(Duration::ZERO), 0.8);
    }

    #[test]
    fn single_effect_preserves_exact_intensity_at_device_step_boundaries() {
        let now = Instant::now();
        let mut mixer = EffectMixer::default();
        for intensity in [0.01, 0.05, 0.1, 0.2, 0.4, 0.8, 1.0] {
            assert_eq!(
                mixer.sample(
                    &[ContinuousEffect {
                        intensity,
                        priority: 20,
                    }],
                    now,
                ),
                intensity
            );
        }
    }

    #[test]
    fn higher_priority_keeps_its_strength() {
        let now = Instant::now();
        let mut mixer = EffectMixer::default();
        let dominant = mixer.sample(
            &[
                ContinuousEffect {
                    intensity: 0.8,
                    priority: 160,
                },
                ContinuousEffect {
                    intensity: 0.4,
                    priority: 20,
                },
            ],
            now,
        );
        let reversed = mixer.sample(
            &[
                ContinuousEffect {
                    intensity: 0.8,
                    priority: 20,
                },
                ContinuousEffect {
                    intensity: 0.4,
                    priority: 160,
                },
            ],
            now,
        );
        assert!(dominant >= 0.8 && dominant > reversed);
        assert!(dominant <= 1.0);
        assert_eq!(
            mixer.sample(
                &[ContinuousEffect {
                    intensity: 2.0,
                    priority: 1
                }],
                now
            ),
            1.0
        );
    }

    #[test]
    fn a_new_priority_layer_cannot_reduce_existing_effects() {
        let now = Instant::now();
        let mut mixer = EffectMixer::default();
        let background = [
            ContinuousEffect {
                intensity: 0.6,
                priority: 20,
            },
            ContinuousEffect {
                intensity: 0.6,
                priority: 20,
            },
        ];
        let baseline = mixer.sample(&background, now);
        let with_pulse = [
            background[0],
            background[1],
            ContinuousEffect {
                intensity: 0.01,
                priority: 160,
            },
        ];
        assert!(mixer.sample(&with_pulse, now) >= baseline);
    }

    #[test]
    fn shift_envelope_preserves_engine_baseline_and_has_no_edge_dips() {
        let now = Instant::now();
        let mut mixer = EffectMixer::default();
        let engine = [ContinuousEffect {
            intensity: 0.65,
            priority: 20,
        }];
        let baseline = mixer.sample(&engine, now);
        mixer.push(pulse(160), now);
        let mut previous = baseline;
        for milliseconds in 0..=100 {
            let output = mixer.sample(&engine, now + Duration::from_millis(milliseconds));
            assert!(
                output >= baseline,
                "pulse reduced the engine at {milliseconds} ms"
            );
            assert!(output <= 1.0);
            if milliseconds <= 60 {
                assert!(output >= previous, "attack/hold must not drop");
            } else {
                assert!(output <= previous, "release must not increase");
            }
            previous = output;
        }
        assert_eq!(previous, baseline);
        assert_eq!(mixer.active_count(), 0);
    }

    #[test]
    fn saturated_mixer_rejects_lower_priority_and_accepts_higher_priority() {
        let now = Instant::now();
        let mut mixer = EffectMixer::default();
        for _ in 0..EffectMixer::MAX_TRANSIENTS {
            mixer.push(
                HapticEffect {
                    intensity: 0.025,
                    ..pulse(160)
                },
                now,
            );
        }
        let sample_at = now + Duration::from_millis(30);
        let baseline = mixer.sample(&[], sample_at);
        mixer.push(
            HapticEffect {
                intensity: 1.0,
                ..pulse(20)
            },
            now,
        );
        assert_eq!(mixer.sample(&[], sample_at), baseline);
        mixer.push(
            HapticEffect {
                intensity: 1.0,
                ..pulse(200)
            },
            now,
        );
        assert_eq!(mixer.sample(&[], sample_at), 1.0);
        assert_eq!(mixer.active_count(), EffectMixer::MAX_TRANSIENTS);
    }

    #[test]
    fn transients_are_bounded_and_removed_after_expiry() {
        let now = Instant::now();
        let mut mixer = EffectMixer::default();
        for _ in 0..100 {
            mixer.push(pulse(100), now);
        }
        assert_eq!(mixer.active_count(), EffectMixer::MAX_TRANSIENTS);
        assert!((0.0..=1.0).contains(&mixer.sample(&[], now + Duration::from_millis(30))));
        assert_eq!(mixer.sample(&[], now + Duration::from_secs(1)), 0.0);
        assert_eq!(mixer.active_count(), 0);
    }
}
