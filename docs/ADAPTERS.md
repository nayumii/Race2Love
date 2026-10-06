# Adapter implementation plan

This document describes future work, not capabilities already implemented.

## Phase 2: Lovense local backend

Add `race2love-lovense` depending on core, serde, reqwest, Tokio and tracing.
Confirm the current reqwest release/features first; use an HTTP client with a
short timeout, bounded response body, no uncontrolled redirects, and explicit
connection state. Do not add a cloud discovery dependency.

Start from the official [Standard API / Game Mode documentation](https://developer.lovense.com/docs/standard-solutions/standard-api)
and [local Game Mode demo](https://developer.lovense.com/standard-api-demo-game-mode).
The direct local `/command` endpoint supports `GetToys` and `Function` commands.
Remote variants can differ in HTTP/HTTPS port and response types; use the address
reported by the user's Remote. The host and optional port are already persisted.
Add a protocol setting during this phase once behavior is verified.

Implementation order:

1. Typed `GetToys` request/response parser, tested with a fake HTTP server for
   disconnected toys, malformed bodies, unexpected status/types and timeouts.
   Some responses encode the `toys` object as a JSON string; verify and support
   actual documented/observed response shapes without permissive guessing.
2. UI Connect/Disconnect and selected-toy discovery. Do not command every toy
   implicitly when a toy identifier is omitted. Separate endpoint discovery from
   discovery of toys connected to a known endpoint.
3. Normalized vibration conversion and idempotent Stop for the selected toy.
   Verify vibration step range and finite `timeSec` rules with current docs and
   test responses. Never use indefinite commands as the default.
4. Short leases plus renewal for unchanged output. Extend the output gate with a
   backend renewal deadline, bounded by its normal rate limit. Avoid stale cached
   positive output when a new device connects. Test command expiry separately
   from normal-exit Stop.
5. A short manual test pulse that respects global/maximum intensity, emergency
   stop, connection state, and lease expiry. It must have a deliberate route that
   does not depend on the presence of racing telemetry.
6. Bounded reconnect backoff, toy status updates and fresh-session Stop. Display
   concise errors and retain protocol/transport details in tracing logs.

Acceptance: a fake Remote proves request bodies, response handling, finite lease
renewal, deduplication, timeout/fault behavior, emergency stop during a request,
disconnect/reconnect, test expiry and shutdown Stop. Then verify an explicitly
selected real toy on localhost and LAN. A LAN outage must expire vibration even
when a stop request cannot arrive.

## Phase 3: Windows LMU

Create `race2love-lmu`, separating a layout-checked parser from platform mapping
access. Verify the current LMU-shipped SDK/header and mapping contract before
adding structures or offsets. Prefer the native shared-memory interface rather
than a legacy third-party plugin. The official [LMU V1.3 notes](https://lemansultimate.com/le-mans-ultimate-releases-v1-3-update-with-final-elms-content-performance-updates/)
confirm new shared-memory parameters, so layout assumptions need version checks.

Open a native mapping read-only with the `windows` crate where required. Check
buffer size/version, reader consistency, player selection, units and game/sample
freshness. Initially normalize RPM, maximum RPM, gear, speed, throttle and brake.
Do not require elevation. Fixtures should prove bounds/offset handling, malformed
buffers, player selection, consistent reads and teardown when LMU exits.

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
