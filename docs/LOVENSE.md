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
Demo to run engine/shift effects through the toy. Real LMU telemetry is still
planned. Disconnect stops the selected toy and restores mock output, stopped.

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
| Positive output | `Function`, `Vibrate:N`, `apiVer: 1`, explicit `toy`, `stopPrevious: 1` |
| Conversion | Clamp normalized output; round down to integer 0–20 |
| Lease | `timeSec: 2`; unchanged positive output renewed every 500 ms |
| Stop | Targeted `Function` with `action: Stop`, `timeSec: 0` |
| Request cap | Core output rate, default 25 Hz; sequential positive commands |
| Status polling | Every two seconds while connected; Stop preempts the wait |
| Request timeout | Configurable 100–5000 ms, default 1000 ms |
| Response limit | 64 KiB, including bodies without a Content-Length |
| Discovery limit | 128 toys; validated identities, statuses, and battery values |
| Queues | Latest settings/selection via watch; at most eight device requests |
| Connection retries | At most five, at 1/2/4/8/16 seconds; Connect starts a new attempt |

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
command expiry. Before declaring a particular Remote/toy version verified, check
GetToys, explicit selection, Test, emergency stop, disconnect/reconnect, normal
exit, and command expiry during LAN loss with that version. Record the platform,
Remote version and toy model; no physical device was available here.
