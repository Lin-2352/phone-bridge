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
