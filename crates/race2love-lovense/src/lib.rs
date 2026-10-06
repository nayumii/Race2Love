//! Direct local Lovense Remote adapter. One worker owns all HTTP requests and
//! serializes selection/output and lets Stop preempt discovery without blocking rendering.

mod protocol;
mod smoothing;
mod worker;

pub use protocol::{LEASE, RENEWAL, RemoteClient, Toy, parse_toys, vibration_step};
pub use worker::{ConnectionState, LovenseControl, LovenseDevice, LovenseService, LovenseSnapshot};
