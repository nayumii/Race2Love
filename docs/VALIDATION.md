# Phase 2 validation record

Development environment: Linux x86-64, Rust stable 1.94.1, Cargo 1.94.1.
Recorded 2026-10-06. No LMU or physical haptic device was accessed. Protocol
integration tests use a loopback-only fake Remote.

## Completed checks

| Check | Result |
| --- | --- |
| `cargo fmt --all` and `cargo fmt --all --check` | Passed |
| `cargo check --workspace --all-targets --locked --offline` | Passed |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed |
| `cargo test --workspace --locked --offline` | 40 passed, 0 failed |
| `cargo build --locked --offline` | Passed; native Linux executable built |
| Display-free Demo, five seconds | Passed; final mock output 0.0 |
| Native Linux/X11 window | Passed; live Dashboard, device settings, Escape input, normal closure |
| Windows GNU all-targets check using Clang, below | Passed |

Checks compile the actual complete workspace, including eframe/egui and the
Lovense adapter. Dependencies were fetched before the offline checks.
`Cargo.lock` is included. Enabled dependencies' declared minimum Rust versions
are at most the workspace's Rust 1.92 minimum; execution used Rust 1.94.1.

## Test coverage

- 25 core tests: curves/clamps, shift detection, envelopes, mixer priorities and
  bounds, scaling, telemetry timeout, TOML/file handling, live settings, worker
  failure/heartbeat safety, cancellation, backend switching, stop and shutdown.
- Two GUI interaction tests: actual egui pointer/key input checks effect edits,
  Stop/Resume/Escape, pause, persistence, missing-port reporting, Connect and
  Disconnect returning safely to Demo. No display server is required.
- One reconnect backoff unit test: five fixed delays, then no further retry timer.
- 12 fake Remote integration tests: typed string/object toy maps, status and
  capability availability, exact targeted requests and headers, malformed/HTTP
  errors, response limits, request timeout, no transport retries, finite command
  expiry, explicit selection, toy switching, duplicate renewal, manual-test
  ceiling/deadline, emergency cancellation during slow commands/discovery, safe
  reconnect, toy loss/return, persistent Stop failures, and shutdown.

The fake Remote simulates command expiry without receiving Stop. This verifies
Race2Love's finite request contract, not actual vendor firmware behavior. Hardware
verification remains outstanding.

## Demo runs

The five-second display-free run uses the normal executable and actual workers:

```text
Demo: connected=true, gear=2, rpm=7053, mixed=0.488, mock_output=0.240, commands=39
Lovense worker stopped
Race2Love shutdown complete output=0.0
```

The native Linux window displayed live telemetry and the Phase 2 device settings.
Escape was sent to the owned test window. Normal closure awaited both the core
workers and the idle Lovense worker and logged final output 0.0. The window used
X11; native Wayland was compiled but not exercised separately.

## Reproduce

With Rust stable and the platform's build/display dependencies installed:

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -- --demo-seconds 5
cargo run --locked -- --demo
```

Loopback tests need permission to bind a local TCP socket. They never scan the
LAN or contact real devices. Normal Windows builds use the MSVC compiler from
Visual Studio Build Tools; Linux builds need a C compiler for ring's TLS crypto.

The first Windows GNU check stopped in ring's build script because the host lacks
`x86_64-w64-mingw32-gcc`. The complete check then passed with installed Clang and
LLVM ar, using ring's freestanding C path and release-style C assertions:

```sh
rustup target add x86_64-pc-windows-gnu
CC_x86_64_pc_windows_gnu=clang \
AR_x86_64_pc_windows_gnu=llvm-ar \
CFLAGS_x86_64_pc_windows_gnu='-ffreestanding -DRING_CORE_NOSTDLIBINC=1 -DNDEBUG' \
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked
```

These are validation-only environment flags, not application defaults or a
replacement for native Windows/MSVC verification. The check compiled target
code; it did not link or execute a Windows desktop binary. Normal Windows GNU
builds should use MinGW with its target headers/libraries.

[GitHub CI](../.github/workflows/ci.yml) adds Linux and native Windows/MSVC jobs
for formatting, all-target checks, Clippy, tests, and the headless smoke run. It
has not run here; pushing to GitHub will trigger it.

## Remaining limits and next step

Physical Remote/toy versions, localhost/LAN hardware behavior, native Windows
execution, native Wayland launch, and measured CPU/latency remain unverified.
LMU telemetry is not yet implemented. See [LOVENSE.md](LOVENSE.md) for sources,
safety timing and hardware acceptance checks.

Phase 3 starts by verifying LMU's shipped SDK/header and mapping contract, then
adding a shared layout-checked parser and Windows mapping reader. See
[ADAPTERS.md](ADAPTERS.md#phase-3-windows-lmu).
