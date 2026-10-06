# Native Le Mans Ultimate telemetry

Phases 3–4 implement Windows and Linux/Proton acquisition in `race2love-lmu`.
Both platforms share the safe decoder and freshness logic. No SimHub, third-party
game DLL, bridge or elevation is required.

## Use

1. Enable LMU **Settings → Gameplay → Enable Plugins**, then restart the game.
   This switch also governs its native shared-memory interface.
2. Select **Le Mans Ultimate** on Dashboard, or launch `race2love --lmu`
   (`race2love.exe --lmu` on Windows).
   Default launch remains Demo with mock output.
3. Enter the player's car in a driving session. Dashboard shows session/track,
   car, speed, RPM/max RPM, gear, throttle and brake when simulation time advances.
4. Connect Lovense and explicitly select a toy. Source and device changes latch
   Stop; click **Resume output** when ready. RPM/gear effects use normalized frames.

Missing-game discovery retries once per second; identical errors are not logged
on every attempt. Monitor/non-realtime/vehicle exit clear old output while keeping
the game connection available and polling at 1 Hz. Resuming a session can take up
to two polls to establish advancing samples. Pausing releases handles. Run both
programs as the same normal user (and the same desktop session on Windows). Access
errors appear on Dashboard; elevation is not part of the intended setup.
On Linux, launch LMU normally through Steam/Proton as the same user. Race2Love
needs no Steam-library or Proton-path setting. Linux source selection uses direct
telemetry and does not silently substitute Demo.

For read-only diagnostics without a window or device connection:

```sh
cargo run --locked -p race2love-lmu --example telemetry_probe -- 30
```

The bounded probe runs for the supplied number of seconds (1–600), reports fresh
frames and normalized values once per second, and disconnects on completion. It
uses the same native adapter and never creates a haptic backend or HTTP client.

## Contract and values

The target is native x64 `LMU_Data`, described by the game-shipped
`Support/SharedMemoryInterface/SharedMemoryInterface.hpp` and `InternalsPlugin.hpp`.
Vehicle records use four-byte packing; outer wrappers introduce padding.
The payload ends at 324820 bytes; the reported SDK allocation is 324824 bytes.
This is different from legacy `$rFactor2SMMP_*$` mappings and their version counters.

| Field | Byte offset | Interpretation |
| --- | ---: | --- |
| `generic.gameVersion` | 64 | Little-endian i32 game marker |
| Scoring info | 1632 | Track at +0; session kind +64; `mInRealtime` +115 |
| Telemetry header | 128464 | Active vehicles, player index, player-has-vehicle: three u8, then padding |
| Vehicle array | 128468 | 104 slots, 1888 bytes each |
| `mID` | Vehicle +0 | i32 identity, used with the clock for freshness |
| `mElapsedTime` | Vehicle +12 | f64 simulation seconds |
| `mVehicleName` | Vehicle +32 | Bounded 64-byte C string |
| `mLocalVel` | Vehicle +184 | Three f64 components, m/s; speed is vector magnitude |
| `mGear` | Vehicle +352 | i32: reverse -1, neutral 0, forward positive |
| `mEngineRPM` | Vehicle +356 | f64 RPM |
| `mUnfilteredThrottle` | Vehicle +388 | f64 fraction 0–1 |
| `mUnfilteredBrake` | Vehicle +396 | f64 fraction 0–1 |
| `mEngineMaxRPM` | Vehicle +532 | f64 RPM; zero yields no engine effect |

Session codes: test day 0, practice 1–4, qualifying 5–8, warmup 9, race 10–13;
unknown codes display “Session”. Strings use bounded, lossy decoding if needed.
Speed is unsigned. Controls are unfiltered driver inputs rather than necessarily
the assisted controls applied by the simulation.

The decoder validates payload length, game-marker family 1.2–1.4
(`12000 <= gameVersion < 15000`), count/index, boolean flags, gear and finite/ranged
numbers. It selects the explicit player slot. Unknown game families and malformed
snapshots fail closed. `gameVersion` is not an ABI version: these checks cannot
prove a patch preserved every offset. Compare the shipped SDK after game updates.
Optional slip, suspension, vertical acceleration and impact values remain `None`
for LMU until separately verified in Phase 6.

## Access and safety

`windows.rs` opens existing `LMU_Data` with `FILE_MAP_READ` and an explicit bounded
payload length. Insufficient backing storage is rejected by Win32; page-rounded
view sizes cannot distinguish four bytes of padding from larger allocations.

The newly installed SDK also requires ordered update gates: check
`LMU_Data_HoldEvent`, then `LMU_Data_DataEvent`, then acquire the lock. Both event
checks use zero-timeout polls. A closed gate skips the sample; a notification is
never consumed before Hold permits access. Gate handles require only SYNCHRONIZE
rights and are never created/signaled by Race2Love. Builds predating these gate
objects report an availability error; the decoder's 1.2–1.4 marker range does not
imply that older Windows producers support this acquisition protocol.

