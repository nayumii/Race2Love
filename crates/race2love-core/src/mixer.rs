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

/// Maximum transient count is fixed. Lower-priority effects are attenuated by
/// 1/(1 + priority_distance/64), then combined as 1-product(1-intensity).
/// The highest active priority keeps full strength and output cannot clip.
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
        let highest = samples()
            .filter(|(intensity, _)| *intensity > 0.0)
            .map(|(_, priority)| priority)
            .max()
            .unwrap_or(0);
        let remaining = samples().fold(1.0, |remaining, (intensity, priority)| {
            let weight = 1.0 / (1.0 + f32::from(highest.saturating_sub(priority)) / 64.0);
            remaining * (1.0 - intensity * weight)
        });
        unit(1.0 - remaining)
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
