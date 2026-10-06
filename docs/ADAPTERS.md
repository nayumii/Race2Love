# Adapter implementation plan

The Lovense backend and Windows LMU acquisition are implemented. Proton acquisition
and additional LMU signals remain future work.

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
version guards, ordered update gates and remaining live Windows checks. The
installed SDK headers have been checked without redistributing or modifying them.

Keep absent optional telemetry as `None`. Inspect reliable wheel slip, suspension,
acceleration and damage/impact fields only after the basic adapter works. Surface
availability separately from configuration, so a checked toggle cannot imply the
simulator provides a usable signal.

## Phase 4: Linux / Proton LMU

Research reference: [fpauker/lmu-rpm-leds](https://github.com/fpauker/lmu-rpm-leds).
Its README documents LMU's SDK header, plugin-adapter process, packed telemetry,
and Wine memfd access via `/proc/<pid>/fd/<n>`. Its license is GPL-3.0-or-later.
It is not a dependency and no code/offset implementation has been copied.

Discover the relevant user's LMU/Wine processes and candidate descriptors at a
low rate while disconnected; read telemetry at the separately configured rate
when connected. No hardcoded Steam library path, Proton version or PID. Mapping
discovery alone is insufficient: use the Phase 3 parser's version/size/consistency
checks and ensure samples belong to the player car. Handle process restart,
descriptor replacement, mapping closure and changed layouts as recoverable errors.

Use safe bounded reads where possible. If read-only mmap is required, isolate
unsafe mapping access and assess game truncation/SIGBUS races; do not assume a
mapping remains valid just because opening it succeeded. Document Wine memfd
heuristics as implementation details rather than a stable public Wine ABI.

Access can be blocked by `/proc` restrictions, user mismatch, Steam/app sandbox
boundaries or a changed Wine implementation. Report these conditions and leave
output stopped. Root privileges and an external telemetry bridge are not part of
the design.

## Phase 6: Additional signals

Every generator must document its exact verified LMU signal and normalization.
Kerbs might use suspension velocity or vertical acceleration if validated; a
collision detector must avoid ordinary suspension/kerb false positives. Optional
signals stay unavailable until the adapter can supply meaningful values. The
synthetic Demo slip/kerb/impact fields do not justify inventing an LMU source.

Use a fixed-capacity rolling graph history. Add `egui_plot` only when graphs are
implemented and its release compatibility/maintenance is checked. Tray support,
autostart and profiles remain separate optional improvements.
