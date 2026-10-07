Race2Love - Linux x86-64 portable build

Extract this archive and run ./race2love from its folder.
No Rust compiler is needed on the test PC. This build uses the build machine's
glibc and requires a desktop with X11 or Wayland and working OpenGL drivers.
For older Linux distributions, build on the oldest distribution you support.

The app starts with Demo telemetry and mock output.
For Le Mans Ultimate under Proton, enable Plugins in LMU and restart the game,
then select Le Mans Ultimate on Dashboard or launch ./race2love --lmu.
Enter the car in a driving session to enable feedback.
Speed/RPM/gear appear only with Devices / Settings > Debug mode enabled.

For Lovense output, enable Allow Control/Game Mode LAN in Lovense Remote.
Enter Remote's host and port in Devices / Settings (protocol is in Connection options), click Connect,
select your toy, then click Resume output in the top bar.
After changing source or device, click Resume output again. Esc stops output.

Settings default to ~/.config/race2love/config.toml (or $XDG_CONFIG_HOME).
config.example.toml is a reference and is not loaded automatically.
For a display-free mock test: ./race2love --demo-seconds 5
To collect a log: RUST_LOG=debug ./race2love > race2love.log 2>&1

See INSTALL.md for upgrades, checksums, troubleshooting and removal.
These builds are unsigned. LICENSE states the project licensing status.
