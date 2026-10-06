# Phase 1 validation record

Development environment: Linux x86-64, Rust stable 1.94.1, Cargo 1.94.1.
Recorded 2026-10-06. No LMU or physical haptic device was accessed.

## Completed checks

| Check | Result |
| --- | --- |
| `cargo fmt --all` | Passed |
| `cargo fmt --all --check` | Passed |
| `cargo check --workspace --all-targets --locked --offline` | Passed |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed |
| `cargo test --workspace --locked --offline` | 25 passed, 0 failed |
| `cargo build --locked --offline` | Passed; native Linux executable built |
| Display-free Demo, five seconds | Passed; normal shutdown output was 0.0 |
| Native Linux/X11 Demo window | Passed; live telemetry, Escape stop, normal closure |
| `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked --offline` | Passed |

These checks compiled the actual complete workspace, including eframe/egui.
Dependencies were fetched before the offline checks. The generated `Cargo.lock`
is included. All enabled dependencies' declared minimum Rust versions are at most
the workspace's Rust 1.92 minimum; execution was tested with Rust 1.94.1.

## Test coverage

The 24 core tests cover RPM thresholds/curves, invalid-value clamping, shift
detection, duplicate frames, envelope boundaries, priorities, bounded transient
count, global scaling/ceiling, telemetry age/future timestamps, TOML round trips,
example config/default agreement, file replacement/invalid-file preservation,
live settings, stop/resume, pause, reconnect, shutdown, deduplication,
communication-fault latching, slow-request cancellation, effects channel closure,
and stalled-heartbeat stopping while telemetry remains live.

The GUI interaction test renders the actual app through an egui context and
supplies pointer/key input. It checks Stop/Resume, live effect toggles, the debug
setting, saved/reloaded TOML, Demo pause, Escape, and exit/shutdown with zero
output. This test does not require a display server or physical hardware.

## Demo runs

The five-second display-free run used the normal executable and its actual
telemetry/effects/output workers. Its output included:

```text
Demo: connected=true, gear=2, rpm=7053, mixed=0.488, mock_output=0.240, commands=39
Race2Love shutdown complete output=0.0
```

The native Linux window displayed live speed, RPM, gear, connection indicators,
and output meters. Escape latched emergency stop and all output meters reached
zero. Normal window closure awaited the workers and logged final output `0.0`.
This launch used X11 under the development desktop; native Wayland was compiled
but was not exercised separately.

## Reproduce

From the repository root with Rust stable and the platform's display/build
dependencies installed:

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -- --demo-seconds 5
cargo run --locked -- --demo
```

The Windows compile check additionally used:

```sh
rustup target add x86_64-pc-windows-gnu
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked
```

## Remaining limits and next step

The Windows GNU check verifies target compilation; it does not establish a linked
MSVC executable or native Windows runtime behavior. Those checks, a separate
native Wayland launch, and measured CPU/latency results remain outstanding.
Physical LMU/Lovense adapters and their hardware behavior belong to later phases.

Phase 1's build and Demo gate passes. The next step is a local Lovense `GetToys`
parser and timeout-bounded client, with a fake HTTP server proving response and
failure handling before physical output is enabled. See the [Phase 2 acceptance
checks](ADAPTERS.md#phase-2-lovense-local-backend).
