# Phase 6 and kerb fallback validation

Recorded 2026-10-06, Rust/Cargo 1.94.1. Formatting, workspace/all-target checks
and strict Clippy pass for Linux and Windows GNU. All **85 Linux tests** pass
(37 core, four GUI, 19 LMU, six Lovense unit, 19 fake Remote). All **80 Windows
tests** pass as MinGW binaries under isolated Wine (14 LMU tests on that target).
This does not replace native Windows/MSVC or physical-device acceptance.

New regressions cover neutral-bridged shifts through decoded LMU telemetry and
the fake Remote pipeline, independent slip/road/impact generators, retained
impact events, malformed optional data, suspension derivatives using game time,
discontinuities, terrain labels, loaded-wheel contact, bounded graph history,
profile serialization and limits. Road tests cover **all kerb flags false**,
alternating vertical acceleration, constant baseline rejection, gaps and engine
mixing headroom. No new dependencies or output-task architecture were introduced.

The owner reported working grass feedback but almost absent kerb feedback, with
all rumble-strip flags false. The new vertical-vibration fallback addresses that
missing flag without inventing material classifications. Actual kerb feel and
false-positive tuning are **not yet confirmed**. Gear display was reported working;
the brief-neutral failure was reproduced synthetically, not captured from live LMU.

The installed SDK was inspected during implementation and the initial wheel,
acceleration and impact assertions compiled against its Windows x64 headers.
A later recheck (including the new terrain-name assertion) could not run because
the supplied LMU installation path no longer contained the SDK. Re-run
`tools/verify-lmu-layout.cpp` against the current installation after game updates.
No proprietary headers or captured game data were added to the repository.

Earlier validation records below describe their respective revisions.

# Pattern cycling correction

The owner reported both Pattern modes cycling from low to maximum intensity.
The earlier digital comparison assumed instantaneous schedule replacement and
does not establish reliable behavior on their Remote/toy. The precise firmware
behavior remains unverified. Direct Vibrate is restored as the default; explicitly
saved Pattern modes are preserved and labeled experimental.

A loopback regression reproduced overlapping active Pattern schedules before
the fix (four overlapping replacements in the plain Pattern ramp). The corrected
backend first issues targeted Function/Vibrate with `stopPrevious: 1` at the new
first strength, explicitly canceling old schedules. Pattern replacement now waits
for one 110 ms slot after acknowledgement while the existing runtime keeps the
latest target; no new task or queue is introduced. Sharp changes use direct
Vibrate and bypass that delay, as do Stop, zero and tighter ceilings.

The regression exercises both modes, increasing targets every 20 ms, requiring
multiple actual replacements, at least 110 ms between Pattern receipts, zero
overlapping schedules and the final latest target within 220 ms. It also checks
a sharp increase within 100 ms, emergency Stop within 100 ms, and no replay. These
are loopback limits, not measured hardware timings. Malformed/error installation
checks now require stopping the positive Function prelude too.

Changing renewal timing to acknowledgement exposed and fixed a core timing race:
a renewal measured from request start could arrive before the backend's renewal
deadline, be suppressed, then be recorded as renewed by the core. The existing
direct Vibrate lease test failed in isolation before this correction and passes
afterwards. Existing fault and safety behavior remains covered.

Recorded 2026-10-06 with Rust/Cargo stable 1.94.1. All **74 Linux tests** pass
(31 core, three GUI, 15 LMU, six Lovense unit tests and 19 fake Remote tests).
All **69 Windows tests** also passed as MinGW executables under isolated Wine
11.19. Formatting, Linux/Windows workspace/all-targets checks and Clippy with
`-D warnings` passed. The corrected digital comparison includes
cadence and both POSTs per Pattern: mean absolute errors are 0.516 (Direct Vibrate,
six requests), 0.611 (Pattern, 13 requests) and 0.328 (Pattern/dithering, 35 requests).
No physical device is commanded by these tests. Hardware cycling, perceived
smoothness and latency must still be retested with the corrected executable.

