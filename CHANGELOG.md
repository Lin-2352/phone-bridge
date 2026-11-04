# Changelog

## 0.2.0 (unreleased)

### Added
- Audio sharing in both directions: hear the phone on the PC, or the PC on the phone, including from the tray's quick panel.
- While sharing, the phone stays silent and its real volume bar sets the level on the PC. Your own volume comes back when you stop.
- Redesigned Windows and Android interfaces, with a quick panel on the PC and a widget and quick tile on the phone.
- Clipboard pictures.

### Changed
- The portable PC build no longer needs a launcher script or the Visual C++ runtime: run `Relay.exe`, and everything stays in the `data` folder beside it, logs included.

### Fixed
- A paired PC could send a "link" such as `upi:` or `tel:` that opened on one tap. Only `http` and `https` addresses open now; anything else arrives as plain text.
- Removing a PC left its "always allow" for control and screen in place, so pairing it again skipped the question.
- The Allow button for PC control can no longer be used from a locked screen.
- Android's share sheet could be pointed at Relay's own files with a `0@` address.
- Home described a microphone share as "what this phone plays".
- Sharing notifications stayed up after turning Relay off.
- A forced stop mid-share left the phone's volume wrong until Relay next started.
- Restart in the tray could fail to come back when the install path has spaces.
- Notifications showed as working in Settings even when Android hadn't connected the listener.

### Known issues
See the [README](README.md#known-issues). Also not fixed yet:
- If the PC's audio device disappears during a share, the session keeps showing as running with no sound.
- Settings shows *Send to* and *Start with Windows* as on even after the folder moves. Turn them off and on to repair them.
- The single-instance pipe doesn't check who owns it, so on a PC shared between users, another local user could receive items sent to Relay.
- If the tray's core fails to start, *Send to* can silently lose files while the tray is up.
- A paired PC that floods screen or camera control messages could exhaust the phone's memory.
- The quick panel can't be read by Narrator.
- If a PC closes at the moment you accept an audio request, capture can keep running with nothing being sent.
- If saving fails when re-pairing a PC, the host drops the working pairing while the other side keeps it.
- If the joining device dies right after the host confirms, pairing can end up on one side only.
- After the host declines a pairing, the other device can keep waiting for up to 2 minutes.
- Anyone on the same network can use up pairing attempts by sending bad codes.

## 0.1.0

First version: pairing, clipboard, file transfer, notifications, phone screen and control, and the phone as a webcam.
