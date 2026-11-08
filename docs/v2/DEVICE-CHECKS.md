# Device checks (things tests cannot prove)

Each line says what to look at, on which setup, and which task added it. Tests prove the code paths and the fallbacks; these checks prove how it looks and feels on a real screen. Tick a line with the date and the Windows or Android build you used.

## PC (Windows 11 22H2 or later, transparency effects on)

- [ ] **M5.1 Mica (Native look).** Settings → Appearance → Look → Native glass. The main window's title bar and navigation rail show the desktop wallpaper through, softly blurred. Text on the rail stays readable over a white wallpaper and over a black one.
- [ ] **M5.1 Acrylic (Native look).** Open the tray flyout over a bright and a dark window. The flyout is blurred glass, its tiles are legible, and the edge is a thin line.
- [ ] **M5.1 Relay glass.** Settings → Appearance → Look → Relay glass. The main window and the flyout are tinted (cooler than Native in light mode), with a faint grain and a 1 px lighter line along the top edge. Text is as readable as in Native.
- [ ] **M5.1 Toggle without restart.** With a window open, turn Windows "Transparency effects" off and on (Settings → Personalization → Colors). Both windows go solid and back to glass at once, keep their page and scroll position, and neither blinks to a new window.
- [ ] **M5.1 High contrast.** Turn on a Windows High contrast theme. Both windows use the system colours with no glass, and every control keeps a visible boundary. Turn it off: glass returns.
- [ ] **M5.1 Remote Desktop.** Over a Remote Desktop session the windows are solid and readable.
- [ ] **M5.1 Text.** Over glass the text has no coloured fringes; on solid cards it is crisp (ClearType). Look at 100%, 125% and 150% display scaling.

## Unlock gate (M7)

- [ ] **M7.1 Phone prompt.** Settings, Security, "Ask for unlock before sensitive actions" on, with a screen lock set. Ask the PC for the screen: tapping Allow shows the fingerprint or screen-lock prompt, passing it allows, cancelling it leaves the request unanswered (no allow). Tap Allow again within a minute of a pass: no second prompt. After a minute, or after locking the screen, it asks again.
- [ ] **M7.1 No screen lock.** With the screen lock removed, the Security row says "Set a screen lock first" and stays off; a request is still answered with the normal consent dialog.
- [ ] **M7.1 Turning it off.** Turning the switch off asks for one unlock. Remove PC and "Accept from my devices automatically" (turning it on) ask too.
- [ ] **M7.2 PC prompt (Windows 11 with Hello set up).** Settings, General, "Ask for Windows Hello before sensitive actions" on. Accept an incoming file: the Windows Hello prompt appears over the dialog; passing saves the file, cancelling declines it. Within a minute a second accept does not ask again.
- [ ] **M7.2 PC without Hello, Windows 10 and Remote Desktop.** The row is greyed with its reason ("Set up Windows Hello first", "Needs Windows 11", "Not available over Remote Desktop"); consent dialogs still work.
- [ ] **M7.2 PC always-allow.** "Let my phone control this PC" on, "Play PC audio without asking" on and "Accept files automatically" on each ask for Hello first; turning them off, or turning the gate on, does not.
- [ ] **M7.4 Disconnect everything.** The tray menu entry and the phone's Settings row close every connection, stop every share, turn every "always allow" off and pause Relay until it is turned on again.

## Several devices (M8)

Setup: the phone paired with two PCs (the tray, and a second identity from `pb.exe --dir <d2> --port 47201`).

- [ ] **M8.5 Upgrade keeps the answers.** Install the build from before the per-PC change, turn on a global "always allow" and "accept automatically", then install the current build over it. Each paired PC keeps those answers (Settings, Connection, the PC's permissions); a PC paired afterwards starts at Ask.
- [x] **M8.2 Names and default.** Rename a PC on the phone, Make default, remove the default PC: the default moves to the next one and the status line names it. *(2026-11-08, Android debug build from `5e3bb46`, Nothing/realme RMX3561: two PCs with the same name show as "(DB78)" and "(1217)" with a Default marker; Make default moved the default and back; removing a PC left no stored settings for it and the default stayed. Not run: Rename)*
- [ ] **M8.1 Share sheet.** Share text to Relay with two PCs: it goes to the default, and "Change PC" sends it to the other.
- [x] **M8.4 Never (screen, audio).** Set a PC to Never for screen, camera, control and audio. Each request from that PC is refused without a dialog, with at most one "Request blocked" note a minute. Also after the PC resumes a screen or camera share inside its grace. *(2026-11-08, Android debug build from `26bffc5`, RMX3561, v2 tray with its own folder: at Ask the screen request showed the consent prompt; at Never the screen and audio requests were refused with no prompt, the PC log says "declined", and one "Request blocked" note appeared. Not run: camera (the test tray refused to install its camera driver because of its temp folder), control (no PC command for it), grace resume)*
- [ ] **M8.4 Always while locked.** With Always set and the phone locked, the request still asks.
- [x] **M8.4 Per-PC switches.** Clipboard, notifications and automatic accept on for one PC only: only that PC receives clips and notification mirrors, and only its files are accepted unasked. *(2026-11-08, Android debug build from `5e3bb46`, Nothing/realme RMX3561: only the new-PC defaults: a PC paired later has no Always entries and no per-PC switch on; its one-shot clipboard send arrived as a notice. Live clipboard sync and notification mirror per PC not run)*
- [ ] **M7 Gate on new switches.** With the unlock gate on, turning on "Share clipboard" or "Share this phone's audio without asking" asks for an unlock; turning them off does not.
- [ ] **M8.3 Tile and widget.** The Send tile and widget name the default PC and send to it.
- [ ] **PC Send to.** With "Send to" set to Ask and two phones, the tray menu lists both and the choice is used once.
- [x] **Finding fixed (2026-11-08, `bb2f333`):** the "Remove <PC>?" dialog showed only the bare name, so two PCs with one name could not be told apart. It now reads "Remove ASUS-ROG-STRIX- (D5C4)?", checked on the phone. Rename and permissions dialogs use the same label (not opened on the phone).
- [x] **Status after a phone-started send (2026-11-08, `5e3bb46`).** After a notification reached the PC, Home says "Last seen just now" (it said "33 min ago" before the fix).