# Lovense output smoothing validation (superseded)

Recorded 2026-10-06 using Rust/Cargo stable 1.94.1. Formatting and Linux/Windows
GNU workspace/all-targets checks and Clippy with `-D warnings` passed. All **73
Linux tests** passed: 31 core, three GUI, 15 LMU, six Lovense unit tests and 18 fake
Remote tests. All **68 Windows tests** also passed as MinGW-built executables
under Wine 11.19 in an isolated temporary prefix, including ten LMU tests with
four Win32 fixtures. This is not native MSVC or physical device validation.

The implementation retains the existing runtime tasks, channels, output rate,
timeouts and finite leases. No dependency or configuration migration is added.
Old TOML files load with Pattern/dithering enabled; all three mode settings
round-trip, and the GUI test saves the selectable previous Vibrate mode.

New tests verify fractional time averages, interpolation, immediate large changes
and zero, finite Pattern expiry, stable command suppression, targeted Stop,
unsupported-command fallback and reconnect reset. Malformed responses and other
API errors fail closed. Emergency Stop interrupts a pending Pattern request. A
full runtime/fake Remote test checks every scheduled level against the absolute
ceiling, including tightening it while the mean target remains unchanged, then
checks telemetry timeout and shutdown. Existing fault tests run in direct Vibrate
mode to preserve their baseline. No test commands a physical device.

A deterministic comparison ramps native intensity from 10 to 13 over two seconds,
updates targets every 40 ms and samples scheduled levels every 5 ms. It assumes
zero network/motor latency. Direct Vibrate has mean absolute error **0.516** native
levels with six commands; Pattern without dithering has **0.536** with seven;
Pattern/dithering has **0.306** with 50. Signed mean error improves from **-0.516**
to **-0.056**. These are digital quantization measurements, not measured physical
smoothness. Fractional targets can increase changing-output traffic up to the
existing 25 Hz default cap; stable output only renews the lease every 500 ms.

Physical A/B testing must still compare perceived smoothness, possible dithering
flutter and feedback latency on the owner's Remote/device. Small changes may
settle on the next 110 ms Pattern slot; large changes and Stop bypass the ramp.
The API details, mode comparison and A/B procedure are in [LOVENSE.md](LOVENSE.md).

# Phase 5 validation record

Recorded 2026-10-06 on Linux x86-64, Rust/Cargo stable 1.94.1. The owner confirmed
the corrected Windows executable works, following the earlier live Proton and
Lovense success reports. That confirmation is a user-reported live result;
Windows toy/Remote versions and detailed physical fault/latency checks were not
recorded. Phase 5 retains the existing adapters and timing defaults.

Formatting, Linux/Windows GNU workspace/all-targets checks and Clippy with
`-D warnings` passed. All **62 Linux workspace tests** passed: 30 core, three GUI,
15 LMU, one Lovense backoff and 13 fake Remote tests. No new dependency,
configuration migration, telemetry layout change or proprietary data is included.

All **57 Windows test cases** also passed as MinGW-built Windows executables
under Wine 11.19 in an isolated temporary prefix: 30 core, three GUI, ten LMU
(including four Win32 fixtures), one Lovense backoff and 13 fake Remote tests.
This includes the new Phase 5 integration test and all existing fault tests. Wine
execution complements the owner's native Windows confirmation; it is not a
native MSVC or detailed physical hardware test. All builds/tests used the locked
offline cache and Cargo target directory under `/tmp`.

The native Linux executable completed a five-second display-free Demo with fresh
gear/RPM, nonzero mixed/mock output and final output **0.0** after awaited shutdown.

Three regression tests reproduced two mixer problems before correction: a weak
high-priority pulse suppressed existing vibration (including combined background
effects), and subtractive soft mixing lost precision at native device steps.
The corrected mixer preserves each priority layer's background, evaluates the
same bounded soft sum without subtractive cancellation, and allocates no memory
per sample. Tests now preserve baseline throughout attack/hold/release, exact
single-effect levels and the existing priority strength/clamping behavior. A
fourth new core test covers priority admission when the 16-transient capacity is
full.