Snapshots use existing SDK objects `LMU_SharedMemoryLockData` (eight bytes) and
`LMU_SharedMemoryLockEvent` (auto-reset wake event). Only synchronization memory is
writable: i32 waiter count +0, interlocked busy flag +4. One compare/exchange tries
acquisition. A busy lock skips the tick, without spinning, waiting or registering
a waiter. An RAII guard releases the busy flag and signals waiting SDK writers.
Only the copy into a reused owned buffer occurs under the lock. Missing lock
objects are an error; no unsynchronized fallback or production-object creation.

Toolhelp locates `Le Mans Ultimate.exe` and opens only a `PROCESS_SYNCHRONIZE`
handle, checked without waiting before copies. Game exit disconnects even when
another reader retains old mappings. RAII releases handles/views on failure,
disconnect and shutdown. Unsafe code is isolated to this module. Discovery/reads
run in a dedicated Tokio blocking worker, separate from effects, HTTP and GUI.

### Linux / Proton access

`linux/proc.rs` enumerates numeric `/proc` entries owned by the current user.
It matches executable arguments exactly (`Le Mans Ultimate.exe` or
`LeMansUltimate.exe`, including Wine loader arguments), avoiding substring matches
against unrelated command lines. `PluginsAdapter.exe` is eligible only with the
same nonempty `WINEPREFIX` or a direct game-parent relationship. Environment reads
retain only the prefix and never log other values. Multiple game instances are
rejected until the user closes the extra instance.

The reader opens existing `/proc/<owner-pid>/fd/<n>` **read-only**. Wine backing
names include `/memfd:wine-mapping (deleted)` and the older/fallback deleted
`tmpmap-xxxxxxxx` files. Names do not reveal the Windows mapping name: every
candidate must have the SDK allocation size (324824 bytes), or up to its 4 KiB
page-rounded size (327680), and pass the shared decoder at container offset zero.
Unrelated, truncated, oversized or unknown-layout files are rejected. This reader
supports the complete native container, not arbitrary telemetry fragments or
large memory scans. Identical device/inode candidates are deduplicated.

If several valid live-player copies exist, discovery observes simulation-clock
progress for at most 150 ms and prefers an advancing copy. Constant RPM is valid.
Menu/frozen mappings can connect but cannot generate fresh output. Discovery caps
process entries (32768), matching processes (32), descriptors per owner (4096),
and accepted candidates (16); it runs on the separate telemetry worker. Retry
remains once per second. Connected polling uses the configured telemetry rate.

Every read checks game/owner start time and zombie state, then fd device/inode and
size, both before and after copying. PID reuse, game/adapter exit, descriptor
closure/replacement and truncation disconnect and rediscover. An owned file
keeping old data alive is insufficient to keep the source connected. `pread`
(`FileExt::read_exact_at`) avoids mmap's possible SIGBUS on concurrent truncation;
short reads become recoverable errors. Linux adds no unsafe code or runtime crate.

**Synchronization limitation:** Wine's named SDK lock and gate objects are not
identifiable through the anonymous data descriptor. Race2Love neither guesses a
lock file nor modifies the producer's memory. It reads version, scoring prefix,
telemetry header and the selected vehicle prefix twice and accepts only identical
results. A change skips the tick without waiting or retrying in a tight loop.
The event queue is not a monotonic version counter; rapidly changing FFB and unused
vehicles are excluded. Repeated reads reduce torn snapshots but cannot prove atomic
transactions if a writer pauses partway through an update. Basic fields still
undergo finite/range checks and the shared clock watchdog. Stronger guarantees
would require a verified Wine synchronization interface or producer sequence lock;
this is a documented platform constraint, not an SDK-lock claim.

Access may fail under `hidepid`, ptrace/dumpability restrictions, a different user,
or Steam/app sandbox boundaries. Wine versions can change their backing names or
keep no usable descriptor in the game/associated adapter; those configurations
report an availability error and stay stopped. There is no root, `/proc/PID/mem`,
Wine-server protocol or external bridge fallback. Use the normal desktop user and
an installation where Race2Love can see the game's `/proc` entries. A game update
outside the supported layout requires new SDK verification.

Every connection requires two different observed simulation timestamps before
publishing. Duplicate `(vehicle ID, elapsed time)` returns no fresh frame; reopening
frozen memory cannot renew output. After two seconds without progress, mappings
close and discovery restarts. Independently, the core stops output at its sample
timeout (default 250 ms), including lock contention. Missing live player/realtime
flags clear output immediately. Source generations travel through controls,
telemetry and effects so old-source values cannot cross a switch, even with an
immediate Resume. Manual Test remains an explicit one-second telemetry exception.

