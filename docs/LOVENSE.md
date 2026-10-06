# Local Lovense backend

`race2love-lovense` uses the local Standard API available in Lovense Remote's
Game Mode. It supports a configured endpoint on the same computer or LAN.
There is no cloud API, developer token, QR callback, Bluetooth implementation,
or automatic network scan. Nothing connects or vibrates on launch.

## Setup and use

Pair a toy in Remote. Enable **Discover → Game Mode → Enable LAN** on mobile or
**Allow Control** on PC. Enter the displayed address/port in Devices / Settings,
choose HTTP or HTTPS, then Connect. Local HTTP preset fills `127.0.0.1:20010`;
confirm the port for your Remote variant. PC's documented HTTPS address is
`127-0-0-1.lovense.club:30010`. HTTPS verifies certificates and hostname; there
is no option to bypass verification.

Connect discovers toys without selecting one. Choose a connected toy explicitly.
Reported non-vibration devices cannot be selected in the UI. Older responses may
omit capabilities; those rows display that uncertainty, so verify the toy before
testing. Neither selection nor connection state is stored in TOML.

Connection/selection changes latch emergency stop. Pause Demo, Resume output,
then Test for one second. Test uses 40% multiplied by global intensity, capped by
maximum intensity, and leaves Demo paused. It deliberately works without game
telemetry. Emergency Stop cancels Test and cannot be cleared by Test. Re-enable
Demo to run engine/shift effects through the toy, or select native Windows LMU
on Dashboard (see [LMU.md](LMU.md)). Disconnect stops the selected toy and restores
mock output, stopped.

Endpoint/policy edits latch output off and apply on Connect. The active endpoint
is displayed separately from edited preferences. Reconnected toys stay stopped
until explicit Resume. A reconnect never replays a cached positive command.

## Protocol and bounds

| Item | Implementation |
| --- | --- |
| Endpoint | POST `{http or https}://{host}:{port}/command` |
| Headers | JSON content type; `X-platform: Race2Love` |
| Discovery | `GetToys`; typed envelope and toy map |
| Toy responses | Object or JSON-encoded object; numeric/string status and battery |
| Positive output | Direct `Function/Vibrate` by default; experimental modes use `Pattern`, `apiVer: 2`, explicit `toy`, preceded by targeted Function cancellation |
| Conversion | Keep native targets as `f32` 0–20; emit bounded integer levels, optionally with temporal dithering |
| Lease | `timeSec: 2`; unchanged positive output renewed every 500 ms |
| Stop | Targeted `Function` with `action: Stop`, `timeSec: 0` |
| Request cap | Core target submissions default 25 Hz; small Pattern changes wait at least 110 ms after acknowledgement; each Pattern batch uses two sequential POSTs |
| Status polling | Every two seconds while connected; Stop preempts the wait |
| Request timeout | Configurable 100–5000 ms, default 1000 ms |
| Response limit | 64 KiB, including bodies without a Content-Length |
| Discovery limit | 128 toys; validated identities, statuses, and battery values |
| Queues | Latest settings/selection via watch; at most eight device requests |
| Connection retries | At most five, at 1/2/4/8/16 seconds; Connect starts a new attempt |

## Smoother output and comparison

Direct Vibrate (`lovense.output_mode = "vibrate"`) is again the default. The owner
reported both Pattern modes cycling from low to maximum intensity rather than
tracking correctly. The original digital comparison did not model Remote's
schedule replacement behavior and was insufficient to establish hardware support.
Pattern modes remain **experimental**, with saved explicit choices preserved.
Existing TOML files without an output mode now retain the original Vibrate behavior.

The corrected Pattern modes keep targets as `f32` 0–20, to 0.01 native level, rather
than truncating before the backend sees them. Core telemetry/effects remain
normalized `0..=1`; shaping state stays in `smoothing.rs`. No new task, queue or
dependency is introduced. The device's earliest replacement deadline travels
through the existing snapshot. The runtime samples normally and takes the latest
current target when that deadline passes; it does not queue old intensity changes.

Pattern uses `rule: "V:1;F:v;S:110#"`, 19 explicitly listed integer strengths,
an explicit toy ID, `apiVer: 2` and `timeSec: 2`. 110 ms satisfies the API table's
interval requirement (>100 ms), despite its 100 ms example. The list covers the
two-second lease without relying on undocumented looping or decimal strengths.

**Replacement is now explicit.** Before each Pattern, the worker sends the existing
targeted Function/Vibrate at the new first strength with `stopPrevious: 1`. This
cancels older schedules using the documented Function contract, without inserting
an extra zero-strength step. Pattern itself has no documented stopPrevious field.
If installation fails, Stop also clears that positive prelude. A pair uses two
HTTP requests, rather than assuming new Pattern commands cancel their predecessors.

Small changes wait at least one 110 ms slot after the previous Pattern's HTTP
acknowledgement. The normal output tick can add up to one configured update period
(default 40 ms). This replaces the previous strategy that could restart 110 ms
Patterns every 40 ms. The deadline starts after acknowledgement, so a slow response
cannot consume the slot budget. Lease renewal likewise starts after acknowledgement.
Startup, Stop, zero and tighter ceilings bypass Pattern pacing. Changes of at least
two native levels use **direct Vibrate** immediately at the normal configured rate,
canceling old Patterns and retaining sharp feedback. A subsequent small change can
return to Pattern. Small ramps still start halfway toward the target and settle
over 60 ms in the scheduled samples.

