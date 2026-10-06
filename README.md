# Race2Love

Race2Love is a native Rust desktop application for turning racing telemetry into
configurable haptic feedback. The first simulator/device target is **Le Mans
Ultimate → Lovense Remote/Game Mode**, on Linux with Steam/Proton and Windows
10/11. The design uses no SimHub, Electron, browser frontend, or mandatory cloud
service.

**Phase 4 implementation:** the application starts with **Demo telemetry** and an
**in-memory mock device**. The local Lovense backend is available through an
explicit Connect and toy selection. **Direct Windows and Proton LMU** is selectable on
Dashboard or with `--lmu`. The owner reports
successful Lovense hardware testing and live Proton LMU output after Resume. See [validation status](docs/VALIDATION.md)
for automated checks, live Linux telemetry verification and remaining acceptance.

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
  mixed/scaled/applied output, Demo/LMU selection, telemetry pause, and
  global intensity controls.
- **Effects:** RPM thresholds expressed as percentages of maximum RPM, vibration
  range, linear/exponential/logarithmic curves, and shift pulse envelopes.
- **Devices / Settings:** saved Lovense host/port preferences, update rates,
telemetry timeout, start-minimized, debug values, and configuration location.
  Connect/discover, explicit toy selection, timed test vibration, and disconnect.

Effect changes apply immediately when valid. Remote address/policy changes latch
output off and apply when you click Connect. **Emergency Stop** (also **Esc**) latches
output off until **Resume output** is clicked. Pausing or switching telemetry
also stops output; switching requires Resume.
Changed settings save on normal exit; **Save settings** saves explicitly.

For a display-free smoke run of the same workers:

```sh
cargo run --locked -- --demo-seconds 5
```

For a build that skips compiling native display dependencies, use
`cargo run --locked --no-default-features -- --demo-seconds 5`. Cargo still needs registry
metadata for workspace dependencies on its first resolution.

The command prints a telemetry/output summary, then awaits the workers' shutdown
and final device stop. The display-free Demo never accesses hardware. The GUI also starts with mock
output; physical output requires Connect, toy selection, and explicit Resume.

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
The TLS crypto provider needs the C/C++ compiler supplied by those Build Tools.
The complete workspace also passes a Windows GNU target check from Linux using
Clang with the validation-only flags recorded in [VALIDATION.md](docs/VALIDATION.md).
Normal Windows GNU builds use MinGW. Native MSVC build/runtime validation remains.

## Architecture

```text
TelemetrySource (Demo or direct Windows/Proton LMU)
    ↓ TelemetryFrame (SI units; optional signals)
independent effect generators
    ↓ continuous effects + transient envelopes
EffectMixer (bounded, priority-aware)
    ↓ normalized mixed intensity
global intensity multiplier + absolute maximum
    ↓ watch channel + rate/duplicate gate + watchdog
HapticDevice (MockDevice or local Lovense Remote)
```