One 324820-byte owned snapshot buffer is retained without a frame queue. Windows
copies that buffer under the SDK lock (about 19.5 MB/s at 60 Hz). Linux reads only
used prefixes twice, at most 1326 bytes per tick (about 80 KB/s at 60 Hz), and
writes them into the shared-layout buffer. Adding decoded fields requires extending
those prefixes/consistency checks. CPU and end-to-end latency need live measurement.

## Sources and verification

Consulted 2026-10-06. References are not dependencies. Protocol facts were
cross-checked; the decoder and nonblocking lock consumer were independently
implemented. No reference code, proprietary SDK header or real game snapshot is
redistributed, and no GPL code was incorporated.

- [TinyPedal/pyLMUSharedMemory](https://github.com/TinyPedal/pyLMUSharedMemory/tree/6cef58e22bf025268acee93e9e5ea4f569939c1b),
  MIT: SDK translation, event count and container/vehicle fields.
- [Swizzjack/lmu-pitwall native layout](https://github.com/Swizzjack/lmu-pitwall/blob/a171b80a7c8c1e02a9d029e32fd717c834d26e71/bridge/src/shared_memory/lmu_data.rs),
  MIT: section sizes/offsets and reported LMU 1.4 probe results.
- [apex-lmu native contract](https://github.com/ralfboltshauser/apex-lmu/blob/3d24d89e5790979dab6ec9558a5bcada0703d560/bridge/lmu_contract.go)
  and [synchronization reference](https://github.com/ralfboltshauser/apex-lmu/blob/3d24d89e5790979dab6ec9558a5bcada0703d560/bridge/lmu_memory_windows.go),
  GPL-3.0-or-later: outer padding, SDK lock objects/protocol and process lifetime.
- [fpauker/lmu-rpm-leds](https://github.com/fpauker/lmu-rpm-leds/tree/e0802f16a88a23731d8402f884e1e02f62fd6c0a),
  GPL-3.0-or-later: independent RPM/gear/elapsed offsets and Proton discovery research.
- [Wine mapping implementation](https://github.com/wine-mirror/wine/blob/master/server/mapping.c),
  LGPL-2.1-or-later: anonymous `wine-mapping` memfds, fallback `tmpmap-*` files and
  page rounding; [Proton 10 Wine source](https://github.com/ValveSoftware/wine/blob/proton_10.0/server/mapping.c)
  also confirms the older deleted-file backing. These are implementation details,
  not a stable public Wine ABI.
- [Linux /proc documentation](https://docs.kernel.org/filesystems/proc.html):
  process lifetime, descriptors, start times and access restrictions.
- [Bytecode Alliance rustix](https://github.com/bytecodealliance/rustix),
  Apache-2.0/MIT with LLVM exception: maintained safe POSIX bindings, added only as
  a Linux test dependency for real memfd fixtures. Existing lockfile version 1.1.5
  is reused; production Linux acquisition has no added dependency.
- [Microsoft MapViewOfFile](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-mapviewoffile)
  and [WaitForSingleObject](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitforsingleobject):
  rights, bounded views and nonblocking lifetime checks.
- [Microsoft windows-rs releases](https://github.com/microsoft/windows-rs/releases):
  maintained `windows` 0.62.2, Windows-only with the required Win32 features.

Synthetic fixtures test offsets, units, player selection, malformed bytes/numbers,
versions, missing player/realtime, progress, freeze, player exit, game restart and
shutdown through the actual pipeline. Windows-only fixtures create isolated named
sections/events and a disposable process: read-only protection, contention and
wakeup, missing/undersized mappings and process exit with a retained mapping.

Linux acquisition has process/filesystem fixtures and a real `/proc` memfd test,
plus full-pipeline freeze, player exit, process loss, reconnect and shutdown checks.
The real Linux/Proton game (marker 14200) was checked in a driving session through
the dashboard and read-only probe, at about 60 fresh frames/s. The owner also
confirmed live Lovense output after Resume. Broader physical fault/latency
acceptance remains; see the detailed validation record.
Windows code/tests compile and pass Clippy from Linux. Executing Windows fixtures
and testing LMU on Windows remain native Windows acceptance work. The installed SDK
was verified once the owner's installation finished; all used offsets and sizes
passed C++ static assertions targeting the Windows x64 ABI. See
[VALIDATION.md](VALIDATION.md) for exact status. Live acceptance: compare headers,
check dashboard values against the game, pause/exit the car, close/restart LMU and
Race2Love, and confirm every stop with the previously tested Lovense setup.

The SDK is proprietary and permits extension development, not header redistribution
or modification. It remains only in the game installation. The optional
[layout verifier](../tools/verify-lmu-layout.cpp) includes those local headers.
From an **x64 Native Tools Command Prompt** on Windows:

```bat
cl /nologo /std:c++17 /EHsc /Zs /I"C:\path\to\Le Mans Ultimate\Support\SharedMemoryInterface" tools\verify-lmu-layout.cpp
```

The Rust build does not require these headers, a game installation or C++ on Linux.