Error diffusion represents fractional means with adjacent levels: 10.5 uses 10/11.
Only actually elapsed samples contribute to the error accumulator. Every strength
honors the **floor of the independent ceiling**: a 53% ceiling permits level 10 but
forbids level 11 even with a mean target of 10.5. Ceiling changes are forwarded even
when the mean target is unchanged. Stable integer output uses Function; stable
fractions use Pattern. Duplicates are suppressed except for 500 ms lease renewals.
Stop, disconnect, selection, errors and shutdown clear all shaping/cadence state.
No deferred positive output can replay after Stop.

Unsupported Pattern responses (documented codes 400/403) trigger one Stop + direct
Vibrate fallback per connection, shown in the UI. Malformed replies, invalid
parameters (404) and network/HTTP errors fail closed. Reconnect permits a new try.

Devices / Settings offers all three saved modes. To apply a choice, click Connect,
reselect the toy, then Resume. Start with Direct Vibrate. Compare the experimental
modes only after the corrected build is confirmed on the actual Remote/toy.

| Mode | Two-second 10→13 ramp: signed mean error | Mean absolute error | Positive HTTP requests |
| --- | ---: | ---: | ---: |
| Direct Vibrate (`vibrate`, default) | -0.516 levels | 0.516 levels | 6 |
| Pattern smoothing (`pattern`, experimental) | -0.611 levels | 0.611 levels | 13 |
| Pattern smoothing + dithering (`pattern_dither`, experimental) | -0.176 levels | 0.328 levels | 35 |

These are **digital simulation** results, with targets checked every 40 ms, native
levels sampled every 5 ms, and no network/motor latency. They include cadence and
both POSTs in each Pattern batch. They do not establish perceived smoothness or
Remote/firmware compatibility. Pattern alone cannot add device strength levels;
dithering can feel like flutter. Small-change coalescing adds latency, so hardware
feel remains the deciding comparison. Reproduce the numbers with:

```sh
cargo test -p race2love-lovense compare_direct_pattern -- --nocapture
```

`Acknowledged device target` is the input accepted by the backend, not motor
readback. Native rounding/ceilings can lower its achievable average. If either
experimental mode still cycles or feels irregular, use `vibrate` and retain the
working RPM/gear pipeline. Remote version, host platform and toy model are useful
for further diagnosis; an OK API response alone does not prove schedule behavior.

## Stop and failure behavior

Finite commands follow the official duration rule: positive `timeSec` must exceed
one second. Race2Love never sends indefinite positive output. Stop's zero duration
does not start vibration. Every function request includes one toy ID; an omitted
ID would affect every paired toy and is not used.

Canceled positive requests can already have reached Remote. The worker cancels
the HTTP future and follows with Stop. If Remote is unreachable or the app
crashes, previously accepted commands expire without renewal under the documented
API. Delayed requests and vendor behavior still need hardware verification;
instant physical stopping cannot be guaranteed across a network outage.

Communication errors invalidate readiness, latch output off, and initiate bounded
connection recovery when enabled. Failed Stop requests while disconnected do not
change the connection epoch again or form a retry loop. Malformed responses fail
closed and appear as concise UI errors, with technical details in tracing logs.

## Dependencies and sources

[reqwest 0.13.5](https://docs.rs/reqwest/0.13.5/reqwest/) is the maintained Tokio HTTP
client requested by the project. Its [release manifest](https://github.com/seanmonstar/reqwest/blob/v0.13.5/Cargo.toml)
declares Rust 1.85. Features are limited to JSON and rustls; HTTP/2, compression,
cookies, and system proxies are disabled. Redirects and automatic HTTP retries
are disabled explicitly. rustls uses ring rather than the larger default AWS-LC
build; the provider is initialized before constructing a client. TLS roots use
the platform verifier. Linux builds need a C compiler; Windows MSVC uses Visual
Studio Build Tools. Windows GNU cross-builds need MinGW.

Protocol references, consulted 2026-10-06:

- [Lovense Standard API](https://developer.lovense.com/docs/standard-solutions/standard-api)
  for local requests, headers, discovery fields, level ranges and finite durations.
- [Official Game Mode demo](https://developer.lovense.com/standard-api-demo-game-mode)
  for direct LAN HTTP/HTTPS setup.
- [Remote integration guide](https://developer.lovense.com/docs/game-engine-plugins/ue-plugin-remote)
  for PC Allow Control setup.

No third-party implementation was copied. Fake Remote tests validate both typed
parsing and the complete pipeline, including faults, renewal, cancellation and
command expiry. Pattern tests also cover fractional targets, native playback,
duplicates, expiry, an in-flight Stop, compatibility fallback, malformed responses
and a tighter ceiling with an unchanged target. Before declaring a particular Remote/toy version verified, check
GetToys, explicit selection, Test, emergency stop, disconnect/reconnect, normal
exit, and command expiry during LAN loss with that version. Record the platform,
Remote version and toy model. On 2026-10-06 the project owner reported that the
physical hardware test worked well. Toy model, Remote version, endpoint and
individual acceptance checks were not recorded; broader hardware compatibility
and fault/expiry behavior remain unverified. Automated tests use fake Remote only.
