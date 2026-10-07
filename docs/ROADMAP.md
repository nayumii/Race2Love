# Race2Love roadmap

## Phases 1–6: Working telemetry and effects

Implemented native Demo mode, Lovense local communication, Windows and Proton
LMU access, RPM/shift feedback, optional slip/road/impact effects, bounded live
graphs and named effect profiles. Hardware feel, track-specific sensitivity and
fault acceptance continue to benefit from real driving tests.

## Phase 7: Polished native UI

Implemented a consistent dark cockpit theme with mint/violet accents, clear
feedback output/activity cards, an actual RPM-response preview,
grouped effect/device controls and a native vector brand mark. Wide windows use
sidebar navigation; smaller windows use tabs and stacked cards. Stop/Resume
stay visible above scrolling content, and Esc still stops output on every page.
Existing controls, configuration and background workers remain in place.

## Phase 8: Release readiness

Implemented reproducible portable packages for Linux x86_64, Windows GNU
cross-builds, and native Windows MSVC builds. `tools/build-release.py` embeds
build metadata, checksums, the PolyForm license, attribution and dependency
notices. `tools/verify-release.py` validates archive contents, executable
identity, hashes and optional Demo smoke runs. CI runs the packaging contract
tests and publishes verified artifacts for version tags. The installation guide
covers configuration paths, Lovense setup, upgrades and troubleshooting.

Remaining acceptance work is environment-specific: run the tagged workflow on a
native Windows runner, review high-DPI and Wayland behavior, and publish only
after the generated artifacts have passed the project's release checklist.

The everyday UI prioritizes feedback and device control. Live driving telemetry,
raw-signal graphs, intermediate output targets and technical runtime settings
are available through Devices / Settings → Debug mode. Connection options
contain transport and experimental output controls.
