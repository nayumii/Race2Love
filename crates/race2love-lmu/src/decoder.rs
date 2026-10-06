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
            // Optional signals require separate verification in Phase 6.
            ..TelemetryFrame::default()
        },
        elapsed_seconds: elapsed,
        vehicle_id: i32_at(car, VEHICLE_ID),
        game_version: version,
    }))
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
