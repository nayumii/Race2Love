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
| Positive output | Changing/fractional output uses `Pattern`, `apiVer: 2`, explicit `toy`; stable integer output uses targeted `Function/Vibrate` |
| Conversion | Keep native targets as `f32` 0–20; emit bounded integer levels, optionally with temporal dithering |
| Lease | `timeSec: 2`; unchanged positive output renewed every 500 ms |
| Stop | Targeted `Function` with `action: Stop`, `timeSec: 0` |
| Request cap | Core output rate, default 25 Hz; sequential positive commands |
| Status polling | Every two seconds while connected; Stop preempts the wait |
| Request timeout | Configurable 100–5000 ms, default 1000 ms |
| Response limit | 64 KiB, including bodies without a Content-Length |
| Discovery limit | 128 toys; validated identities, statuses, and battery values |
| Queues | Latest settings/selection via watch; at most eight device requests |
| Connection retries | At most five, at 1/2/4/8/16 seconds; Connect starts a new attempt |

## Smoother output and comparison

The default `lovense.output_mode = "pattern_dither"` retains fractions to 0.01 of
a native level instead of truncating to one of 20 levels before the backend sees
the target. Core telemetry/effects remain normalized `0..=1`; the native `f32`
target and shaping state live only in `smoothing.rs`. No new task, channel or
dependency is introduced. An optional device method forwards the existing
absolute output ceiling; other backends retain their previous behavior.

Changing output uses `Pattern` with `rule: "V:1;F:v;S:110#"`, 19 explicitly
listed integer strengths, an explicit toy ID, `apiVer: 2` and `timeSec: 2`.
110 ms satisfies the API table's interval requirement (>100 ms), even though the
same page shows a 100 ms example. The list covers the full two-second lease;
Race2Love does not depend on undocumented looping or decimal wire strengths.
Only vibration is requested. Stop remains the existing targeted Function/Stop.

For small target changes, shaping starts halfway from the current interpolated
value to the new target, then settles over 60 ms. Fresh commands can replace a
pattern immediately, within the existing configured output cap (25 Hz by default).
There is no 110 ms request queue or wait. Startup, zero, and changes of at least
two native levels bypass the ramp, keeping strong shift pulses responsive. A
small isolated change can still await the next native pattern slot; software
interpolation does not make the toy's physical levels continuous.

Error diffusion represents fractions with adjacent levels: 10.5 uses 10/11 slots.
When a new target or lease renewal replaces a pattern, the error accumulator
accounts only for elapsed slots, including partially played slots. It does not
count future samples that the toy never played. Every sample stays at or below
the **floor of the independent ceiling**. For example, a 53% ceiling forbids level
11 even if the requested average is 10.5; the backend then uses level 10. A ceiling
change is forwarded even when the target stays unchanged.

Identical targets/patterns are suppressed until a 500 ms lease renewal is needed.
Stable integers use ordinary finite Vibrate commands; stable fractions use one
locally executed pattern plus renewals. Stop, selection, disconnect, errors and
shutdown reset shaping history. Unsupported Pattern responses (documented codes
400/403) trigger one Stop + direct-Vibrate fallback per connection, shown in the
UI. Malformed replies, invalid parameters (404) and network/HTTP errors fail
closed rather than guessing compatibility. Reconnect permits a new Pattern try.

Devices / Settings offers three saved modes. Change mode, click Connect, reselect
the toy, then Resume; all existing connection/Stop rules remain. Compare using
the same effect settings and global intensity:

| Mode | Two-second 10→13 ramp: signed mean error | Mean absolute error | Positive requests |
| --- | ---: | ---: | ---: |
| Direct Vibrate (`vibrate`) | -0.516 levels | 0.516 levels | 6 |
| Pattern smoothing (`pattern`) | -0.536 levels | 0.536 levels | 7 |
| Pattern smoothing + dithering (`pattern_dither`, default) | -0.056 levels | 0.306 levels | 50 |

These are deterministic **digital output simulation** results with targets at
25 Hz, output sampled at 5 ms, and no network/motor latency. They demonstrate
reduced quantization bias, not measured physical smoothness. Dithering can send
more requests during a ramp; it still obeys the existing 25 Hz cap, and stable
output requires only renewals. Pattern alone does not improve level resolution.
Run the comparison again with:

```sh
cargo test -p race2love-lovense compare_direct_pattern -- --nocapture
```

Actual feel, flutter and latency depend on the toy/Remote and need a physical A/B
comparison. `Acknowledged device target` is the input accepted by the backend,
not motor readback; native rounding/ceilings can also lower its achievable average.
Existing TOML files load without migration;
the absent mode defaults to `pattern_dither`. Select `vibrate` for the previous
output strategy.

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
