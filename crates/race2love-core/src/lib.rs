//! Simulator-independent telemetry to normalized haptics.
//!
//! Adapters own game memory and device protocols. This crate only handles SI
//! telemetry values, effect configuration, normalized intensity, and channels.

pub mod config;
pub mod devices;
pub mod effects;
pub mod mixer;
pub mod runtime;
pub mod telemetry;

/// Clamp normalized values, treating all non-finite inputs as zero.
pub fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Apply the global multiplier and the independent absolute output ceiling.
pub fn scale_output(mixed: f32, multiplier: f32, ceiling: f32) -> f32 {
    (unit(mixed) * unit(multiplier)).min(unit(ceiling))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamping_rejects_non_finite_values() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
            assert_eq!(unit(value), 0.0);
        }
        assert_eq!(unit(2.0), 1.0);
        assert_eq!(unit(0.3), 0.3);
    }

    #[test]
    fn global_scaling_obeys_absolute_ceiling() {
        assert_eq!(scale_output(0.8, 0.5, 1.0), 0.4);
        assert_eq!(scale_output(1.0, 1.0, 0.3), 0.3);
        assert_eq!(scale_output(1.0, 0.0, 1.0), 0.0);
        assert_eq!(scale_output(f32::NAN, 1.0, 1.0), 0.0);
    }
}
