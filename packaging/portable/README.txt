Relay: your phone and PC, kept together.

Start: double-click Relay.cmd. Everything Relay keeps (pairing, settings,
log) lives in the data folder next to it, so you can move or delete the
whole folder. Keep the folder on a local drive.

Pair the phone from the tray icon (Pair a phone). The phone app is the
Relay app.

Windows may ask once to allow Relay.exe through the firewall: choose the
private network. Using the phone as a camera asks once for administrator
permission to install the camera.

relay-cli.exe is the command-line tool. Run it with --dir "<this folder>\data\cli"
so it keeps its own pairing inside this folder (otherwise it uses
%LOCALAPPDATA%\PhoneBridge\cli).
