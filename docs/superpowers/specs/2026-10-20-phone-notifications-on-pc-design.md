# Phone notifications in the Windows notification centre: design

Status: approved in brainstorming on 2026-10-20 (sub-project 4 of 4). Its implementation plan comes before sub-project 1's, because it is small. Next: implementation plan.

## Purpose and scope
Notifications that arrive on the phone appear on the PC as real Windows toasts, and stay in the Windows notification centre. Phone Bridge's own messages move to the same toasts.

- **Direction:** phone → PC. Actions (reply, buttons, clear, open, call answer) go back to the phone.
- **Nothing is stored:** no history file on either side. The PC forgets everything when the connection ends. Toasts already shown stay in Windows' notification centre until they are cleared there or the phone removes them.

### Out of scope
- PC notifications on the phone.
- Call audio on the PC (audio stays on the phone).
- Notification history or search.

## Approach
Real Windows toasts (WinRT `ToastNotificationManager`) under Phone Bridge's own app identity. They are fed by an Android `NotificationListenerService` over a new stream on the existing paired QUIC connection.

**Rejected:**
- **Tray balloons** (`Shell_NotifyIcon`, used today): no reply box, images or grouping; one at a time; hidden by Do Not Disturb.
- **Windows App SDK notifications:** the same features, but they need the App SDK runtime installed or bundled.

## 1. Phone side and wire

**Listener:**
- `NotificationListenerService` inside the Phone Bridge process. The user grants it once under Settings → Notification access.
- On Android 13 that page is a "restricted setting" for apps installed outside the Play Store; the user has already allowed restricted settings for the accessibility service.
- **Skipped:**
  - ongoing notifications (music, downloads, navigation);
  - group summaries;
  - Phone Bridge's own notifications;
  - muted apps.

**Each notification carries:**
- a stable key, the app package and the app name;
- the title and text; `MessagingStyle` gives the sender and the conversation;
- the time;
- **flags:**
  - quiet: silent on the phone, or arrived during the phone's Do Not Disturb;
  - private: then only the app's public version is sent;
  - call: `CallStyle`, or the dialer's call category;
  - one-time code, with the code (see "One-time codes");
- **actions:** a label each, and whether it takes a reply;
- **pictures:** app icons are sent once per app, by hash. Avatars and photos are scaled to at most 256 px, and each is capped in size.

**Wire:**
- When a paired PC connects, it opens one long-lived stream, `NotifyStart`.
- The phone first sends a **snapshot** of the current notifications, all marked quiet so a reconnect never re-alerts. After that it sends `Posted` and `Removed` as they happen.
- PC → phone on the same stream: `Dismiss{key}`, `Action{key, i}`, `Reply{key, i, text}`, `Open{key}`, `Silence`.
- **Limits:**
  - a record over 1 MiB ends the stream;
  - phone → PC: 50 records/s;
  - PC → phone: 10 actions/s;
  - replies: up to 4 KB.
- With several PCs paired, each connected PC gets its own stream.

**Actions on the phone:**
- **Reply:** fills the app's own `RemoteInput` and sends its reply intent, exactly what the phone's inline reply does.
- **Buttons:** send the app's own action intents. Answer and Decline on a call press the dialer notification's own buttons; no extra permission is needed.
- **Dismiss:** `cancelNotification(key)`. It removes the notification on every connected PC.
- **Open:** sends the notification's content intent. Android allows this from the background because the control accessibility service is bound.
- **Silence:**
  - Android lets only the default dialer silence the ringer directly. Phone Bridge asks for **Do Not Disturb access** (a one-time Settings grant).
  - With it, Silence switches the ringer to vibrate for that call and restores the previous mode when the call ends.
  - Verify on the realme.

**One-time codes:**
- Detected on the phone: 4–8 digit (or short alphanumeric) codes next to words like code, OTP, verification, passcode.
- Prices, dates, times and phone numbers are rejected.

## 2. PC side

**App identity** (registered by the tray at start; per user, no admin):
- A Start-menu shortcut "Phone Bridge" carrying the AppUserModelID and a toast-activator CLSID.
- A per-user COM registration (HKCU) pointing at `pb-tray.exe`. Clicks, buttons and replies on toasts in the notification centre reach the running tray, or start it if it isn't running.
- Phone Bridge's own messages (control declined, file received, pairing, "phone stopped sharing its screen") move from balloons to these toasts, so they appear in the notification centre. The status pill stays for live feedback during control.

**A mirrored notification as a toast:**
- **Layout:**
  - the app's name and icon;
  - the sender or title;
  - the text;
  - the photo, if any;
  - the phone's timestamp.
- **Grouping:** one toast per conversation, updated in place with the latest lines. Other apps get one toast per notification. Windows' toast tag and group are hashes of the key and the conversation.
- **Reply:** messaging apps with an inline reply get a reply box and Send.
- **Buttons:** the app's other actions, up to Windows' limit of 5.
- **Right-click menu:** "Stop showing from <app>".
- **Calls:** a call-style toast (Answer / Decline / Silence) that stays until the call is answered, declined or missed on the phone.
- **One-time codes:** a "Copy 123456" button.
  - The copy is kept out of Windows clipboard history and cloud clipboard, and never synced back to the phone by the universal clipboard.
  - After 60 s it is cleared, but only if the clipboard still holds that exact code (clipboard sequence number unchanged and same text), so newer copied text is never overwritten.
- **Removed on the phone:** removed from the notification centre.

