# Effect signals and tuning

Additional effects default **off** to preserve existing profiles. Enable them on
Effects; valid changes apply immediately. Global intensity and maximum output
still apply after the mixer. Stop, stale telemetry, disconnect and shutdown use
the existing safety path. Direct Vibrate remains the default Lovense mode.

## LMU signal contract

Offsets below are relative to the 1888-byte player telemetry record unless
specified as wheel-relative. Wheels start at +848, stride 260, FL/FR/RL/RR.
These definitions come from LMU's shipped `Support/SharedMemoryInterface` SDK;
no SDK code/header is redistributed. `tools/verify-lmu-layout.cpp` provides
Windows x64 static assertions. Both operating systems use this same decoder.

| Effect/data | SDK field and offset | Conversion / limitation |
| --- | --- | --- |
| Slip | Wheel `mGripFract` +112, `mTireLoad` +104 | Maximum loaded-wheel sliding contact-patch fraction, 0–1. Not longitudinal slip ratio. Loads below 50 N contribute zero. |
| Suspension speed | Wheel `mSuspensionDeflection` +0 | Difference in metres divided by **game elapsed time**. Requires two valid samples, 1/240–0.25 s apart; reject speeds beyond ±20 m/s. Reset across player/session changes. |
| Vertical vibration | `mLocalAccel.y` +216 | Body-local m/s², minus a 200 ms low-pass baseline; absolute residual. No assumption about removal of gravity. Initialize/reset baseline without an onset pulse. |
| Explicit kerb contact | Wheel `mSurfaceType` +176 | Code 5 is rumble strip; require at least 50 N tyre load. Some tracks leave all four flags false even over kerbs. |
| Terrain debug | Wheel `mTerrainName` +160, 16 bytes | Bounded material label, displayed only. No heuristic matching or inferred kerb contact. |
| Impact | `mLastImpactET` +552 and `mLocalAccel` +208 | New positive impact timestamp within 250 ms of game time. Severity estimate = acceleration-vector magnitude / 100 m/s², clamped 0–1. SDK impact-magnitude units are unspecified, so that field is unused. |

Invalid optional numbers disable the affected signal. The impact identity is
retained to prevent replay; the initial retained event on connect is ignored.
Braking/vertical acceleration alone cannot generate collision pulses. Collision
severity is an estimate at the sampled instant and may miss a brief peak.

## Road feedback when rumble-strip flags stay false

Road output takes the maximum of three candidates, capped by Road maximum:

- Suspension: `max(abs(travel speed) - threshold_mps, 0) * gain`.
- Vertical vibration: `max(filtered acceleration - acceleration_threshold_mps2, 0) * acceleration_gain`.
- Explicit loaded-wheel contact: `kerb_intensity` while any rumble-strip flag is true.

This is **road vibration**, not reliable material classification. Grass, bumps,
landings and kerbs can all excite it; no signal can identify a flat kerb that the
game neither labels nor physically represents. Slip and road activate above
3 m/s. Road uses a 10 ms attack and 80 ms release to retain short events between
output updates; vertical filtering runs only on new telemetry, without lookahead
or a queued delay. Existing output rate/leases remain unchanged.

For tuning, enable Kerbs / road, temporarily disable engine/slip/impact effects,
and compare smooth tarmac against kerbs at similar speed. Default vertical
threshold is 1 m/s² and gain 0.12; lower the threshold or increase gain if the
Road meter is weak on kerbs. If ordinary driving triggers too much, raise the
threshold. Adjust suspension gain separately if grass overwhelms the cue. These
are starting defaults, not hardware-calibrated values. Keep enough global/device
headroom to feel an additional cue over the engine background.

Enable **Devices / Settings → Debug mode** for speed, RPM, gear, pedal inputs,
session/car details, shift detection, rumble flags and raw signals. Normal
Dashboard shows mixer output, device output, effect activity and output limits.
**Show output history** plots mixer output; Debug mode adds RPM, sliding fraction,
suspension speed and vertical acceleration graphs. History is at most 20 seconds / 400 samples and
clears when disabled or switching source; unavailable telemetry creates gaps.

## Shift and impact envelopes

Forward adjacent shifts may pass through neutral for up to 250 ms. Long neutral,
reverse, skipped gears, or stale sample gaps do not count. Dashboard shows a
500 ms shift indication, total detected count and individual pulse meter. A
counter increment confirms detection even when engine output/device quantization
makes a short pulse hard to feel.

Impact pulses use 5 ms attack, 80 ms hold and 180 ms release, with a 300 ms
cooldown and one pulse per event. Engine/slip/road combine at priority 20;
shift priority is 160 and collision priority 220. Every result remains bounded.

## Profiles

Effects → Effect profiles can store, apply and delete up to 16 named configurations.
Profiles contain effect settings only; device settings and global safety limits
remain independent. Use the existing Save configuration action to persist them
in TOML. Older configurations receive defaults for missing fields; explicit
choices are retained. New effects are also usable with synthetic Demo telemetry.
