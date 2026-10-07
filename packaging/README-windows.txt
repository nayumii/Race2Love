Race2Love - Windows x86-64 portable build

For Windows 10/11 on a 64-bit Intel or AMD PC.
Extract the entire ZIP, then double-click race2love.exe.
No Rust compiler, Visual Studio or administrator access is needed to run it.

The app starts with Demo telemetry and mock output.
For Le Mans Ultimate, enable Settings > Gameplay > Enable Plugins in LMU and
restart the game. Select Le Mans Ultimate on Dashboard, or use Launch-LMU.cmd.
Enter the car in a driving session to enable feedback.
Speed/RPM/gear appear only with Devices / Settings > Debug mode enabled.

For Lovense output, enable Allow Control/Game Mode LAN in Lovense Remote.
Enter Remote's host and port in Devices / Settings (protocol is in Connection options), click Connect,
select your toy, then click Resume output in the top bar.
After changing source or device, click Resume output again. Esc stops output.

Settings are saved in %APPDATA%\race2love\config.toml.
config.example.toml is a reference and is not loaded automatically.
For troubleshooting, use Launch-with-logs.cmd and send back race2love.log.
From Command Prompt, Launch-with-logs.cmd --demo-seconds 5 runs a five-second
display-free Demo using mock output.

See INSTALL.md for upgrades, checksums, troubleshooting and removal.
These builds are unsigned. LICENSE states the project licensing status.
