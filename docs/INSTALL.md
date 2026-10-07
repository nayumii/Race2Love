# Install and run Race2Love

Use an x86-64 package matching your operating system. Extract the whole archive
to a writable folder you own. No installer, administrator privileges, Rust,
SimHub or cloud account is required for Race2Love. Pairing a device still requires
Lovense Remote. Packages are unsigned; only run builds from a source you trust.
The included LICENSE states the project's licensing status.

## Linux

Extract `Race2Love-<version>-linux-x86_64.tar.gz` and run `./race2love`.
The executable needs an X11/Wayland desktop, working OpenGL drivers and the host's
normal display libraries. CI uses Ubuntu 22.04; locally built packages can require
a newer glibc. The build host/libc is recorded in `build-info.json`. A missing or
older `GLIBC_x.y` error means the package needs a newer system; build from source
on the target distribution rather than replacing system libraries.

Run Race2Love and Steam/Proton as the same user. Enable Plugins in LMU, restart
LMU, then select Le Mans Ultimate in Race2Love and enter the car. No fixed Steam
library path or Proton version is assumed. If process access is restricted by a
sandbox or `/proc` policy, see the repository's `docs/LMU.md` troubleshooting.

## Windows 10/11

Extract `Race2Love-<version>-windows-msvc-x86_64.zip` (native CI) or
`Race2Love-<version>-windows-x86_64.zip` (MinGW cross-build). Double-click
`race2love.exe`. `Launch-LMU.cmd` selects LMU immediately. Enable Plugins in LMU
and restart the game before driving. Both package variants run the same app.

MSVC builds may require the Microsoft Visual C++ x64 runtime if it is not already
installed. Obtain it from Microsoft's official support/download documentation;
do not download individual DLLs from third-party sites. The application is
unsigned and may prompt Windows reputation checks; verify the package source
and checksum before deciding whether to run it.

## Connect a device

1. Pair your device in Lovense Remote and enable Allow Control / Game Mode LAN.
2. In Race2Love's Devices / Settings, enter Remote's IP/host and displayed port.
   Protocol and the local HTTP preset are under Connection options.
3. Connect, explicitly select a device, then Resume output in the top bar for the
   initial session. A transient Remote/toy reconnect resumes automatically.
4. Use the one-second test or start driving. Start with a comfortable global
   intensity and maximum output. Emergency Stop stops feedback; its key can be
   configured as Esc, F8, F9, F10 or Disabled.

Race2Love starts with Demo and mock output, so opening it never auto-selects a
physical device. Testing pauses game input; enable it again when returning to
racing. Direct Vibrate is the tested default; experimental Pattern modes are
optional under Connection options.

Dashboard emphasizes mixer/device output and effect activity. Speed, RPM, gear,
raw graphs and technical settings are under Devices / Settings → Debug mode.

## Verify the download

Each artifact includes `SHA256SUMS.txt`. On Linux, from its directory:

```sh
sha256sum --check SHA256SUMS.txt
```

On Windows PowerShell, compare the result with the corresponding manifest line:

```powershell
Get-FileHash .\Race2Love-0.1.0-windows-msvc-x86_64.zip -Algorithm SHA256
```

A checksum detects corruption; it does not establish who built the file.
`build-info.json` records version, compiler, target, source commit, whether the
source checkout was dirty, and executable hash. Version numbers in examples
must be replaced with the downloaded version.

## Settings, upgrades and removal

- Linux: `$XDG_CONFIG_HOME/race2love/config.toml`, otherwise
  `~/.config/race2love/config.toml`.
- Windows: `%APPDATA%\race2love\config.toml`.
- `RACE2LOVE_CONFIG` can override the file path. The included
  `config.example.toml` is a reference, not automatically loaded.

To upgrade, close Race2Love normally, back up your configuration, and extract the
new build into a separate folder. Launch it and reconnect/select/resume your
device. Existing settings are retained; new fields receive defaults. If you need
to roll back, restore the matching config backup and launch the previous folder.
Do not replace files while the app is running. To uninstall, remove the extracted
folder and optionally the config directory. There is no background service or
system-wide installation to remove.

## Troubleshooting

- No vibration: check Resume, selected/connected device, enabled game input,
  global intensity, maximum output and the effect's enable toggle.
- Game waiting/frozen: enter a driving session, enable LMU Plugins and restart;
  use Debug mode for telemetry details.
- Remote connection fails: verify the LAN IP and Remote's displayed port. Check
  local firewall/Wi-Fi isolation and selected HTTP/HTTPS protocol.
- For a hardware-free test, run `./race2love --demo-seconds 5` on Linux, or
  `.\race2love.exe --demo-seconds 5` from PowerShell.
- Linux log: `RUST_LOG=debug ./race2love > race2love.log 2>&1`.
  Windows: `Launch-with-logs.cmd`; it creates `race2love.log` beside the executable.
  Review logs before sharing: they can include paths, LAN addresses and device IDs.
