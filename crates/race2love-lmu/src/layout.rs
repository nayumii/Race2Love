//! Native LMU x64 contract facts, independently cross-checked against the SDK
//! translations listed in docs/LMU.md. These are NOT legacy rF2 plugin offsets.
//! Payload records use pack(4); the outer allocation has four tail-padding bytes.

pub const PAYLOAD_SIZE: usize = 324_820;
pub const ALLOCATION_SIZE: usize = 324_824;
pub const MAX_VEHICLES: usize = 104;
pub const GAME_VERSION: usize = 64;
pub const SCORING: usize = 1_632;
pub const TELEMETRY: usize = 128_464;
pub const VEHICLES: usize = TELEMETRY + 4;
pub const VEHICLE_SIZE: usize = 1_888;

pub const VEHICLE_ID: usize = 0;
pub const ELAPSED: usize = 12;
pub const CAR: usize = 32;
pub const CAR_LEN: usize = 64;
pub const VELOCITY: usize = 184;
pub const GEAR: usize = 352;
pub const RPM: usize = 356;
pub const THROTTLE: usize = 388;
pub const BRAKE: usize = 396;
pub const MAX_RPM: usize = 532;

const _: () = assert!(VEHICLES + MAX_VEHICLES * VEHICLE_SIZE == PAYLOAD_SIZE);