**Pictures on the PC:**
- Windows toasts from a non-Store app can only load images from files on disk.
- Icons, avatars and photos go into a private temp folder in Phone Bridge's data directory, capped at 50 MB.
- Each is deleted when its toast goes away, and the folder is emptied at every start.

**Settings** (a new "Phone notifications" page in the tray settings):

| Setting | Choices | Default |
|---|---|---|
| Show phone notifications | on / off | on |
| Content | Full / Full, hidden while PC is locked / Sender only | Full, hidden while locked |
| Show on the Windows lock screen | on / off. Off: toasts that arrive while the PC is locked skip the lock screen and go straight into the notification centre. | off |
| Sound and pop-ups | Follow the phone / Always alert / Always quiet | Follow the phone |
| Clearing on PC clears on phone | on / off | on |
| Clicking opens it on the phone | on / off. On: also opens the phone-screen window once sub-project 1 exists. | on |
| Auto-copy one-time codes | on / off. Never while the PC is locked. | off |
| Quiet while the phone-screen window is focused | on / off. Toasts still go into the notification centre, without a pop-up. | on |
| Muted apps | a toggle per app seen so far (names only, no content) | none muted |

- **"Hidden while locked":** while the PC is locked (session lock notifications), toasts say only "<app>: new notification".
- **"Sender only":** toasts show the app and sender, never the text; the code button reads "Copy code", without the digits.
- **"Follow the phone":** a notification that is quiet on the phone (or arrives during the phone's Do Not Disturb) goes quietly into the notification centre, with no banner or sound. One that alerts on the phone alerts on the PC. Windows' own Do Not Disturb always applies on top.
- **"Show on the Windows lock screen":** whether a toast actually appears there also depends on Windows' own lock-screen notification setting, which Phone Bridge doesn't change.

**Noticing a clear on the PC** (when "Clearing on PC clears on phone" is on):
- A pop-up the user swipes away reports it directly.
- A toast cleared from the notification centre is noticed by comparing Windows' toast history for Phone Bridge every 5 s, only while mirrored toasts exist. Verify on this PC.

**Phone app:**
- The main page gets a "Notifications on PC" card: on/off, and the Notification access status with a "Grant" button.
- Do Not Disturb access is asked for only the first time Silence is used.

## 3. Errors, privacy, budgets and testing

**Errors:**
- **Notification access not granted or revoked:** the phone card shows "Not allowed: Grant". The PC's settings page says "The phone hasn't allowed notification access". If Android disconnects the listener, it asks to be rebound.
- **Link lost:** the stream ends. On reconnect, the quiet snapshot brings the PC back in step without re-alerting: toasts for notifications cleared in the meantime are removed.
- **An action that no longer works** (the notification is gone, or the app rejected the reply): a toast says "Couldn't send your reply: the message was cleared on the phone", with the reply text shown so it isn't lost.
- **Toast setup fails** (the shortcut or registration can't be written): fall back to balloons plus the pill, and log why.
- **Phone Bridge stopped by Android:** no notifications arrive; the existing "not running, tap to turn on" notice covers the phone side.

**Privacy and security:**
- Content goes only to paired peers, over the existing encrypted connection.
- Logs never contain notification text: only the app package and a hash of the key.
- `docs/THREAT_MODEL.md` and `docs/WIRE_PROTOCOL.md` get the new stream.

**Lightweight budgets** (the realme lesson from 2026-10-20):
- The phone side is event-driven only: no polling.
- Pictures are scaled down before sending, and the send queue is capped at 512 KiB; when it is backed up, pictures are dropped first.
- About 5 MB extra memory at most.

**Automated tests:**
- Wire encode/decode and limits.
- The snapshot is quiet.
- One-time-code detection: real samples, plus non-codes such as prices, dates, times and phone numbers.
- Mute filter.
- Clear sync with the setting on and off.
- The clipboard is cleared only if it is unchanged.
- Two-node loopback: posted, removed, reply, action, reconnect.

**Device tests** (realme, Android Studio closed, light sampler running):
- a message gives a toast; a reply from the PC arrives; a conversation updates one toast;
- pictures: app icon, avatar, photo preview;
- clearing in each direction, with the setting on and off;
- a click opens it on the phone (setting on), and does nothing on the phone (setting off);
- a locked PC hides the content; the lock-screen setting on and off;
- the phone's Do Not Disturb keeps it quiet;
- call toast: Answer, Decline, Silence (the ringer comes back after the call);
- code copy, auto-copy, and the 60 s clear that leaves newer clipboard text alone;
- a muted app stays muted;
- a reconnect doesn't re-alert;
- after a tray restart, an old toast's buttons and reply still work;
- a 10-minute run with bursts of messages.

## Decisions recorded here
- Real WinRT toasts with Phone Bridge's own AppUserModelID, Start-menu shortcut and per-user COM activator; balloons only as a fallback.
- Phone Bridge's own messages move to the same toasts.
- All apps by default, with per-app mute; ongoing and summary notifications skipped.
- One long-lived stream per connected PC, starting with a quiet snapshot; nothing stored.
- Reply, buttons, call Answer and Decline through the apps' own intents; Silence through Do Not Disturb access (verify).
- Every behaviour choice the user named is a setting, with the defaults in the table above.
- One-time codes: a copy button, optional auto-copy, never into clipboard history, cleared after 60 s only if unchanged.
- Toast images as temp files (Windows requires files), deleted with their toast.
