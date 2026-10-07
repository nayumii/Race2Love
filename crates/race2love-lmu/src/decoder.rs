//! Bounds-checked, little-endian decoding into normalized telemetry. No pointers,
//! C++ structs or raw LMU records escape the adapter boundary.

use std::time::Instant;

use race2love_core::telemetry::TelemetryFrame;
use thiserror::Error;

use crate::layout::*;

#[derive(Debug, Error, PartialEq)]
pub enum DecodeError {
    #[error("LMU shared memory is too short ({0} bytes; need {PAYLOAD_SIZE})")]
    Truncated(usize),
    #[error("Unsupported LMU game version {0}; this reader supports the 1.2–1.4 layout")]
    UnsupportedVersion(i32),
    #[error("Invalid LMU field: {0}")]
    Invalid(&'static str),
}

pub struct DecodedFrame {
    pub frame: TelemetryFrame,
    /// Game time is the freshness witness; reading unchanged bytes is not a tick.
    pub elapsed_seconds: f64,
    pub vehicle_id: i32,
    pub game_version: i32,
    /// Adapter-private travel samples for a game-clock derivative in LmuSource.
    pub suspension_deflection: Option<[f32; 4]>,
}

/// `None` means no live player car (menus, non-realtime, loading or vehicle exit).
/// Callers must clear old output immediately on this result.
pub fn decode(bytes: &[u8], observed: Instant) -> Result<Option<DecodedFrame>, DecodeError> {
    if bytes.len() < PAYLOAD_SIZE {
        return Err(DecodeError::Truncated(bytes.len()));
    }
    let version = i32_at(bytes, GAME_VERSION);
    // gameVersion is a game marker, not an ABI guarantee. Reject unknown layout
    // families rather than attempting to interpret plausible-looking floats.
    if !(12_000..15_000).contains(&version) {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    let count = usize::from(bytes[TELEMETRY]);
    let player = usize::from(bytes[TELEMETRY + 1]);
    let has_player = boolean(bytes[TELEMETRY + 2], "playerHasVehicle")?;
    let realtime = boolean(bytes[SCORING + 115], "mInRealtime")?;
    if count > MAX_VEHICLES || player >= MAX_VEHICLES {
        return Err(DecodeError::Invalid("vehicle count/index"));
    }
    if !has_player || !realtime {
        return Ok(None);
    }
    if player >= count {
        return Err(DecodeError::Invalid("player index outside active vehicles"));
    }
    let car = &bytes[VEHICLES + player * VEHICLE_SIZE..][..VEHICLE_SIZE];
    let elapsed = number(car, ELAPSED, "mElapsedTime", 0.0, 1e9)?;
    let rpm = number(car, RPM, "mEngineRPM", 0.0, 100_000.0)?;
    let max_rpm = number(car, MAX_RPM, "mEngineMaxRPM", 0.0, 100_000.0)?;
    let gear = i32_at(car, GEAR);
    if !(-1..=32).contains(&gear) {
        return Err(DecodeError::Invalid("mGear"));
    }
    let velocity = [
        number(car, VELOCITY, "mLocalVel.x", -2_000.0, 2_000.0)?,
        number(car, VELOCITY + 8, "mLocalVel.y", -2_000.0, 2_000.0)?,
        number(car, VELOCITY + 16, "mLocalVel.z", -2_000.0, 2_000.0)?,
    ];
    let track = text(&bytes[SCORING..SCORING + 64]);
    let wheels: [&[u8]; 4] =
        std::array::from_fn(|index| &car[WHEELS + index * WHEEL_SIZE..][..WHEEL_SIZE]);
    let suspension_deflection =
        optional_four(|index| optional_number(wheels[index], SUSPENSION_DEFLECTION, -2.0, 2.0));
    let wheel_slip = optional_four(|index| wheel_slip_signal(wheels[index], velocity[2] as f32));
    let kerb_contact = wheels
        .iter()
        .all(|wheel| wheel[SURFACE_TYPE] <= 6)
        .then(|| {
            std::array::from_fn(|index| {
                wheels[index][SURFACE_TYPE] == 5
                    && optional_number(wheels[index], TIRE_LOAD, 0.0, 1e7)
                        .is_some_and(|load| load >= 50.0)
            })
        });
    let terrain: [String; 4] = std::array::from_fn(|index| {
        text(&wheels[index][TERRAIN_NAME..TERRAIN_NAME + 16]).unwrap_or_default()
    });
    let wheel_terrain = terrain
        .iter()
        .any(|name| !name.is_empty())
        .then_some(terrain);
    let acceleration: Option<[f32; 3]> = (|| {
        Some([
            optional_number(car, ACCELERATION, -5_000.0, 5_000.0)?,
            optional_number(car, ACCELERATION + 8, -5_000.0, 5_000.0)?,
            optional_number(car, ACCELERATION + 16, -5_000.0, 5_000.0)?,
        ])
    })();
    let impact_time = number(car, LAST_IMPACT_TIME, "mLastImpactET", 0.0, elapsed).ok();
    // SDK impact magnitude has no documented SI units. Severity is instead a
    // documented acceleration proxy (100 m/s² = full scale), gated by the game's
    // explicit recent impact timestamp. Normal braking/kerbs alone cannot fire it.
    let impact = impact_time.zip(acceleration).map(|(time, accel)| {
        if time > 0.0 && elapsed - time <= 0.25 {
            (accel.iter().map(|a| a * a).sum::<f32>().sqrt() / 100.0).clamp(0.0, 1.0)
        } else {
            0.0
        }
    });
    let session = match i32_at(bytes, SCORING + 64) {
        0 => "Test day",
        1..=4 => "Practice",
        5..=8 => "Qualifying",
        9 => "Warmup",
        10..=13 => "Race",
        _ => "Session",
    };
    Ok(Some(DecodedFrame {
        frame: TelemetryFrame {
            timestamp: observed,
            speed_mps: velocity.iter().map(|v| v * v).sum::<f64>().sqrt() as f32,
            engine_rpm: rpm as f32,
            engine_max_rpm: max_rpm as f32,
            gear: gear as i8,
            throttle: number(car, THROTTLE, "mUnfilteredThrottle", 0.0, 1.0)? as f32,
            brake: number(car, BRAKE, "mUnfilteredBrake", 0.0, 1.0)? as f32,
            car: text(&car[CAR..CAR + CAR_LEN]),
            session: Some(
                track.map_or_else(|| session.into(), |track| format!("{session} · {track}")),
            ),
            wheel_slip,
            vertical_acceleration: optional_number(car, ACCELERATION + 8, -5_000.0, 5_000.0),
            impact,
            impact_id: impact_time.filter(|time| *time > 0.0).map(f64::to_bits),
            kerb_contact,
            wheel_terrain,
            ..TelemetryFrame::default()
        },
        elapsed_seconds: elapsed,
        vehicle_id: i32_at(car, VEHICLE_ID),
        game_version: version,
        suspension_deflection,
    }))
}

/// Return a normalized sliding signal for one loaded wheel.
///
/// `mGripFract` is not populated for every LMU car. The SDK still publishes
/// patch/ground velocities on those cars, and wheel rotation plus the static
/// radius provides a second fallback for wheelspin/lockup when those velocity
/// fields are also unavailable. All three signals are estimates of contact
/// slip, rather than a vehicle-level slip angle.
fn wheel_slip_signal(wheel: &[u8], vehicle_longitudinal_speed: f32) -> Option<f32> {
    let load = optional_number(wheel, TIRE_LOAD, 0.0, 1e7)?;
    if load < 50.0 {
        return Some(0.0);
    }

    let sliding = optional_number(wheel, SLIDING_FRACTION, 0.0, 1.0).unwrap_or(0.0);
    let kinematic = match (
        optional_number(wheel, LATERAL_PATCH_VELOCITY, -2_000.0, 2_000.0),
        optional_number(wheel, LONGITUDINAL_PATCH_VELOCITY, -2_000.0, 2_000.0),
        optional_number(wheel, LATERAL_GROUND_VELOCITY, -2_000.0, 2_000.0),
        optional_number(wheel, LONGITUDINAL_GROUND_VELOCITY, -2_000.0, 2_000.0),
    ) {
        (Some(patch_lat), Some(patch_long), Some(ground_lat), Some(ground_long)) => {
            let relative = (patch_lat - ground_lat).hypot(patch_long - ground_long);
            let ground_speed = ground_lat.hypot(ground_long);
            if ground_speed >= 1.0 {
                (relative / ground_speed).clamp(0.0, 1.0)
            } else {
                0.0
            }
        }
        _ => 0.0,
    };

    // Some current LMU car packages zero all patch/ground velocity fields too.
    // Compare the wheel's circumferential speed with its ground speed so
    // throttle wheelspin and braking lockup still produce a useful signal.
    let rotational = match (
        optional_number(wheel, ROTATION, -2_000.0, 2_000.0),
        wheel.get(STATIC_UNDEFLECTED_RADIUS).copied(),
        optional_number(wheel, LONGITUDINAL_GROUND_VELOCITY, -2_000.0, 2_000.0),
    ) {
        (Some(rotation), Some(radius_cm), Some(ground_long)) if (10..=100).contains(&radius_cm) => {
            let wheel_speed = rotation.abs() * (f32::from(radius_cm) / 100.0);
            // A few car packages publish a zero wheel ground speed while the
            // car velocity remains valid. Use the vehicle longitudinal speed
            // in that case so wheelspin is not mistaken for missing motion.
            let ground_long = if ground_long.abs() < 1.0 && vehicle_longitudinal_speed.abs() >= 1.0
            {
                vehicle_longitudinal_speed
            } else {
                ground_long
            };
            let ground_speed = ground_long.abs();
            let denominator = wheel_speed.max(ground_speed).max(1.0);
            ((wheel_speed - ground_speed).abs() / denominator).clamp(0.0, 1.0)
        }
        _ => 0.0,
    };

    Some(sliding.max(kinematic).max(rotational))
}

fn optional_number(bytes: &[u8], offset: usize, min: f64, max: f64) -> Option<f32> {
    number(bytes, offset, "optional telemetry", min, max)
        .ok()
        .map(|value| value as f32)
}

fn optional_four(mut read: impl FnMut(usize) -> Option<f32>) -> Option<[f32; 4]> {
    Some([read(0)?, read(1)?, read(2)?, read(3)?])
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated fixed layout"),
    )
}

fn number(
    bytes: &[u8],
    offset: usize,
    field: &'static str,
    min: f64,
    max: f64,
) -> Result<f64, DecodeError> {
    let value = f64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("validated fixed layout"),
    );
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(DecodeError::Invalid(field))
    }
}

fn boolean(value: u8, field: &'static str) -> Result<bool, DecodeError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DecodeError::Invalid(field)),
    }
}

fn text(bytes: &[u8]) -> Option<String> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let value = String::from_utf8_lossy(&bytes[..end]);
    let value = value.trim_matches(|ch: char| ch.is_control() || ch.is_whitespace());
    (!value.is_empty()).then(|| value.to_owned())
}
