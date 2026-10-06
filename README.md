# Race2Love

Race2Love is a native Rust desktop application for turning racing telemetry into
configurable haptic feedback. The first simulator/device target is **Le Mans
Ultimate → Lovense Remote/Game Mode**, on Linux with Steam/Proton and Windows
10/11. The design uses no SimHub, Electron, browser frontend, or mandatory cloud
service.

**Phase 1 implementation:** the application starts in **Demo** mode and sends
output to an **in-memory mock device**. Real LMU and Lovense connections are not
implemented yet. See [validation status](docs/VALIDATION.md) for the build checks
and Linux Demo checks completed, plus the remaining platform limitations.

## Try Demo

Install Rust stable **1.92 or later** using [rustup](https://rustup.rs/). The GUI
uses eframe 0.34.3 with its OpenGL renderer and bundled fonts. X11 and Wayland are
enabled; the larger wgpu renderer is disabled. The dependency version supports
the declared minimum compiler version: [eframe workspace manifest](https://github.com/emilk/egui/blob/0.34.3/Cargo.toml).

From the repository directory:

```sh
cargo run --locked -- --demo
```

The native window has three views:

- **Dashboard:** live speed, RPM, gear, throttle, brake, connection indicators,
  mixed/scaled/applied output, Demo pause, and global intensity controls.
- **Effects:** RPM thresholds expressed as percentages of maximum RPM, vibration
  range, linear/exponential/logarithmic curves, and shift pulse envelopes.
- **Devices / Settings:** saved Lovense host/port preferences, update rates,
  telemetry timeout, start-minimized, debug values, and configuration location.
  Physical-device controls are marked as coming in Phase 2.

Changes apply immediately when valid. **Emergency Stop** (also **Esc**) latches
output off until **Resume output** is clicked. Pausing Demo also stops output.
Changed settings save on normal exit; **Save settings** saves explicitly.

For a display-free smoke run of the same workers:

```sh
cargo run --locked -- --demo-seconds 5
```

For a build that skips compiling native display dependencies, use
`cargo run --locked --no-default-features -- --demo-seconds 5`. Cargo still needs registry
metadata for workspace dependencies on its first resolution.

The command prints a telemetry/output summary, then awaits the workers' shutdown
and final device stop. Neither Demo command accesses physical hardware.

## Linux setup

On CachyOS / Arch, install the build tools and native display libraries:

```sh
sudo pacman -S --needed base-devel pkgconf libx11 libxcb libxkbcommon wayland mesa
cargo build --locked --release
./target/release/race2love --demo
```

Use your GPU vendor's normal OpenGL driver if you do not use Mesa. A normal
desktop session is required for the GUI; the display-free smoke run does not
open a window. On Debian/Ubuntu, corresponding packages include `build-essential`,
`pkg-config`, `libx11-dev`, `libxcb-render0-dev`, `libxcb-shape0-dev`,
`libxcb-xfixes0-dev`, `libxkbcommon-dev`, `libwayland-dev`, and `libgl1-mesa-dev`.
The [upstream eframe setup guide](https://github.com/emilk/eframe_template#testing-locally)
provides further distribution notes.

Game reading and application use must run as the same user; Race2Love is not
intended to run as root. Installing OS build packages is a separate setup step.

## Windows setup

Install Rust stable using rustup with the default MSVC target. Install Visual
Studio Build Tools with **Desktop development with C++** and the Windows SDK.
From PowerShell:

```powershell
cargo build --locked --release
.\target\release\race2love.exe --demo
```

The release executable uses the Windows GUI subsystem. No administrator access
is needed to run the application. Debug builds retain the console for logging.
The complete workspace passes a Windows GNU target compile check from Linux.
A native Windows MSVC build and runtime check remain to be performed.

## Architecture

```text
TelemetrySource (Demo now; LMU adapter later)
    ↓ TelemetryFrame (SI units; optional signals)
independent effect generators
    ↓ continuous effects + transient envelopes
EffectMixer (bounded, priority-aware)
    ↓ normalized mixed intensity
global intensity multiplier + absolute maximum
    ↓ watch channel + rate/duplicate gate + watchdog
HapticDevice (MockDevice now; Lovense adapter later)
```

The workspace has an executable plus two library crates:

```text
Cargo.toml
src/main.rs                       # logging, configuration, runtime lifetime
crates/race2love-core/src/
  config.rs                       # defaults, validation, TOML, config path
  telemetry/                      # normalized frames, source trait, Demo/mock
  effects.rs                      # RPM curves, shift detector, effect engine
  mixer.rs                        # transient envelopes and priority mixing
  devices.rs                      # device trait and bounded in-memory mock
  runtime.rs                      # telemetry/effects/output tasks and controls
crates/race2love-gui/src/lib.rs     # native eframe/egui views
docs/                             # adapter plans and validation record
```

Separate adapter crates will be added when their implementations exist. The
core contains no LMU memory layouts or Lovense protocol fields. Telemetry reads,
effects, and output each have their own Tokio task on a two-worker runtime. The
GUI reads snapshots and sends settings/controls through latest-value `watch`
channels; it makes no network requests. See [architecture details](docs/ARCHITECTURE.md).

## Effects and timing

RPM uses `rpm / max_rpm`, maps the configured start/end window through the selected
curve, then interpolates the minimum/maximum vibration. Exponential is `x²`;
logarithmic is `log10(1 + 9x)`. Invalid RPM/max RPM produces zero output.

An adjacent forward gear change creates an attack/hold/release pulse. Neutral,
reverse, first samples, skipped gears, and reconnect/reset samples do not trigger
normal shift pulses. Engine priority is 20 and shift priority is 160.

The mixer gives the highest active priority full strength, attenuates lower
priorities, and combines using a bounded soft sum. At most 16 transient effects
are retained. The global multiplier applies **after mixing**, followed by an
independent absolute intensity ceiling. All output is normalized to `0..=1`.

Defaults are **60 Hz telemetry**, **60 Hz effects**, **25 Hz device output**, and
**30 Hz GUI refresh**. Device updates are quantized downward to 1% steps and
duplicates are suppressed. Stops bypass the positive-output rate limit. Paused
telemetry polls at 1 Hz and inactive effect/UI timers use 2 Hz. These are starting
choices, not measured latency/CPU guarantees. Physical-backend command resolution
and finite-lease refresh must be verified in Phase 2.

Demo simulates a 24-second driving cycle with RPM ramps, up/downshifts, braking,
slip, kerbs, and occasional explicit impacts. Slip, suspension velocity, vertical
acceleration, and impact signals are **synthetic debug values only** in Phase 1.
They do not drive additional effects yet. No rolling graph history is stored.

## Configuration and logging

Default config locations:

- Linux: `$XDG_CONFIG_HOME/race2love/config.toml`, or
  `~/.config/race2love/config.toml` when XDG is unset/relative.
- Windows: `%APPDATA%\race2love\config.toml`.

Set `RACE2LOVE_CONFIG` to override the file location. Missing files use defaults;
invalid files are reported and preserved until an explicit save or a changed
settings save on exit. Writes stage a temporary file in the same directory before
replacement. TOML stores preferences, never active sessions, telemetry frames,
emergency-stop state, or detected devices. [Example config](config.example.toml)
shows all Phase 1 fields.

`RUST_LOG` controls tracing. The default logs connections, configuration activity,
and failures rather than every frame:

```sh
RUST_LOG=race2love_core=debug cargo run -- --demo
```

## Safety

The runtime stops on emergency stop, source pause/disconnection, missing/stale
telemetry (250 ms by default), effects worker failure/stalled heartbeat, and normal
application shutdown. Device connection transitions get a stop before output
resumes. Commands and stops have bounded timeouts. Communication failures latch
output off; only an explicit Stop/Resume permits another attempt. A best-effort
stop is attempted once after an output failure, with no endless retry loop.

Phase 1 uses a mock device and cannot leave a real toy running. Phase 2 must use
finite-duration commands, refresh their lease even when intensity is unchanged,
and verify actual Remote behavior. A crash, power loss, or unreachable Remote
cannot be handled by sending a final network stop; command expiry must provide
that protection. No physical-device crash protection is claimed yet.

## LMU telemetry (planned, Phases 3 and 4)

LMU provides a native shared-memory interface. Race2Love will use that interface
directly and normalize samples at the adapter boundary. It will not require
SimHub or substitute the legacy rFactor 2 plugin format for the native interface.
LMU's [official V1.3 announcement](https://lemansultimate.com/le-mans-ultimate-releases-v1-3-update-with-final-elms-content-performance-updates/)
confirms that the shared-memory telemetry interface is evolving.

On **Windows**, the adapter will open the native named mapping read-only, verify
the current SDK layout/version and sample consistency, select the player's car,
then expose RPM, maximum RPM, gear, speed, throttle, and brake. Mapping names,
offsets, and packing are deliberately not guessed or frozen in Phase 1.

On **Linux/Proton**, the same parser will be used after a platform adapter finds
the relevant LMU/Wine process and shared-memory-backed descriptor dynamically.
The [lmu-rpm-leds project](https://github.com/fpauker/lmu-rpm-leds#how-it-works)
documents accessing Wine memfd mappings through `/proc/<pid>/fd/<n>` without a
bridge, root access, or SimHub. This is a research reference, not a dependency;
no source code was copied. Its GPL-3.0-or-later license must be respected.

The adapter must discover processes/descriptors without assuming a Steam library
path or Proton version. `/proc` mount restrictions, different users, sandboxing,
and Wine implementation changes can prevent access. Opening a descriptor does
not prove it is the correct layout; size/version/consistency checks are required.
Wheel slip, kerb, and collision effects will stay optional until their LMU values,
units, availability, and false-positive behavior are verified. Details are in
[adapter plans](docs/ADAPTERS.md).

## Lovense setup (for Phase 2)

Race2Love currently saves connection preferences but does **not** connect to
Lovense Remote. To prepare a mobile Remote for its future local backend:

1. Pair your toy in Lovense Remote.
2. Open **Discover → Game Mode** and enable **Enable LAN**.
3. Use the IP/port shown by Remote; keep the computer and Remote on the same LAN.
4. Enter that host/port under Devices / Settings. Port `0` means unset in Phase 1.

These steps follow [Lovense's official Game Mode demo](https://developer.lovense.com/standard-api-demo-game-mode).
For PC Remote, the official integration documentation calls the setting
**Allow Control**: [Remote integration guide](https://developer.lovense.com/docs/game-engine-plugins/ue-plugin-remote).
Endpoint/protocol differences must be tested before enabling real output.
The manual LAN path avoids mandatory developer registration, QR/cloud discovery,
and external service calls. Automatically finding Remote on a LAN is a separate
problem from asking a known Remote endpoint for its connected toys.

## Development checks and next phase

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

All **25 tests** pass: 24 core tests and one GUI interaction test. Core and pipeline
tests use Demo, a controllable mock telemetry source, and mock, failing, and slow
devices. They cover curves, clamps, shifts, envelopes, mixer priorities/bounds,
scaling, timeout handling, configuration, live settings, stop/resume, connection
changes, output deduplication, slow requests, faults, and shutdown. The GUI test
drives actual egui input to check controls, effect edits, saved/reloaded TOML,
pause, Escape, and exit. Tests require no desktop display, LMU, or hardware.

The native Demo window was also verified on Linux/X11, including live telemetry,
Escape emergency stop, and normal shutdown with zero output. `Cargo.lock` is
included for reproducible dependency resolution.

**Exact Phase 2 next step:** add `race2love-lovense` with a timeout-bounded reqwest
client, first implement local `/command` `GetToys` discovery and typed response
validation against a fake HTTP server, then add selected-toy finite-duration
vibration/Stop commands and lease renewal. Replace the mock backend only after
the connection/test/stop flow is verified. See [Phase 2 acceptance checks](docs/ADAPTERS.md#phase-2-lovense-local-backend).

Known Phase 1 limits: native Windows/MSVC and native Wayland runtime checks remain;
no real LMU or Lovense adapter; no slip, kerb, collision effects, graphs, tray,
autostart, or named profiles. CPU usage and end-to-end latency have not been
benchmarked.

## License

[LICENSE](LICENSE) is a placeholder pending the owner's license selection. It
does not grant open-source redistribution rights yet. All packages are marked
`publish = false` until that decision is made.
