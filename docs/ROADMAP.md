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

Next: finalize Linux/Windows packaging, automate release builds and artifacts,
and improve installation/setup documentation. Use the existing CI and packaging
work as a starting point. Native Windows/MSVC, high-DPI and Wayland acceptance,
plus the project's license selection, remain explicit release considerations.

The everyday UI prioritizes feedback and device control. Live driving telemetry,
raw-signal graphs, intermediate output targets and technical runtime settings
are available through Devices / Settings → Debug mode. Connection options
contain transport and experimental output controls.
