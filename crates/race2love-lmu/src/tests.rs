use super::*;
use crate::{
    decoder::{DecodeError, decode},
    layout::*,
};
use race2love_core::{
    config::Config,
    devices::MockDevice,
    runtime::{RaceRuntime, StopReason},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Entirely synthetic contract fixture. No game bytes/header/code redistributed.
pub(crate) fn fixture(player: usize, elapsed: f64) -> Vec<u8> {
    let mut data = vec![0; ALLOCATION_SIZE];
    data[GAME_VERSION..GAME_VERSION + 4].copy_from_slice(&14_000_i32.to_le_bytes());
    data[TELEMETRY] = (player + 1) as u8;
    data[TELEMETRY + 1] = player as u8;
    data[TELEMETRY + 2] = 1;
    data[SCORING + 115] = 1;
    data[SCORING + 64..SCORING + 68].copy_from_slice(&10_i32.to_le_bytes());
    data[SCORING..SCORING + 5].copy_from_slice(b"Spa\0\0");
    let base = VEHICLES + player * VEHICLE_SIZE;
    data[base..base + 4].copy_from_slice(&(player as i32).to_le_bytes());
    data[base + CAR..base + CAR + 7].copy_from_slice(b"GT3 car");
    data[base + GEAR..base + GEAR + 4].copy_from_slice(&4_i32.to_le_bytes());
    for (offset, value) in [
        (ELAPSED, elapsed),
        (RPM, 7_000.0),
        (MAX_RPM, 8_000.0),
        (THROTTLE, 0.75),
        (BRAKE, 0.125),
        (VELOCITY, 3.0),
        (VELOCITY + 8, 4.0),
        (VELOCITY + 16, -12.0),
    ] {
        put_float(&mut data, base + offset, value);
    }
    data
}

fn put_float(data: &mut [u8], offset: usize, value: f64) {
    data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn player_selection_offsets_units_and_optional_availability() {
    for player in [0, 5, 103] {
        let now = Instant::now();
        let result = decode(&fixture(player, 123.5), now).unwrap().unwrap();
        assert_eq!(result.vehicle_id, player as i32);
        assert_eq!(result.elapsed_seconds, 123.5);
        let frame = result.frame;
        assert_eq!(frame.timestamp, now);
        assert_eq!(frame.speed_mps, 13.0);
        assert_eq!(
            (frame.engine_rpm, frame.engine_max_rpm, frame.gear),
            (7_000.0, 8_000.0, 4)
        );
        assert_eq!((frame.throttle, frame.brake), (0.75, 0.125));
        assert_eq!(frame.car.as_deref(), Some("GT3 car"));
        assert_eq!(frame.session.as_deref(), Some("Race · Spa"));
        assert_eq!(frame.wheel_slip, Some([0.0; 4]));
        assert!(frame.suspension_velocity.is_none()); // Requires two game-clock samples.
        assert_eq!(frame.vertical_acceleration, Some(0.0));
        assert_eq!(frame.impact, Some(0.0));
    }
}

#[test]
fn lmu_gear_shift_through_neutral_reaches_the_effect_engine() {
    let start = Instant::now();
    let mut engine = race2love_core::effects::EffectEngine::default();
    let mut config = Config::default();
    config.effects.engine.enabled = false;
    for (ms, gear) in [(0, 3_i32), (20, 0), (40, 4), (60, 4)] {
        let mut bytes = fixture(0, 10.0 + ms as f64 / 1000.0);
        bytes[VEHICLES + GEAR..VEHICLES + GEAR + 4].copy_from_slice(&gear.to_le_bytes());
        let now = start + Duration::from_millis(ms);
        let frame = decode(&bytes, now).unwrap().unwrap().frame;
        let output = engine.update(&frame, &config.effects, now);
        if ms == 60 {
            assert!(output > 0.8);
            assert_eq!(engine.levels().shift_count, 1);
        }
    }
}

#[test]
fn optional_wheel_and_event_fields_have_verified_units_and_fail_independently() {
    let mut bytes = fixture(5, 123.5);
    let base = VEHICLES + 5 * VEHICLE_SIZE;
    for index in 0..4 {
        let wheel = base + WHEELS + index * WHEEL_SIZE;
        put_float(&mut bytes, wheel + SLIDING_FRACTION, 0.1 * index as f64);
        put_float(&mut bytes, wheel + TIRE_LOAD, 1_000.0);
        put_float(
            &mut bytes,
            wheel + SUSPENSION_DEFLECTION,
            0.02 * index as f64,
        );
        bytes[wheel + SURFACE_TYPE] = if index == 2 { 5 } else { 0 };
    }
    put_float(&mut bytes, base + ACCELERATION + 8, 30.0);
    put_float(&mut bytes, base + LAST_IMPACT_TIME, 123.4);
    let decoded = decode(&bytes, Instant::now()).unwrap().unwrap();
    assert_eq!(decoded.frame.wheel_slip, Some([0.0, 0.1, 0.2, 0.3]));
    assert_eq!(decoded.suspension_deflection, Some([0.0, 0.02, 0.04, 0.06]));
    assert_eq!(
        decoded.frame.kerb_contact,
        Some([false, false, true, false])
    );
    assert_eq!(decoded.frame.vertical_acceleration, Some(30.0));
    assert_eq!(decoded.frame.impact, Some(0.3));
    assert_eq!(decoded.frame.impact_id, Some(123.4_f64.to_bits()));
    put_float(&mut bytes, base + WHEELS + SLIDING_FRACTION, f64::NAN);
    put_float(&mut bytes, base + ACCELERATION + 8, f64::INFINITY);
    let decoded = decode(&bytes, Instant::now()).unwrap().unwrap();
    assert_eq!(decoded.frame.wheel_slip, Some([0.0, 0.1, 0.2, 0.3]));
    assert!(decoded.frame.vertical_acceleration.is_none() && decoded.frame.impact.is_none());
    assert_eq!(decoded.frame.gear, 4); // Malformed optional fields do not break RPM/gear.
    put_float(&mut bytes, base + ACCELERATION + 8, 30.0);
    put_float(&mut bytes, base + LAST_IMPACT_TIME, 100.0);
    assert_eq!(
        decode(&bytes, Instant::now())
            .unwrap()
            .unwrap()
            .frame
            .impact,
        Some(0.0)
    );
}

#[test]
fn wheel_slip_uses_velocity_difference_when_grip_fraction_is_zero() {
    let mut bytes = fixture(0, 123.5);
    let wheel = VEHICLES + WHEELS;
    put_float(&mut bytes, wheel + TIRE_LOAD, 1_000.0);
    put_float(&mut bytes, wheel + LATERAL_PATCH_VELOCITY, 0.0);
    put_float(&mut bytes, wheel + LONGITUDINAL_PATCH_VELOCITY, 6.0);
    put_float(&mut bytes, wheel + LATERAL_GROUND_VELOCITY, 0.0);
    put_float(&mut bytes, wheel + LONGITUDINAL_GROUND_VELOCITY, 3.0);
    put_float(&mut bytes, wheel + SLIDING_FRACTION, 0.0);
    let slip = decode(&bytes, Instant::now())
        .unwrap()
        .unwrap()
        .frame
        .wheel_slip
        .unwrap()[0];
    assert!((slip - 1.0).abs() < f32::EPSILON);
}

#[test]
fn wheel_slip_falls_back_to_wheel_rotation_when_patch_velocities_are_missing() {
    let mut bytes = fixture(0, 123.5);
    let wheel = VEHICLES + WHEELS;
    put_float(&mut bytes, wheel + TIRE_LOAD, 1_000.0);
    put_float(&mut bytes, wheel + ROTATION, 100.0);
    bytes[wheel + STATIC_UNDEFLECTED_RADIUS] = 33;
    put_float(&mut bytes, wheel + LONGITUDINAL_GROUND_VELOCITY, 20.0);
    put_float(&mut bytes, wheel + LATERAL_PATCH_VELOCITY, f64::NAN);
    put_float(&mut bytes, wheel + LONGITUDINAL_PATCH_VELOCITY, f64::NAN);
    put_float(&mut bytes, wheel + LATERAL_GROUND_VELOCITY, f64::NAN);
    put_float(&mut bytes, wheel + SLIDING_FRACTION, 0.0);

    let slip = decode(&bytes, Instant::now())
        .unwrap()
        .unwrap()
        .frame
        .wheel_slip
        .unwrap()[0];
    assert!((slip - 0.393_939_4).abs() < 0.001);
}

#[test]
fn malformed_layouts_and_numbers_fail_closed() {
    let now = Instant::now();
    let data = fixture(0, 1.0);
    for len in [0, 64, TELEMETRY + 3, PAYLOAD_SIZE - 1] {
        assert!(matches!(
            decode(&data[..len], now),
            Err(DecodeError::Truncated(_))
        ));
    }
    for version in [0_i32, 11_000, 15_000, i32::MAX] {
        let mut bytes = data.clone();
        bytes[GAME_VERSION..GAME_VERSION + 4].copy_from_slice(&version.to_le_bytes());
        assert!(matches!(
            decode(&bytes, now),
            Err(DecodeError::UnsupportedVersion(_))
        ));
    }
    for (offset, value) in [
        (RPM, f64::NAN),
        (MAX_RPM, f64::INFINITY),
        (ELAPSED, -1.0),
        (BRAKE, 2.0),
        (VELOCITY, f64::NEG_INFINITY),
    ] {
        let mut bytes = data.clone();
        put_float(&mut bytes, VEHICLES + offset, value);
        assert!(matches!(decode(&bytes, now), Err(DecodeError::Invalid(_))));
    }
    for (offset, value) in [
        (TELEMETRY, 105),
        (TELEMETRY + 1, 104),
        (TELEMETRY + 1, 1),
        (TELEMETRY + 2, 2),
        (SCORING + 115, 2),
    ] {
        let mut bytes = data.clone();
        bytes[offset] = value;
        assert!(matches!(decode(&bytes, now), Err(DecodeError::Invalid(_))));
    }
    let mut bytes = data;
    bytes[VEHICLES + GEAR..VEHICLES + GEAR + 4].copy_from_slice(&128_i32.to_le_bytes());
    assert!(matches!(
        decode(&bytes, now),
        Err(DecodeError::Invalid("mGear"))
    ));
}

#[test]
fn no_player_and_non_realtime_never_produce_effect_frames() {
    let mut bytes = fixture(0, 1.0);
    bytes[TELEMETRY + 2] = 0;
    assert!(decode(&bytes, Instant::now()).unwrap().is_none());
    bytes[TELEMETRY + 2] = 1;
    bytes[SCORING + 115] = 0;
    assert!(decode(&bytes, Instant::now()).unwrap().is_none());
}

#[test]
fn reverse_neutral_zero_rpm_and_lossy_strings_are_valid() {
    for gear in [-1_i32, 0] {
        let mut bytes = fixture(0, 1.0);
        bytes[VEHICLES + GEAR..VEHICLES + GEAR + 4].copy_from_slice(&gear.to_le_bytes());
        put_float(&mut bytes, VEHICLES + RPM, 0.0);
        put_float(&mut bytes, VEHICLES + MAX_RPM, 0.0);
        bytes[VEHICLES + CAR] = 255;
        let frame = decode(&bytes, Instant::now()).unwrap().unwrap().frame;
        assert_eq!(frame.gear, gear as i8);
        assert_eq!(frame.engine_max_rpm, 0.0);
        assert!(frame.car.unwrap().starts_with('�'));
    }
}

#[derive(Clone, Default)]
struct Producer {
    alive: Arc<AtomicBool>,
    advancing: Arc<AtomicBool>,
    tick: Arc<AtomicU64>,
    player: Arc<AtomicBool>,
}

struct FixtureReader {
    producer: Producer,
    connected: bool,
}
impl SnapshotReader for FixtureReader {
    fn connect(&mut self) -> Result<(), TelemetryError> {
        self.connected = self.producer.alive.load(Ordering::SeqCst);
        if self.connected {
            Ok(())
        } else {
            Err(TelemetryError::Disconnected)
        }
    }
    fn disconnect(&mut self) {
        self.connected = false;
    }
    fn is_connected(&self) -> bool {
        self.connected
    }
    fn snapshot(&mut self, destination: &mut [u8]) -> Result<bool, TelemetryError> {
        if !self.producer.alive.load(Ordering::SeqCst) {
            return Err(TelemetryError::Disconnected);
        }
        if self.producer.advancing.load(Ordering::SeqCst) {
            self.producer.tick.fetch_add(1, Ordering::SeqCst);
        }
        let mut data = fixture(0, self.producer.tick.load(Ordering::SeqCst) as f64 / 60.0);
        data[TELEMETRY + 2] = u8::from(self.producer.player.load(Ordering::SeqCst));
        destination.copy_from_slice(&data[..PAYLOAD_SIZE]);
        Ok(true)
    }
}

fn source(producer: Producer) -> LmuSource<FixtureReader> {
    LmuSource::new(FixtureReader {
        producer,
        connected: false,
    })
}

fn producer() -> Producer {
    let producer = Producer::default();
    producer.alive.store(true, Ordering::SeqCst);
    producer.advancing.store(true, Ordering::SeqCst);
    producer.player.store(true, Ordering::SeqCst);
    producer
}

#[test]
fn progress_is_required_after_open_and_duplicates_do_not_refresh() {
    let producer = producer();
    let mut source = source(producer.clone());
    source.connect().unwrap();
    assert!(source.read_frame().unwrap().is_none());
    let frame = source.read_frame().unwrap().unwrap();
    producer.advancing.store(false, Ordering::SeqCst);
    assert!(source.read_frame().unwrap().is_none());
    assert!(
        source
            .read_at(frame.timestamp + Duration::from_secs(2))
            .is_err()
    );
    source.connect().unwrap();
    assert!(source.read_frame().unwrap().is_none());
    assert!(source.read_frame().unwrap().is_none());
    producer.advancing.store(true, Ordering::SeqCst);
    assert!(source.read_frame().unwrap().is_some());
    producer.player.store(false, Ordering::SeqCst);
    assert!(source.read_frame().is_err());
    assert!(source.is_connected());
}

async fn wait_for(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("fixture pipeline did not reach expected state");
}

#[tokio::test]
async fn lmu_pipeline_stops_on_freeze_player_exit_and_game_restart() {
    let producer = producer();
    let device = Arc::new(MockDevice::default());
    let runtime = RaceRuntime::spawn(
        Box::new(source(producer.clone())),
        device.clone(),
        Config::default(),
    )
    .unwrap();
    wait_for(|| device.intensity() > 0.0).await;
    producer.advancing.store(false, Ordering::SeqCst);
    wait_for(|| {
        runtime.control.snapshot().effects.reason == StopReason::StaleTelemetry
            && device.intensity() == 0.0
    })
    .await;
    producer.advancing.store(true, Ordering::SeqCst);
    wait_for(|| device.intensity() > 0.0).await;
    producer.player.store(false, Ordering::SeqCst);
    wait_for(|| runtime.control.snapshot().telemetry.frame.is_none() && device.intensity() == 0.0)
        .await;
    producer.player.store(true, Ordering::SeqCst);
    wait_for(|| device.intensity() > 0.0).await;
    producer.alive.store(false, Ordering::SeqCst);
    wait_for(|| !runtime.control.snapshot().telemetry.connected && device.intensity() == 0.0).await;
    producer.alive.store(true, Ordering::SeqCst);
    wait_for(|| device.intensity() > 0.0).await;
    runtime.shutdown().await;
    assert_eq!(device.intensity(), 0.0);
}

#[test]
fn suspension_derivative_uses_game_time_and_resets_across_discontinuities() {
    struct BytesReader(Vec<u8>);
    impl SnapshotReader for BytesReader {
        fn connect(&mut self) -> Result<(), TelemetryError> {
            Ok(())
        }
        fn disconnect(&mut self) {}
        fn is_connected(&self) -> bool {
            true
        }
        fn snapshot(&mut self, destination: &mut [u8]) -> Result<bool, TelemetryError> {
            destination.copy_from_slice(&self.0[..PAYLOAD_SIZE]);
            Ok(true)
        }
    }
    let start = Instant::now();
    let mut source = LmuSource::new(BytesReader(fixture(0, 10.0)));
    source.connect().unwrap();
    assert!(source.read_at(start).unwrap().is_none());
    put_float(&mut source.reader.0, VEHICLES + ELAPSED, 10.02);
    put_float(
        &mut source.reader.0,
        VEHICLES + WHEELS + SUSPENSION_DEFLECTION,
        0.01,
    );
    let frame = source
        .read_at(start + Duration::from_millis(40))
        .unwrap()
        .unwrap();
    assert!((frame.suspension_velocity.unwrap()[0] - 0.5).abs() < 0.001);
    assert!(
        source
            .read_at(start + Duration::from_millis(50))
            .unwrap()
            .is_none()
    );
    put_float(&mut source.reader.0, VEHICLES + ELAPSED, 11.0);
    assert!(
        source
            .read_at(start + Duration::from_millis(60))
            .unwrap()
            .unwrap()
            .suspension_velocity
            .is_none()
    );
    put_float(&mut source.reader.0, VEHICLES + ELAPSED, 1.0);
    assert!(matches!(
        source.read_at(start + Duration::from_millis(80)),
        Err(TelemetryError::Waiting(_))
    ));
    put_float(&mut source.reader.0, VEHICLES + ELAPSED, 1.02);
    assert_eq!(
        source
            .read_at(start + Duration::from_millis(100))
            .unwrap()
            .unwrap()
            .suspension_velocity,
        Some([0.0; 4])
    );
    put_float(&mut source.reader.0, VEHICLES + ELAPSED, 1.04);
    put_float(&mut source.reader.0, VEHICLES + WHEELS, f64::NAN);
    assert!(
        source
            .read_at(start + Duration::from_millis(120))
            .unwrap()
            .unwrap()
            .suspension_velocity
            .is_none()
    );
    put_float(&mut source.reader.0, VEHICLES + ELAPSED, 1.06);
    put_float(&mut source.reader.0, VEHICLES + WHEELS, 0.0);
    assert!(
        source
            .read_at(start + Duration::from_millis(140))
            .unwrap()
            .unwrap()
            .suspension_velocity
            .is_none()
    );
}

#[test]
fn terrain_names_are_diagnostic_and_unloaded_tyres_do_not_report_contact() {
    let mut bytes = fixture(0, 1.0);
    let wheel = VEHICLES + WHEELS;
    bytes[wheel + TERRAIN_NAME..wheel + TERRAIN_NAME + 5].copy_from_slice(b"KERB\0");
    let decode_frame = |bytes: &[u8]| decode(bytes, Instant::now()).unwrap().unwrap().frame;
    let frame = decode_frame(&bytes);
    assert_eq!(frame.wheel_terrain.unwrap()[0], "KERB");
    assert_eq!(frame.kerb_contact, Some([false; 4])); // Never guess contact from a name.
    bytes[wheel + SURFACE_TYPE] = 5;
    assert_eq!(decode_frame(&bytes).kerb_contact, Some([false; 4]));
    put_float(&mut bytes, wheel + TIRE_LOAD, 500.0);
    assert_eq!(
        decode_frame(&bytes).kerb_contact,
        Some([true, false, false, false])
    );
}
