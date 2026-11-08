# Changelog

## 0.3.0 (not released yet)

### Added
- **Files that stop part-way continue.** If the link drops during a transfer, the receiving device keeps what arrived (up to a day, within a size limit) and the sender sends only the rest. The PC retries a dropped link by itself; on the phone the *Retry* button on the notification continues the transfer. Relay 0.2.0 devices still work; they just start over.
- **Send queue.** Files wait in line per device (up to 64). *Pause queue* on the PC's Transfers page holds the next file without stopping the one sending; waiting files are listed with a *Remove* button, and after a restart they come back paused.
- **Drag and drop.** Drop files on the PC's quick panel, main window or screen window; with several phones paired Relay asks which one. On a tablet, foldable or other wide window, drop files from another app onto Relay to send them to the default PC.
- **The unlock gate and *Disconnect everything*.** Sensitive switches ask for your fingerprint or Windows Hello first; one action closes every connection and turns every "always allow" off.
- **Per-device permissions.** Each paired device has its own "always allow" answers, a *Never* choice that refuses without asking, nicknames, and a default device; sends ask which device when several are paired.
- **Keyed discovery.** Paired devices find each other without anyone else on the network being able to tell them apart or plant an address.
- **A Windows installer** (per user; an administrator install also adds a firewall rule for Private networks only) next to the portable zip, and a Windows on ARM build that has not been run.
- **Release integrity tooling.** A signed release manifest can be verified (rollback, replay and key replacement are refused) and a signing step for Windows files is in the build. Neither is switched on: no public key or certificate is set up yet.
- Redesigned PC windows and a Transfers screen on the phone.

### Changed
- The wire protocol's draft label is gone; new fields are optional, so 0.2.0 devices ignore them.

### Not done yet
- Clipboard history is built (encrypted, opt-in, seven days) but has no switch or list on screen.
- The update check has no button yet, and the app cannot verify a manifest until the owner's public key is added.
- The phone does not retry a dropped link by itself. The PC's *Retry* button on a failed row sends outside the queue.
- Most of this version has been tested by automatic tests only. The checklist of what still needs a person with a phone is `docs/DEVICE-TEST-CHECKLIST.md`.

### Added (since 0.2.0, earlier work)
- A PC asking for control while *Relay control* is off now sends you to Accessibility settings, or posts a notification that does if Android won't let Relay open them from the background. The request waits up to 40 seconds for you to turn it on, instead of being refused on the spot.

- *Silence earphones too*, on by default. Turned off, connected earphones keep playing during a share, and the audio also goes to the PC.

### Fixed
- Notification access and PC control: on Android 13 and later, an APK from outside Google Play can find these switches locked. *Grant* and *Fix* now explain how to unlock them and open the right pages, instead of leaving you at a greyed-out switch.
- Turning Relay off and on again could leave it unable to start until the phone killed it. A finished camera, screen, audio or touchpad session kept the network port busy. Those sessions now let go of it, a failed start is retried, and if it still fails you're told and the reason is logged.
- The status said "<PC> not reachable" whenever no session was open, even right after a transfer worked. It now says that only after an attempt really failed.
- Sound could be heard from the phone during an audio share: games stayed loud, because Android applies the silencing effect to only one audio output, and opening one played sound before the phone went quiet. The phone is now muted from the start of a share.
- The volume keys now set the PC's level in every app. Apps that keep the keys for themselves, like YouTube and Instagram, get the normal volume bar, already at the PC's level. Games on the low-latency output get a cast-style bar.
- Audio lag: the PC held 170 to 340 ms of sound, and now holds what the network needs, usually about 100 to 120 ms.
- Audio crackling and skips: a bug in the audio library left playback at normal priority, so a busy PC could run the sound device dry. Small drifts are now played off by a change of speed too slight to hear, instead of a skip.
- An audio share could end by itself with "the app does not allow capture" when nothing was playing but an app held a silent player open, as YouTube's home screen does. Silence never ends a share now. If an app seems to be playing and no sound arrives for 10 seconds, the share's notification says so.
- A stray tap beside a screen, camera or control question no longer counts as Deny.
- Turning on "Let my PC control this phone" while Relay control was off also gave every paired PC permission to skip the question. It now only leads to the Accessibility page.
- The PC no longer says a phone "declined" control when Relay control was simply off.

### Known issues
- Play Protect can block installing the APK from a browser, messaging app or file manager in regions with enhanced fraud protection, including India, because Relay declares a notification listener and an accessibility service. No code change avoids this while both features stay in the APK. See [If Android blocks the app](README.md#if-android-blocks-the-app).

## 0.2.0 (2026-11-04, pre-release)

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
- A host on the same network that plants a false address for a paired device can make connections from the device being targeted fail, repeatedly if it answers first. It got through every time in tests on Windows and failed every time on Linux; Android hasn't been tested. The wrong device is still refused and never gets a session.

## 0.1.0

First version: pairing, clipboard, file transfer, notifications, phone screen and control, and the phone as a webcam.
