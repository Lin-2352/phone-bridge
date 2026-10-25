# UI polish (PC and phone), Quick Settings style flyout, and the v0.1.0 release

Status: draft for review. Date: 2026-10-26.

## Intent (what the user said, and what is assumed)

Said: polish the UI on the app and on the PC by grouping, making it sharp and smooth, not jagged; add a Windows entry like the Android quick tile (chosen: the tray flyout restyled like Windows 11 Quick Settings); scope "restyle plus new screens"; publish a GitHub release (Android APK + Relay portable zip), version `v0.1.0`, repository stays private, keystore created by me and kept by the user.

Assumed (user did not name them): the "new screens" are (1) a phone home screen of status cards and a tile grid and (2) a rebuilt PC settings window using the same groups as the flyout. Anything else is out of scope until named.

Fact: Windows does not let apps add tiles to its own Quick Settings panel; the tray flyout is the nearest honest equivalent and is already tile based.

## Success criteria

- PC text and shapes have no stair-stepped edges at 100%, 150% and 200% display scale; tiles animate hover and press without flicker.
- Related items sit in named groups on both platforms: Connection, Camera, Screen, Files, Settings.
- Nothing that works today stops working: the unit suites stay green and the earlier device checks still pass.
- A private GitHub release `v0.1.0` has `app-release.apk` (signed with the new key) and `Relay-portable.zip`.

## Design

### PC drawing (sharp and smooth)

The jagged edges come from GDI filling rounded rectangles without anti-aliasing. `fluent.rs` is the single drawing helper for the flyout, menu, status, settings and camera windows, so it is fixed once:
- Shapes (rounded rectangles, circles, toggles) are drawn with GDI+ in anti-aliased mode into the existing double buffer; text keeps ClearType through GDI.
- All sizes go through `px()` (DPI scaling) so edges land on whole pixels.
- Animation: one 16 ms timer per open window while any control is easing; hover and press are floats (0.0 to 1.0) eased toward their target; the timer stops when nothing moves. Pure easing function with unit tests.

### PC flyout (Quick Settings style)

Rows of tiles like the screenshot: each tile has an icon, a label and an on/off state (accent fill when on). Tiles: Phone camera, Phone screen, Clipboard sync, Send file, Pair device. Under the tiles a status line (peer name, link, last event). Groups are separated by thin dividers with small captions. Left click on the tray icon opens it; Esc, a second click on the icon or clicking away closes it (drag and drop of files onto it keeps working).

### PC windows

Camera, settings and status windows get the same spacing, type sizes and group captions. The settings window is rebuilt around the five groups; its option names and stored values do not change.

### Phone (Compose, Material 3)

Home screen of cards: a link card (phone to PC state), a tile grid (Camera, Screen, Clipboard, Send file) and a recent-items list; settings regrouped into the five groups with consistent spacing and shared-element-free, short transitions (animateContentSize and crossfade only). Behavior and preference keys do not change.

### Release

`scripts/make-portable.ps1` output is zipped as `Relay-portable.zip`. The APK is `assembleRelease` signed by the key in `%USERPROFILE%\.phonebridge-release` (never in the repo; CI falls back to the debug key). The release is created with `gh release create v0.1.0` on the private repository, notes written from `docs/DECISIONS.md`, and only after the polished build passes the gates and CI is green.

Known consequence: the phone currently has the debug-signed app. Installing the new signed APK needs an uninstall first, which clears pairing; the notes say so.

## Review focus

- 200% display scale and a second monitor with a different scale: no blur, no clipped text.
- Flyout opened with a long peer name or a very long last-event line: ellipsis, no overlap.
- Animation timer left running when a window is hidden: must stop.
- Phone in dark mode and with large font size: grouped cards stay readable.
- The keystore path missing on another machine: build falls back, nothing breaks.

## Out of scope

Real Windows Quick Settings tile, renaming window titles and the Android package, new features, a Play Store listing.
