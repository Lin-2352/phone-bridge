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