The workspace has an executable plus four library crates:

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
crates/race2love-lovense/src/       # typed protocol and local connection worker
crates/race2love-lmu/src/           # shared decoder/freshness, Windows mappings, Linux /proc
crates/race2love-gui/src/lib.rs     # native eframe/egui views
docs/                             # adapter plans and validation record
```

The core contains no LMU layouts or Lovense protocol fields. Telemetry discovery
and reads run in a dedicated blocking worker; effects and output run on a
two-worker Tokio runtime. The
GUI reads snapshots and sends settings/controls through latest-value `watch`
channels; it makes no network requests. Lovense discovery and commands run in
a separate connection worker. See [architecture details](docs/ARCHITECTURE.md).

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
**30 Hz GUI refresh**. Device updates round downward to 1% steps for mock output
and 5% steps for Lovense. Duplicates are suppressed, except for required Lovense
lease renewal every 500 ms. Stops bypass the positive-output rate limit. Paused
or unavailable telemetry polls at 1 Hz and inactive effect/UI timers use 2 Hz. These are starting
choices, not measured latency/CPU guarantees. Lovense commands expire after two seconds without renewal. Hardware timing
and CPU/latency still require measurement.

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
shows all current fields. Lovense protocol is `http` or `https`; HTTPS verifies
certificates. Toy selection and connection state are not persisted.

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

Lovense output always targets one explicitly selected toy. Positive commands use
`timeSec = 2`, `stopPrevious = 1`, and a 500 ms renewal interval. Zero intensity
sends Stop. Source/device switches stop previous output. Reconnected devices stay
stopped until Resume; automatic connection retries stop after five attempts.

The one-second manual Test intentionally works without game telemetry, pauses
Demo, and respects emergency stop and both intensity limits. It leaves Demo
paused when it ends. Enable Demo again to use racing effects.

A crash, power loss, or unreachable Remote can prevent a final network Stop.
Finite command expiry bounds remaining output under the documented Remote API.
This behavior is verified against a fake Remote. The owner reports successful
physical testing; toy/Remote versions and detailed fault checks were not recorded.
See [Lovense details](docs/LOVENSE.md).

## LMU telemetry

LMU provides a native shared-memory interface. Race2Love uses `LMU_Data` directly
and normalizes samples at the adapter boundary, without SimHub or a third-party DLL.
LMU's [official V1.3 announcement](https://lemansultimate.com/le-mans-ultimate-releases-v1-3-update-with-final-elms-content-performance-updates/)
confirms that the shared-memory telemetry interface is evolving.

On **Windows**, enable **Settings → Gameplay → Enable Plugins** in LMU and restart
the game. Select **Le Mans Ultimate** on Dashboard, or launch `race2love.exe --lmu`.
Enter the car in a driving session. The adapter opens read-only telemetry and uses
the SDK lock to copy bounded snapshots; it validates the known 1.2–1.4 layout,
selects the player, and exposes RPM/max RPM, gear, speed, throttle and brake.
Frozen clocks cannot refresh output. Game exit is detected via a process handle;
non-realtime/player exit clears output. Snapshots poll under the SDK lock; frame
notification events are left untouched so they cannot starve the reader. See [LMU.md](docs/LMU.md) for the verified installed layout, sources and
remaining live Windows acceptance.

On **Linux/Proton**, enable Plugins and restart LMU, then select **Le Mans
Ultimate** or run `race2love --lmu`. Race2Love finds the same user's game and
associated `PluginsAdapter.exe` dynamically, validates Wine's shared-memory
backing files through `/proc/<pid>/fd/<n>`, and reuses the Windows decoder. No
Steam library path, Proton version, root privileges or bridge is required.
Reads are read-only and bounded; changed processes/descriptors, truncation and
frozen clocks stop output and trigger rediscovery.

Linux uses two matching reads of the fields it consumes, rather than the Windows
SDK lock: `/proc` does not expose Wine's named synchronization objects. This
reduces torn reads but cannot guarantee a fully atomic producer transaction.
`/proc` restrictions, different users, sandbox boundaries, or Wine builds that do
not expose a supported backing descriptor can prevent access. These conditions
appear on Dashboard and leave output stopped. See [LMU.md](docs/LMU.md) for exact
heuristics, synchronization limits, reference licenses and test status.
Wheel slip, kerb, and collision effects will stay optional until their LMU values,
units, availability, and false-positive behavior are verified. Details are in
[adapter plans](docs/ADAPTERS.md).

## Lovense setup

1. Pair your toy in Lovense Remote.
2. On mobile, open **Discover → Game Mode → Enable LAN**. On PC, enable
   **Allow Control**.
3. In Devices / Settings, enter Remote's host/IP, port, and HTTP/HTTPS protocol.
   The **Local HTTP preset** fills `127.0.0.1:20010`; confirm your Remote's port.
   The documented PC HTTPS endpoint is `127-0-0-1.lovense.club:30010`.
4. Click **Connect / Discover toys**, then select one connected toy. These actions
   latch emergency stop and send Stop before enabling the backend.
5. Pause Demo on Dashboard, click **Resume output**, then use **Test vibration**.
   The test lasts one second at 40% × global intensity, capped by maximum intensity.
6. Enable Demo to run the RPM/shift pipeline with synthetic telemetry. Use
   **Emergency Stop** at any time. **Disconnect / Use Demo output** stops the toy
   and returns to mock output with emergency stop latched.

Mobile setup and the HTTP preset follow the [official Game Mode demo](https://developer.lovense.com/standard-api-demo-game-mode).
PC setup is described in the [Remote integration guide](https://developer.lovense.com/docs/game-engine-plugins/ue-plugin-remote)
and [Standard API](https://developer.lovense.com/docs/standard-solutions/standard-api).
For LAN operation, keep Race2Love and Remote on the same network. Linux can use
mobile Remote over LAN. HTTPS requires the certificate's valid hostname; raw IP
addresses may fail verification. Certificate verification is never disabled.

Discovery asks the configured Remote for its toys. There is no automatic LAN
scan, QR/cloud discovery, developer token, or auto-connect on launch. Reconnects
use bounded backoff; Resume is always required before output returns. Endpoint
errors appear in the GUI and technical details are logged.

## Development checks and next phase

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

All **57 Linux tests** pass: 26 core, three GUI, 15 LMU (including nine Linux
acquisition tests), one reconnect backoff,
and 12 fake Remote tests. Four additional Windows-only mapping/process fixtures
compile here and run under an isolated Wine prefix; native Windows CI is also
configured (52 Windows tests in total). Tests cover effect logic,
configuration, pipeline safety, GUI controls, typed discovery, targeted request
bodies, bounded responses/timeouts, finite expiry, renewal/deduplication,
selection changes, manual-test limits, cancellation, reconnect, toy loss,
persistent Stop failures, and shutdown. Linux tests cover process/prefix discovery,
PID/fd reuse, memfd reads, truncation, inconsistent snapshots and recovery. No real
LMU or toy is required.

`Cargo.lock` is included. See [validation results](docs/VALIDATION.md) for native
Linux and platform compile checks. GitHub CI checks Linux and native Windows/MSVC
on push and pull requests; the new workflow has not run in this local session.

**Next step (Phase 5 acceptance):** the live Proton LMU → RPM/gear effects →
mixer → Lovense path now works, confirmed by the owner. Check physical Stop during
pause/game exit, reconnect and shutdown, then tune defaults/latency before adding
Phase 6 signals. The RPM/shift generators, global scaling and mixer are implemented.

Known limits: native Windows LMU/MSVC execution and native Wayland runtime checks
remain; broader Lovense hardware/fault acceptance is unrecorded; no slip,
kerb, collision effects, graphs, tray, autostart, or named profiles. CPU usage and
end-to-end latency have not been benchmarked.

## License

[LICENSE](LICENSE) is a placeholder pending the owner's license selection. It
does not grant open-source redistribution rights yet. All packages are marked
`publish = false` until that decision is made.
