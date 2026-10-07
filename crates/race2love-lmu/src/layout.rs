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
pub const ACCELERATION: usize = 208;
pub const LAST_IMPACT_TIME: usize = 552;
pub const WHEELS: usize = 848;
pub const WHEEL_SIZE: usize = 260;
pub const SUSPENSION_DEFLECTION: usize = 0;
pub const LATERAL_PATCH_VELOCITY: usize = 48;
pub const LONGITUDINAL_PATCH_VELOCITY: usize = 56;
pub const LATERAL_GROUND_VELOCITY: usize = 64;
pub const LONGITUDINAL_GROUND_VELOCITY: usize = 72;
pub const ROTATION: usize = 40;
pub const TIRE_LOAD: usize = 104;
pub const SLIDING_FRACTION: usize = 112;
pub const TERRAIN_NAME: usize = 160;
pub const SURFACE_TYPE: usize = 176;
pub const STATIC_UNDEFLECTED_RADIUS: usize = 179;

const _: () = assert!(WHEELS + 4 * WHEEL_SIZE == VEHICLE_SIZE);

const _: () = assert!(VEHICLES + MAX_VEHICLES * VEHICLE_SIZE == PAYLOAD_SIZE);
