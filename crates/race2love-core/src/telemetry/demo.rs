//! Deterministic driving simulation. Every signal is synthetic and labeled Demo.

use std::time::Instant;

use super::{TelemetryError, TelemetryFrame, TelemetrySource};

#[derive(Default)]
pub struct DemoSource {
    started: Option<Instant>,
}

impl DemoSource {
    pub fn frame_at(seconds: f32, timestamp: Instant) -> TelemetryFrame {
        let time = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        };
        let lap = time % 24.0;
        let accelerating = lap < 18.0;
        let shift_phase = (lap % 3.0) / 3.0;
        let rpm_ratio = if accelerating {
            0.42 + 0.55 * shift_phase
        } else {
            0.85 - 0.65 * (lap - 18.0) / 6.0
        };
        let gear = if accelerating {
            ((lap / 3.0) as i8 + 1).min(6)
        } else {
            (6 - ((lap - 18.0) / 1.2) as i8).max(1)
        };
        let kerb = (lap % 7.0) > 6.5;
        let slip = (lap % 9.0) > 7.6;
        let impact = (22.0..22.12).contains(&lap);
        let vertical = if impact {
            30.0
        } else if kerb {
            (time * 45.0).sin() * 9.0
        } else {
            (time * 3.0).sin() * 0.25
        };
        TelemetryFrame {
            timestamp,
            speed_mps: if accelerating {
                15.0 + lap * 3.0
            } else {
                69.0 - (lap - 18.0) * 9.0
            },
            engine_rpm: rpm_ratio * 9_000.0,
            engine_max_rpm: 9_000.0,
            gear,
            throttle: if accelerating { 0.85 } else { 0.05 },
            brake: if accelerating { 0.0 } else { 0.75 },
            wheel_slip: Some(if slip {
                [0.08, 0.1, 0.45, 0.5]
            } else {
                [0.01; 4]
            }),
            suspension_velocity: Some(if kerb {
                [vertical * 0.08, 0.1, vertical * 0.05, 0.05]
            } else {
                [0.02; 4]
            }),
            vertical_acceleration: Some(vertical),
            impact: Some(if impact { 0.85 } else { 0.0 }),
            impact_id: impact.then_some((time / 24.0) as u64 + 1),
            kerb_contact: Some([kerb, false, kerb, false]),
            wheel_terrain: None,
            session: Some("Demo • simulated practice".into()),
            car: Some("Demo prototype".into()),
        }
    }
}

impl TelemetrySource for DemoSource {
    fn name(&self) -> &'static str {
        "Demo"
    }
    fn connect(&mut self) -> Result<(), TelemetryError> {
        self.started = Some(Instant::now());
        Ok(())
    }
    fn disconnect(&mut self) {
        self.started = None;
    }
    fn is_connected(&self) -> bool {
        self.started.is_some()
    }
    fn read_frame(&mut self) -> Result<Option<TelemetryFrame>, TelemetryError> {
        let started = self.started.ok_or(TelemetryError::Disconnected)?;
        let now = Instant::now();
        Ok(Some(Self::frame_at(
            now.duration_since(started).as_secs_f32(),
            now,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_covers_shifts_kerbs_slip_and_impacts() {
        let now = Instant::now();
        let first = DemoSource::frame_at(0.0, now);
        let later = DemoSource::frame_at(3.2, now);
        assert_ne!(first.gear, later.gear);
        assert_ne!(first.engine_rpm, later.engine_rpm);
        assert!(
            DemoSource::frame_at(6.8, now)
                .vertical_acceleration
                .unwrap()
                .abs()
                > 0.25
        );
        assert!(DemoSource::frame_at(8.0, now).wheel_slip.unwrap()[3] > 0.4);
        assert!(DemoSource::frame_at(22.05, now).impact.unwrap() > 0.8);
        assert!(DemoSource::frame_at(20.0, now).brake > 0.0);
    }

    #[test]
    fn disconnected_demo_produces_no_frames() {
        let mut source = DemoSource::default();
        assert!(source.read_frame().is_err());
        source.connect().unwrap();
        assert!(source.read_frame().unwrap().is_some());
        source.disconnect();
        assert!(!source.is_connected());
        assert!(source.read_frame().is_err());
    }
}
