# Relay 0.3.0: device test checklist

For whoever runs the checks that tests cannot prove (a person, or Claude on the owner's desktop with the phone on USB debugging). Automatic tests cover the code paths; this list covers how things look, feel and survive real networks and a real phone. The run log is [docs/v2/DEVICE-CHECKS.md](v2/DEVICE-CHECKS.md): tick a line there with the date and the build you used, and add what you found.

## Setup

- **Builds.** Take the PC build (`Relay-<version>-windows-x64-portable.zip` or the setup file) and `Relay-<version>.apk` from the same release or CI run. Use a **separate data folder** for a test PC (`Relay.exe --dir <folder>` or the portable folder) so the owner's own pairing is untouched, and never run the packaging script over the owner's installed folder.
- **Network.** Phone and PC on the same Wi-Fi, the PC's network set to *Private* (see the README: a *Public* network blocks the phone). Do not change Wi-Fi, hotspot, VPN or system settings to make a test pass; if the network blocks the phone, report it.
- **Pairing needs a person.** The phone pairs only by scanning the PC's QR code and comparing six digits. Ask the owner to scan it, then continue.
- **Unlock gate.** If a check needs the gate off, ask the owner to turn it off with one unlock (Settings, Security on the phone; Settings, General on the PC). Turn it back on afterwards and say so.
- **Android over adb.** Always `adb -s <serial>`. Never run `adb kill-server`, `usb`, `tcpip`, `root` or `reboot`; never `svc` or `settings put`; never put the phone to sleep (no `KEYCODE_SLEEP`); never mirror it with Android Studio's Running Devices while testing (it thrashes some phones). Leave the screen awake at the end with a harmless app open.
- **Secrets.** Never open, print or commit the release signing key, `keystore.properties` or any `*.pfx`.

## How to report

For each check: **pass**, **fail** (what happened, what was expected, the app log lines around it: Relay's log is in the data folder, the phone's through the app's Settings, Logs) or **skipped** (why). Add each result to `docs/v2/DEVICE-CHECKS.md`. A failure that is a bug gets a failing test in the code first, then a fix; do not change a test to make a check pass.

## Checks

Each check: do the steps, compare with *Expect*.

### Files, resume and the queue (M9)
1. **Resume after a dropped link, PC to phone.** Send a file of about 1 GB. At about half, turn the phone's Wi-Fi off for ten seconds, then on. *Expect:* the PC log says it retried, the Transfers row keeps counting from where it stopped (not from 0), and the file arrives with the same SHA-256 as the source.
2. **Resume, phone to PC.** Share a 500 MB video to Relay, drop the link the same way at about half, then tap *Retry* on the notification. *Expect:* it continues, not restarts, and arrives intact.
3. **An unanswered prompt is not retried.** With automatic accept off on the receiver, send a file and ignore the question until it times out. *Expect:* the question is shown once, not again by itself.
4. **Queue.** Send ten files at once. Press *Pause queue* on the PC's Transfers page while the third is sending. *Expect:* the third finishes, the rest list as waiting with *Remove* buttons, the button reads *Resume queue*. Remove one, resume. *Expect:* the rest arrive in order, the removed one does not.
5. **Queue survives a restart.** Pause with files waiting, quit Relay, start it. *Expect:* the files are listed as waiting, nothing sends until *Resume queue*; a file you moved meanwhile shows *Needs the file again*.
6. **Cancel.** Cancel the file that is sending. *Expect:* it stops, the phone keeps no half file, and what was queued behind it is gone too.
7. **Receiver on another drive.** Set the PC's save folder to another drive than the data folder and send a file from the phone. *Expect:* it arrives (this was a real bug in review).
8. **0.2.0 interop.** With a Relay 0.2.0 PC or phone, send a file both ways. *Expect:* it works (no resume, no errors).
9. **Drag and drop, PC.** Drag files from Explorer onto the quick panel, the main window and the screen window. With two phones paired *Expect:* a "Send to" menu; with one, no menu.
10. **Drag and drop, phone.** On a tablet, foldable or desktop mode, drag a photo from another app onto Relay. *Expect:* it goes to the default PC. On a phone-sized window nothing accepts the drop.

### Security and several devices (M7, M8)
Run every line under *Unlock gate*, *Several devices* and *Not run yet* in the run log: the phone prompt, Windows Hello, Disconnect everything, upgrade keeps the answers, names and default, Never, per-PC switches, share sheet, tile and widget, PC Send to.

### Windows look (M5)
Mica, Acrylic, Relay glass, toggling transparency, high contrast, Remote Desktop and text at 100%, 125% and 150% scaling, as written in the run log under *PC*.

### Installer and firewall (M10)
11. **Per-user install.** Run the setup file as a normal user. *Expect:* no administrator prompt, a Start menu entry, Relay starts, Windows asks once about the firewall, and after choosing *Private networks* the phone connects.
12. **All-users install.** Run it and choose *install for all users*. *Expect:* an administrator prompt and a rule named *Relay* (UDP, inbound, Private only; check with `netsh advfirewall firewall show rule name=Relay`). Uninstall: the rule is gone.
13. **Portable zip.** Unzip on a local drive and run. *Expect:* everything stays in the `data` folder beside it.

## Measurements to write down
- The PC tray's idle private memory and CPU over 60 seconds with the window closed (`scripts/pc-budget.ps1`). The budget is 15 MB and 0.05 s; the last run was over, and the cause is open.
- The speed of a 1 GB transfer each way on the same Wi-Fi.
- The phone's battery use over an hour idle with Relay on.

## Not testable yet
- The clipboard history has no switch or list on screen.
- The update check has no button and no public key.
- The phone has no automatic retry; the PC's *Retry* on a failed row sends outside the queue.
