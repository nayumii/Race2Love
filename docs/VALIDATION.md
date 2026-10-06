# Phase 3 validation record

Recorded 2026-10-06 on Linux x86-64, Rust/Cargo stable 1.94.1. Automated device
tests use loopback-only fake Remote. The owner separately reported successful
physical hardware testing; model, Remote version, endpoint and fault acceptance
details were not provided. No live LMU telemetry or physical device was accessed
by the automated checks in this phase.

## Completed checks

| Check | Result |
| --- | --- |
| `cargo fmt --all` and `cargo fmt --all --check` | Passed |
| `cargo check --workspace --all-targets --locked --offline` | Passed |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed |
| `cargo test --workspace --locked --offline` | 48 passed, 0 failed |
| `cargo build --locked --offline` | Passed, native Linux executable |
| Display-free Demo, five seconds | Passed, final mock output 0.0 |
| Native Linux/X11 window | Passed, source controls, live values, Escape and normal closure |
| Windows GNU all-targets check/Clippy using Clang | Passed, including three Windows fixtures |
| Installed SDK offsets/sizes, Clang Windows x64 ABI static assertions | Passed |

The complete workspace is compiled, including eframe, Lovense and the new LMU
adapter. `Cargo.lock` is included. The minimum Rust version remains 1.92; these
checks executed with 1.94.1.

## Test coverage

- 26 core tests: curves/clamps, shifts, envelopes, priorities, scaling, timeout,
  config/files, live settings, heartbeat/failure safety, cancellation, source and
  device switching (including immediate Resume), stop and shutdown.
- Three actual egui input/render tests: effects/settings/persistence, device
  controls, normalized LMU-style dashboard values, source selection/availability,
  Stop/Resume/Escape and pause. No desktop server is required for these tests.
- Six platform-independent LMU tests: synthetic contract bytes for player slots
  0/5/103, offsets/units, optional signal absence, malformed/truncated data, unknown
  game families, non-finite/out-of-range numbers, reverse/neutral/zero RPM, bounded
  strings, inactive player/realtime, progress, freeze, player exit, game restart and
  shutdown through the actual pipeline.
- One Lovense backoff test and 12 fake Remote integration tests retain Phase 2
  protocol, timeout, expiry/renewal, cancellation, reconnect, selection, manual-test,
  fault and shutdown coverage.
- Three additional Windows-only tests compile here and are configured for native
  Windows CI: isolated sections/events verify read-only data, Hold-before-Data
  notification order, closed gates, contention, waiter wakeup, missing/undersized
  sections, process exit with retained mapping, and teardown. They never create
  production LMU object names or launch the game. They were not executed on this
  Linux host (51 total tests on Windows).

## Installed SDK verification

After installation finished, the game headers were read directly from its
`Support/SharedMemoryInterface` directory. Steam manifest build ID: **25661166**.
Recorded SHA-256:

| Header | SHA-256 |
| --- | --- |
| `InternalsPlugin.hpp` | `9b6ee8cf610fa5049b18df580a9a9bc9ebb91346fc466584d576a6442abcf68f` |
| `PluginObjects.hpp` | `f65f1d2226af1acb277f8337fb10d8955db384118fc36eef95ea446be058e247` |
| `SharedMemoryInterface.hpp` | `a82833d5e9e277a7c3af8802518e35ea62f4ec7c7c7c9083650282e4edf1f8bf` |

The current SDK confirmed the layout and shared lock, and introduced a gate
contract absent from older references: check `LMU_Data_HoldEvent` before
`LMU_Data_DataEvent`, then lock/copy. The implementation follows that order using
nonblocking polls.

[verify-lmu-layout.cpp](../tools/verify-lmu-layout.cpp) checks every used record size
and offset against the installed, unmodified SDK. Clang 23 syntax-only compilation
targeted `x86_64-pc-windows-msvc`, with temporary minimal Win32/math/optional/utility
declarations because the host lacks Windows C++ headers. These shims do not define
game records. Assertions passed with Windows's four-byte long, eight-byte pointers,
one-byte bool and actual header packing. This verifies ABI layout, not Win32/SDK
execution. Native Windows reproduction with real C++/Windows headers is documented
in [LMU.md](LMU.md). No proprietary headers were modified or copied into the repo.
Reference revisions, licenses and offsets are documented there too.

## Demo and reproduction

The built native executable ran the actual workers for five seconds:

```text
Demo: connected=true, gear=2, rpm=7052, mixed=0.488, mock_output=0.240, commands=38
Telemetry worker stopped
Output worker stopped
Lovense worker stopped
Race2Love shutdown complete output=0.0
```

The Linux/X11 window showed Demo values, source controls and the disabled Linux
LMU option. Escape and normal close were sent only to the owned test window.
Closure awaited the workers and logged output 0.0. Native Wayland is compiled but
not separately exercised.

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -- --demo-seconds 5
cargo run --locked -- --demo
```

Fake Remote tests need loopback socket permission; they never scan LAN or contact
toys. Windows/MSVC uses Visual Studio Build Tools. ring's TLS crypto needs a C
compiler. This host lacks MinGW; Windows GNU checks use Clang/LLVM ar and ring's
freestanding C path:

```sh
rustup target add x86_64-pc-windows-gnu
CC_x86_64_pc_windows_gnu=clang \
AR_x86_64_pc_windows_gnu=llvm-ar \
CFLAGS_x86_64_pc_windows_gnu='-ffreestanding -DRING_CORE_NOSTDLIBINC=1 -DNDEBUG' \
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked
```

The same environment ran Windows Clippy with `-- -D warnings`. These are validation
flags, not defaults or a substitute for native Windows testing: target checks did
not link/run a Windows executable. Normal GNU builds need MinGW headers/libraries.
[CI](../.github/workflows/ci.yml) runs native Linux/Windows checks/tests on push/PR;
it has not run in this local session.

## Remaining acceptance and Phase 4

Windows fixture execution, live Windows LMU, broader Remote/toy/fault coverage,
native Wayland and CPU/end-to-end latency measurements remain. The Windows adapter
and installed layout are implemented/verified; live compatibility is not claimed
from fixtures alone.

Phase 4 adds dynamic LMU/Wine process and memfd discovery under Proton, implements
`SnapshotReader`, and reuses the decoder/freshness handling. Verify a live session
against the installed SDK; isolate/document synchronization and `/proc` limitations,
and test process/mapping replacement without fixed paths, Proton versions, root
privileges or a bridge.