The new end-to-end test uses a controllable normalized telemetry source, the
actual runtime/effect engine/mixer and the real Lovense backend against a
loopback-only fake Remote. It checks below-threshold/end RPM behavior, all three
curves, up/downshift pulses without baseline dips, disabling/re-enabling a pulse,
global scaling and zero, a 32% ceiling floored to Lovense step 6, telemetry timeout
during a one-second held shift, recovery without replay and final shutdown Stop.
It commands no physical device. Existing LMU fixtures separately verify decoding
and source loss/restart through the shared pipeline.

The GUI identifies Phase 5 and shows the output interval when configuring short
shift pulses. The default 140 ms envelope and 25 Hz output cap are unchanged;
pulses shorter than the configured output interval can be missed. Broader
physical fault acceptance, native MSVC build validation, Wayland runtime checks
and CPU/latency measurements remain separate work. Phase 6 begins by verifying
reliable wheel/road/impact signals before implementing their generators/graphs.

# Windows telemetry freeze correction

The owner reported Race2Love showing frozen telemetry on Windows while TinyPedal
continued to work, including with TinyPedal completely closed. The old reader
required successful zero-timeout polls of Hold and Data notification events before
every locked copy. This conflated notification availability with advancing data.

A Windows regression fixture publishes valid, advancing player telemetry while
its separate Hold notification stays unavailable. The old reader reproduced the
failure: no player frame was returned. The corrected reader polls the mapping
under the existing SDK lock without consuming either update notification. It
retains player-clock freshness, stale-output shutdown, process-exit detection,
read-only telemetry and contention handling.

All **10 Windows LMU tests** passed under Wine 11.19 in an isolated temporary
prefix, including the four Win32 fixtures. This executes Windows handles, mappings,
events, lock atomics and the shared pipeline; it is stronger than a compile check,
while remaining distinct from testing LMU on native Windows. The regression also
checks that another client's consumable Data notification remains signaled.

Formatting, Linux/Windows workspace/all-targets checks and Clippy passed. All
**57 Linux workspace tests** passed, including fake Remote coverage. A MinGW-built
Windows x64 release executable completed the five-second display-free Demo under
Wine, then shut down all workers with final mock output **0.0**. The artifact uses
the Windows GUI subsystem and imports standard Windows DLLs; no extra MinGW
runtime DLL is required. A read-only console telemetry probe is included for
Windows diagnostics.
Reproduce the fixture execution from a Linux host with Wine and MinGW installed:

```sh
cargo test -p race2love-lmu --target x86_64-pc-windows-gnu --no-run --locked
mkdir -p /tmp/race2love-windows-tests
WINEPREFIX=/tmp/race2love-windows-tests WINEDEBUG=-all \
  wine target/x86_64-pc-windows-gnu/debug/deps/race2love_lmu-<hash>.exe
cargo build --release --target x86_64-pc-windows-gnu --locked
```

Use the actual hashed test executable reported by Cargo. The isolated prefix keeps
fixtures separate from the game's Wine prefix. Tests create unique fixture names,
not production LMU objects. All runs here used the offline cache and Cargo target
directory under `/tmp`.

The owner subsequently confirmed the corrected executable works on Windows.
Detailed physical fault/latency checks remain unrecorded. Sources and protocol
distinctions are documented in [LMU.md](LMU.md); no reference code or
proprietary SDK header was copied. Historical Phase 3/4 results below describe the
older builds and are retained as records, including their previous gate behavior.

# Phase 4 validation record

Recorded 2026-10-06 on Linux x86-64, Rust/Cargo stable 1.94.1. Automated pipeline checks
use `MockDevice`; fake Remote tests use loopback only. The telemetry-only probe
has no device/network backend. The owner also confirmed the live LMU → Lovense
output worked after explicitly resuming; earlier Phase 2 hardware testing remains
recorded below.

