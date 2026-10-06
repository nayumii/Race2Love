# Adapter implementation plan

The Lovense backend and direct Windows/Proton LMU acquisition are implemented.
Additional LMU signals remain future work.

## Phase 2: Lovense local backend

`race2love-lovense` implements local `/command` `GetToys` and targeted `Function`
requests using reqwest 0.13.5, typed serde responses, and a bounded Tokio worker.
It accepts documented string-encoded toy maps and object maps, and numeric/string
status fields. Unknown capabilities remain unknown; reported unsupported vibration
is unavailable. No toy is selected automatically or persisted across launches.

Fake Remote acceptance tests cover protocol headers/bodies, malformed responses,
HTTP errors, response limits, timeouts, lease expiry/renewal, duplicate suppression,
selection/disconnect, manual tests, in-flight cancellation, reconnect, toy loss,
Stop failure bounds, and normal shutdown. GUI tests exercise connection controls
and error reporting. The owner reported a successful physical hardware test on
2026-10-06; Remote/toy versions and detailed fault acceptance were not recorded.

See [LOVENSE.md](LOVENSE.md) for setup, official sources, dependency rationale,
finite command timing, retry limits, and hardware verification steps. Discovery is
through a configured endpoint; automatic LAN scanning is not implemented.

## Phase 3: Windows LMU

`race2love-lmu` separates a safe byte decoder/freshness adapter from `SnapshotReader`
platform access. Windows opens `LMU_Data` read-only, uses the existing SDK lock,
tracks game process lifetime, and exposes speed, RPM/max RPM, gear, throttle and
brake plus session/car labels. Synthetic parser/pipeline and Windows named-mapping
fixtures cover malformed data, player selection, lock contention, freeze, teardown
and restart. See [LMU.md](LMU.md) for exact verified reference offsets, licenses,
version guards, notification-independent locked polling and remaining live Windows
checks. The
installed SDK headers have been checked without redistributing or modifying them.

Keep absent optional telemetry as `None`. Inspect reliable wheel slip, suspension,
acceleration and damage/impact fields only after the basic adapter works. Surface
availability separately from configuration, so a checked toggle cannot imply the
simulator provides a usable signal.

## Phase 4: Linux / Proton LMU

`linux.rs` implements `SnapshotReader` using safe, bounded positioned reads.
`linux/proc.rs` discovers same-user game processes and associates plugin adapters
by Wine prefix or parent PID. Backing descriptors are filtered by Wine's memfd or
temporary-file convention, size, and the shared decoder's verified layout. Live
simulation clocks disambiguate retained copies; RPM changes are not required.

Process start times, zombie state, fd device/inode, and backing size are checked
before and after reads. Exit, PID reuse, descriptor closure/replacement and
truncation disconnect safely, even while Race2Love retains a readable file.
The generic source worker handles bounded reconnect and telemetry timeout.

Two matching compact reads reduce inconsistent samples without writing Wine
memory. They do not provide the Windows SDK lock's atomicity guarantee. This
platform-specific limitation is documented in [LMU.md](LMU.md), alongside `/proc`
permissions and unsupported Wine backing arrangements. No unsafe Linux mapping,
fixed Steam path, Proton version, root requirement or telemetry bridge is added.

Linux fixtures exercise discovery, foreign-prefix exclusion, malformed backing,
PID/fd replacement, truncation, inconsistent reads, selection by clock progress,
actual memfd access through `/proc`, and the full mock pipeline's stop/reconnect.
The only added direct dependency is test-only `rustix` for safe memfd creation;
production Linux acquisition uses the standard library. Reference code was not
copied. See [VALIDATION.md](VALIDATION.md) for live acceptance status.

## Phase 5: RPM / gear effects to Lovense

Windows LMU, Proton LMU and Demo feed the same normalized effect engine. Engine
vibration maps `rpm / max_rpm` through the configurable start/end window, then
interpolates minimum/maximum intensity using linear, exponential (`x²`) or
logarithmic (`log10(1 + 9x)`) response. Invalid RPM/max RPM fails closed.

Adjacent forward up/downshifts create independent attack/hold/release pulses.
First samples, neutral/reverse, duplicate observations and skipped gears do not
trigger pulses. Stop, telemetry loss and source switching reset detection and
clear envelopes. Disabling gear shifts also clears an active pulse immediately.
Engine/shift priorities are 20/160. The bounded mixer preserves the existing
background while adding higher-priority pulses; see [ARCHITECTURE.md](ARCHITECTURE.md)
for the exact formula and precision handling. Global scaling and the absolute
output ceiling apply after mixing, before backend-specific downward quantization.

The Effects page applies valid settings live. Defaults retain 60 Hz telemetry
and envelope updates, 25 Hz positive device output and a 10/70/60 ms shift envelope
(140 ms total). Short pulses may fall between the device's 40 ms updates; the UI
shows the configured interval. Duplicate device levels are suppressed, except
finite-lease renewals. Stops bypass throttling and stale telemetry stops output
after 250 ms by default. No production dependency or game-specific effect is added.

Regression tests cover engine baseline preservation throughout a pulse, combined
background layers, priority saturation and exact single-effect values at device
steps. A controlled normalized source exercises the actual runtime and loopback
Lovense backend: RPM thresholds, all curves, up/downshifts, live pulse cancellation,
global scaling, a non-step output ceiling, zero intensity, timeout during a held
pulse, recovery without replay and shutdown. It uses synthetic frames, not a
physical device. The owner has reported the live pipeline working on Proton and
the corrected Windows telemetry build working; detailed fault/latency acceptance
remains in [VALIDATION.md](VALIDATION.md).

## Phase 6: Additional signals

Every generator must document its exact verified LMU signal and normalization.
Kerbs might use suspension velocity or vertical acceleration if validated; a
collision detector must avoid ordinary suspension/kerb false positives. Optional
signals stay unavailable until the adapter can supply meaningful values. The
synthetic Demo slip/kerb/impact fields do not justify inventing an LMU source.

Use a fixed-capacity rolling graph history. Add `egui_plot` only when graphs are
implemented and its release compatibility/maintenance is checked. Tray support,
autostart and profiles remain separate optional improvements.