| Check | Result |
| --- | --- |
| `cargo fmt --all` and `cargo fmt --all --check` | Passed |
| Linux workspace/all-targets `cargo check` | Passed |
| Linux workspace/all-targets Clippy with `-D warnings` | Passed |
| `cargo test --workspace --locked --offline` | 57 passed, 0 failed |
| Native Linux build and five-second display-free Demo | Passed; final mock output 0.0 |
| Windows GNU workspace/all-targets check and Clippy | Passed; Windows fixtures compiled, not executed |
| Native Linux/X11 window with synthetic Wine memfd producer | Passed; discovered real `/proc` fd and displayed normalized values |
| Real LMU 1.4.2 under Proton | Live dashboard and 30-second probe passed; about 60 fresh frames/s |
| Live LMU → Lovense output | Owner confirmed it worked after Resume output |

All 48 prior Linux tests remain; nine new Linux adapter tests cover exact process
matching and stat parsing, prefix/parent association, foreign adapters, candidate
layout/size rejection, PID reuse, fd replacement, adapter/game loss, truncation,
consistency skips, actual memfd reads through `/proc`, selecting advancing clocks
with constant RPM, and full-pipeline freeze/player exit/process loss/reconnect and
shutdown. GUI coverage now exercises Linux LMU selection and its Stop latch.
The additional Windows-only tests remain compiled for native CI (51 on Windows).
The only added direct dependency is Linux test-only `rustix` 1.1.5; acquisition
uses safe standard-library file I/O and the existing decoder.

The native synthetic test used a disposable process with LMU's executable
argument and a 327680-byte `wine-mapping` memfd containing independently generated
fixture values. Race2Love discovered the `/proc` descriptor, displayed Spa / fixture
GT3, 13 m/s, 7000/8000 RPM, gear 4, throttle 75% and brake 12.5%, and generated only
mock output. Frozen telemetry and discovery ambiguity cleared output. The fixture
was stopped before the real-game check; it does not constitute live LMU validation.

The real game then exposed a 327680-byte `wine-mapping` descriptor in the game and
its associated plugin adapter. Race2Love discovered the game's descriptor without
any configured path/PID/Proton version. With no live player car, Dashboard reported
that state, retained its connection and kept output at zero. After the owner
entered the car, the real session supplied game marker **14200**, Practice at
Autodromo Enzo e Dino Ferrari, and Manthey DK Engineering 2026 #91:LM. Dashboard
displayed live speed/RPM/max RPM/gear/throttle/brake; the owner confirmed this
information was correct. The read-only 30-second probe observed approximately
60 fresh frames per second while braking, accelerating and shifting, with max RPM
9400. The probe disconnected normally. Frozen game clocks were also observed
clearing output and causing rediscovery. No game inputs were automated.

The desktop window started on mock output. The owner subsequently connected and
selected their Lovense device through the UI, then reported no vibration. At that
point both connections and telemetry were live, while the UI showed **Emergency
stop latched** and zero mixed/target/applied output. The owner was directed to the
existing **Resume output** button; an explanatory Stop/Resume hint was added.
Race2Love never automatically clears this latch after device/source changes.
After Resume, Dashboard showed **Running**, mixed output 40%, scaled target 20%
and applied Lovense output 15% in one observation, alongside live player data.
The owner then confirmed physical output worked and identified the missed Resume
action as the cause. This is a live end-to-end success report; toy/Remote/Proton
versions and a complete physical fault/latency acceptance run were not recorded.

Linux's repeated read-only snapshots reduce inconsistency but cannot provide the
Windows SDK lock's transaction guarantee. Exact bounds, heuristics, protocol
references and `/proc` limitations are in [LMU.md](LMU.md). Full game/toy fault
acceptance (live game exit/restart, disconnect/reconnect and physical Stop confirmation),
native Windows/MSVC execution, Wayland and latency/CPU benchmarking
remain separate checks. No proprietary SDK headers or real telemetry dump are
redistributed.

Use the reproduction commands and Windows Clang environment below. All checks used
the lockfile/offline cache; CI has not run in this local session.

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
