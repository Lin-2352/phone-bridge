# Phone notifications on the PC: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** phone notifications appear as real Windows toasts, staying in the notification centre, with reply, the apps' buttons, call actions, one-time-code copy and every setting from the spec. Phone Bridge's own messages move to the same toasts.

**Architecture:**
- **Phone:** an Android `NotificationListenerService` hands notifications to Rust (`pb-ffi`). A per-PC outbox in `pb-transport` sends them as new `ControlMsg` variants on short-lived sessions, the same pattern as clipboard sync.
- **PC:** the tray turns them into WinRT toasts under its own AppUserModelID, with a per-user COM activator. Actions go back on a session the PC opens, the same pattern as file offers and control.

**Tech Stack:**
- Rust: prost, iroh/pb-transport, uniffi, `windows` 0.62 (WinRT `UI_Notifications`, `Data_Xml_Dom`; Win32 shell links, COM, WTS).
- Kotlin (Android 13+ target, `minSdk` unchanged).

**Spec:** `docs/superpowers/specs/2026-10-20-phone-notifications-on-pc-design.md`

## Decisions this plan makes where the spec left room (or differs)
Each one is also recorded in `docs/DECISIONS.md` (Task 11), and the spec gets a short "Revised in the plan" note.

1. **No long-lived stream: short-lived sessions instead.** A session held open forever sends heartbeats every 5 s both ways (`HEARTBEAT`), which wakes the phone's Wi-Fi radio all day. Every feature in this code base opens sessions on demand, so this one does too:
   - The phone opens a "notify session" to a PC when there is something to send, keeps it while traffic continues, and finishes it after **15 s idle**.
   - Each notify session starts with `NotifySync{keys}` (the keys active on the phone), so the PC removes stale toasts. Notifications that failed to reach that PC earlier are re-sent first, marked quiet (at most 50).
   - When the tray starts, it sends `NotifySyncRequest` once to each paired phone. The phone answers with quiet `Notification`s for what is active (at most 50, without pictures), then `NotifySync`.
   - The spec's guarantees hold: no re-alerts, and the PC converges after any gap.
2. **The 64 KiB control frame limit stays** (`MAX_CONTROL_FRAME`); the spec's 1 MiB record cap isn't needed.
   - Pictures: JPEG, at most 256 px and **48 KiB**.
   - App icons: PNG at 96 px, at most **16 KiB**.
   - Title: 512 B. Text: 4096 B. Each of the 5 conversation lines: 512 B.
   - Strings are truncated on a UTF-8 character boundary.
3. **Muting is filtered on the PC.** Each PC has its own mute list, and the phone doesn't need per-PC state.
4. "Quiet while the phone-screen window is focused" is wired through `Ctx::mirroring_focused`. It stays `false` until sub-project 1 exists.

## Global Constraints
- Never add AI attribution to commits; commit as the local git identity.
- Existing gates must stay green:
  - `cargo fmt --all -- --check`;
  - `cargo clippy --workspace --all-targets -- -D warnings`, on Windows *and* Linux CI (anything Win32 is `#[cfg(windows)]`, including `mod` lines in `crates/pb-tray/src/main.rs`);
  - `cargo test --workspace`;
  - the Android compile plus lint (`pwsh scripts/android-build.ps1`).
- `unsafe` only in modules that already `#![allow(unsafe_code)]`, or in new tray modules with the same header and a `// SAFETY:` comment per block.
- No notification text in any log, on either side: only the package and a 16-hex key hash.
- Nothing about notifications is stored on disk except toast image temp files (`<data dir>/toast-images/`, capped at 50 MB, emptied at tray start, deleted with their toast) and the PC's muted/seen app list in `settings.txt`.
- **Limits:**
  - phone → PC: 50 notify records/s per session (more closes the session);
  - PC → phone: 10 actions/s;
  - reply text up to 4096 B.
- No admin rights: the shortcut goes in `%APPDATA%\Microsoft\Windows\Start Menu\Programs`, and the COM registration in `HKCU\Software\Classes\CLSID`.
- **AppUserModelID** `PhoneBridge.Tray`. **Toast activator CLSID** `{72c4c623-ea97-46ea-8c4d-5de9e1f8582d}`.
- **Device rules:**
  - always `adb -s 79KRFEJVPVOBQCHQ`;
  - never `KEYCODE_SLEEP`, `settings put`, `svc`, `adb kill-server`, `adb usb|tcpip|root|reboot`;
  - Android Studio is closed during device tests;
  - no persistent logcat streams or uiautomator dumps;
  - announce every permission prompt to the user before it appears.

## Review Focus
1. **Untrusted text reaching toast XML:** a message containing `<`, `&`, `"`, `]]>` or a fake `<action>` must show as literal text. Pinned by `toast_xml_escapes_untrusted_text` (Task 7).
2. **Long or exotic text:** 10 KB messages, emoji and surrogate pairs, RTL. They must truncate without splitting a character, without panicking, and without exceeding the frame. Pinned by `truncation_keeps_char_boundaries` (Task 2) and `oversized_notification_is_trimmed_to_fit_a_frame` (Task 3).
3. **A busy group chat:** 30 updates to one key in 2 s must give one toast updated in place. They must not trip the 50/s limit or open 30 sessions. Pinned by `outbox_coalesces_updates_to_one_key` (Task 3).
4. **Clicking a toast when the phone is away, or after the tray restarted:** an error toast "Couldn't reach <phone>", never silence or a hang. Pinned by `action_to_unreachable_peer_reports_failure` (Task 3); the restart case is a device test (Task 12).
5. **Unpairing a phone with toasts still in the centre:** its toasts are removed, and later activations of old toasts are ignored. Pinned by `unpair_removes_that_peers_toasts` (Task 8).

---

### Task 1: Wire messages

**Files:**
- Modify: `crates/pb-protocol/src/control.rs` (the `Control` oneof tags `"1, …, 9"` become `"1, …, 16"`, plus new messages and tests)
- Modify: `docs/WIRE_PROTOCOL.md` (new section "Phone notifications")

**Interfaces:**
- Produces, in `pb_protocol::control`:

| Variant | Tag | Fields |
|---|---|---|
| `ControlMsg::Notification(Notification)` | 10 | `key: String`=1, `package: String`=2, `app_name: String`=3, `title: String`=4, `text: String`=5, `when_ms: i64`=6, `conversation: String`=7, `lines: Vec<String>`=8, `quiet: bool`=9, `private: bool`=10, `call: bool`=11, `otp: String`=12, `actions: Vec<NotifyActionDesc>`=13, `icon_hash: Vec<u8>`=14, `picture: Vec<u8>`=15 |
| `ControlMsg::NotifyIcon(NotifyIcon)` | 11 | `hash: Vec<u8>`=1, `png: Vec<u8>`=2 |
| `ControlMsg::NotifyRemoved(NotifyRemoved)` | 12 | `key: String`=1 |
| `ControlMsg::NotifySync(NotifySync)` | 13 | `keys: Vec<String>`=1 |
| `ControlMsg::NotifySyncRequest(NotifySyncRequest)` | 14 | none |
| `ControlMsg::NotifyAction(NotifyAction)` | 15 | `id: u64`=1, `key: String`=2, `kind: i32`=3 (`NotifyActionKind`), `index: u32`=4, `text: String`=5 |
| `ControlMsg::NotifyActionResult(NotifyActionResult)` | 16 | `id: u64`=1, `ok: bool`=2, `reason: String`=3 |

- `NotifyActionDesc { label: String = 1, reply: bool = 2 }`.
- `#[repr(i32)] enum NotifyActionKind { Dismiss = 0, Button = 1, Reply = 2, Open = 3, Silence = 4 }` (`prost::Enumeration`).
- Constants:
  - `MAX_NOTIFY_KEY = 256`;
  - `MAX_NOTIFY_TITLE = 512`;
  - `MAX_NOTIFY_TEXT = 4096`;
  - `MAX_NOTIFY_LINES = 5`;
  - `MAX_NOTIFY_LINE = 512`;
  - `MAX_NOTIFY_PICTURE = 48 * 1024`;
  - `MAX_NOTIFY_ICON = 16 * 1024`;
  - `MAX_NOTIFY_SYNC_KEYS = 200`;
  - `MAX_REPLY_TEXT = 4096`.

- [ ] **Step 1: Extend `control_envelope_round_trips_each_variant`** with one fully populated value of each of the 7 new variants. Add `notification_with_max_picture_fits_one_frame`: a `Notification` with every string at its max and a `MAX_NOTIFY_PICTURE` picture → `encode_frame` is `Ok`, and `decode_frame` gives it back equal.
- [ ] **Step 2: Run** `cargo test -p pb-protocol`. Expected: compile errors (types missing).
- [ ] **Step 3: Add the messages, enum, constants and `Control` tags** as specified, with doc comments in the file's style (who sends it, and when).
- [ ] **Step 4: Run** `cargo test -p pb-protocol`. Expected: PASS.
- [ ] **Step 5: Document in `docs/WIRE_PROTOCOL.md`:**
  - the 7 messages and the limits;
  - the session flow (notify session: `NotifySync`, quiet re-sends, then live messages, finished after 15 s idle; tray start: `NotifySyncRequest`; action: `NotifyAction` → `NotifyActionResult`).
- [ ] **Step 6: Commit** `protocol: notification messages (posted, icon, removed, sync, actions)`.

### Task 2: Text helpers (one-time codes, truncation, rate window)

**Files:**
- Create: `crates/pb-transport/src/notify.rs` (add `pub mod notify;` in `lib.rs`)

**Interfaces:**
- Produces:
  - `pub fn find_otp(text: &str) -> Option<String>`
  - `pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str`
  - `pub fn key_hash(key: &str) -> String` (the first 8 bytes of SHA-256, as 16 lowercase hex digits; used in logs and toast tags)
  - `pub struct RateWindow` with `new(limit: u32)` and `fn hit(&mut self) -> bool` (false once more than `limit` hits fall inside the current 1 s window)

- [ ] **Step 1: Write the tests:**

```rust
#[test] fn finds_codes_next_to_keywords() {
    for (t, c) in [("Your OTP is 482913", "482913"), ("G-582044 is your Google verification code", "582044"),
                   ("Use code 7731 to sign in", "7731"), ("Passcode: 09AB12", "09AB12"),
                   ("123456 is your WhatsApp code", "123456")] { assert_eq!(find_otp(t).as_deref(), Some(c), "{t}"); } }
#[test] fn ignores_numbers_that_are_not_codes() {
    for t in ["Your order of ₹1,299 ships on 12/10/2026", "Call me on 9876543210", "Meeting at 10:30",
              "Balance 45231.50", "See you in 2026", "Room 4021 is ready"] { assert_eq!(find_otp(t), None, "{t}"); } }
#[test] fn truncation_keeps_char_boundaries() {
    let s = "a😀b"; assert_eq!(truncate_utf8(s, 2), "a"); assert_eq!(truncate_utf8(s, 5), "a😀");
    assert_eq!(truncate_utf8("שלום", 3), "ש"); assert_eq!(truncate_utf8("abc", 10), "abc"); }
#[test] fn rate_window_allows_limit_then_refuses() { let mut w = RateWindow::new(50);
    assert!((0..50).all(|_| w.hit())); assert!(!w.hit()); }
#[test] fn key_hash_is_16_hex() { let h = key_hash("0|com.whatsapp|1|null|10123");
    assert_eq!(h.len(), 16); assert!(h.bytes().all(|b| b.is_ascii_hexdigit())); assert_ne!(h, key_hash("x")); }
```

- [ ] **Step 2: Run** `cargo test -p pb-transport notify::`. Expected: FAIL to compile.
- [ ] **Step 3: Implement.** Rule for `find_otp`:
  - the text must contain one of these words (case-insensitive): `otp`, `code`, `passcode`, `verification`, `verify`, `one-time`, `pin`, `password`;
  - the candidate is the first token of 4–8 characters, made of digits or uppercase letters plus digits, containing at least 4 digits;
  - the candidate is not part of a longer number, and not next to `₹ $ € £ , . : /`;
  - a `G-` style prefix is stripped.

  `sha2` is already used in the workspace; add it to `pb-transport/Cargo.toml` if it isn't there.
- [ ] **Step 4: Run** `cargo test -p pb-transport notify::`. Expected: PASS.
- [ ] **Step 5: Commit** `transport: one-time code detection, UTF-8 truncation, rate window for notifications`.

### Task 3: Phone-side outbox, sync and actions over sessions

**Files:**
- Modify: `crates/pb-transport/src/notify.rs`
- Test: `crates/pb-transport/src/tests.rs` (two real nodes, as in the existing loopback tests)

**Interfaces:**
- Consumes:
  - `Node::connect`, `Session::{send, recv, finish}`;
  - Task 1's messages;
  - Task 2's `truncate_utf8`, `find_otp`, `RateWindow`.
- Produces:
  - `pub const OUTBOX_LINGER: Duration = Duration::from_secs(15);`
  - `pub const MAX_RESEND: usize = 50;`
  - `pub const MAX_NOTIFY_PER_SEC: u32 = 50;`
  - `pub const MAX_ACTIONS_PER_SEC: u32 = 10;`
  - `pub fn trim(n: &mut Notification)`: applies every Task 1 limit. An oversized picture or too many lines are dropped (oldest lines first), never an error.
  - `pub struct NotifyOutbox`, which is `Clone` and `Send + Sync`:
    - `fn new(node: Node) -> Self`
    - `fn seed(&self, list: Vec<Notification>)`: fills the active set without sending.
    - `fn post(&self, n: Notification, icon_png: Option<Vec<u8>>)`: trims; stores the icon by package (at most 64 packages); updates the active set (stored without the picture); queues to every paired peer.
    - `fn remove(&self, key: &str)`
    - `fn answer_sync_request(&self, s: &Session) -> impl Future<Output = Result<(), Error>>`: sends quiet copies of up to `MAX_RESEND` active notifications (the newest by `when_ms`), each preceded by its `NotifyIcon` once per session, then `NotifySync`.
  - `impl Node { pub async fn notify_action(&self, peer: &PeerId, action: NotifyAction) -> Result<NotifyActionResult, Error> }`: connect, send, wait up to 10 s for the result whose `id` matches, then finish.
  - `impl Node { pub async fn notify_sync_request(&self, peer: &PeerId) -> Result<Session, Error> }`: connect, send `NotifySyncRequest`, and return the session, so the caller's normal pump reads the answer until the phone finishes.

**Per-peer outbox task.** Each paired peer has its own task, fed by an unbounded channel of `Out::{Post(Notification), Remove(String)}`:
1. On the first item, connect, then send `NotifySync{active keys}` and the quiet re-sends of that peer's `missed` keys.
2. **Before each write:** drain the channel. Keep only the last `Post` per key; a `Remove` cancels a queued `Post` of the same key. Send a `NotifyIcon` before the first `Notification` in this session that uses that icon hash.
3. With no item for `OUTBOX_LINGER`, call `finish()` and end the task. The next item starts a new task.
4. **On a connect or write error:**
   - add the queued posts' keys to `missed` (insertion-ordered, at most `MAX_RESEND`, oldest dropped);
   - drop the queue;
   - end the task.
- Peers that are unpaired get no task.

- [ ] **Step 1: Write the loopback tests in `tests.rs`.** On the PC side each test runs a test-local pump: it reads `ControlMsg`s into a `Vec` until the session ends.
  - `notify_session_starts_with_sync_then_posts`: seed `[a]`, post `b` → the PC sees `NotifySync{keys: [a, b]}` (any order), then `NotifyIcon`, then `Notification{key: b}`.
  - `outbox_coalesces_updates_to_one_key`: post 30 versions of key `k` (text `"m0".."m29"`) back to back → the PC receives at most 3 `Notification`s for `k`, and the last has text `"m29"`.
  - `remove_before_send_cancels_the_post`: post `x`, then remove `x` before the PC accepts → no `Notification{key: x}` arrives, and a later `NotifySync` doesn't contain `x`.
  - `missed_posts_are_resent_quietly`: post `m` while the PC node is down; start the PC; post `n` → the PC sees `Notification{key: m, quiet: true}` before `n`.
  - `oversized_notification_is_trimmed_to_fit_a_frame`: text of 10 000 `"😀"`, a 200 KiB picture, 9 lines → it arrives with `text.len() <= 4096`, an empty picture and 5 lines.
  - `sync_request_answers_with_quiet_snapshot`: seed 60 notifications on the phone; the PC calls `notify_sync_request` and pumps → it gets 50 quiet `Notification`s and one `NotifySync` with 60 keys.
  - `action_round_trip`: the phone pump answers `NotifyAction{kind: Reply, text: "ok"}` with `NotifyActionResult{ok: true}` → `notify_action` returns `ok == true` with the matching `id`.
  - `action_to_unreachable_peer_reports_failure`: the peer node is shut down → `notify_action` returns `Err(_)` within 25 s (the existing dial timeout), without panicking.
- [ ] **Step 2: Run** `cargo test -p pb-transport notify`. Expected: FAIL to compile.
- [ ] **Step 3: Implement `trim`, `NotifyOutbox`, `notify_action` and `notify_sync_request`** in `notify.rs`.
- [ ] **Step 4: Run** `cargo test -p pb-transport`. Expected: all PASS, including the 35 existing tests.
- [ ] **Step 5: Commit** `transport: notify outbox (coalescing, linger, quiet resend), sync request and actions`.

### Task 4: FFI for the phone

**Files:**
- Modify: `crates/pb-ffi/src/lib.rs`

**Interfaces:**
- Consumes: Task 3's `NotifyOutbox`, plus `find_otp` and `RateWindow`.
- Produces (uniffi):
  - `#[derive(uniffi::Record)] PbNotification { key, package, app_name, title, text: String, when_ms: i64, conversation: String, lines: Vec<String>, quiet: bool, private: bool, call: bool, actions: Vec<PbNotifyActionDesc>, icon_png: Option<Vec<u8>>, picture_jpeg: Option<Vec<u8>> }`
  - `PbNotifyActionDesc { label: String, reply: bool }`
  - `#[derive(uniffi::Enum)] PbNotifyActionKind { Dismiss, Button, Reply, Open, Silence }`
  - `#[uniffi::export(with_foreign)] trait NotifyHandler: Send + Sync { fn on_action(&self, key: String, kind: PbNotifyActionKind, index: u32, text: String) -> Result<(), PbError>; }`. An `Err(PbError::Failed(reason))` becomes `NotifyActionResult{ok: false, reason}`.
  - On `PbNode`:
    - `fn notify_posted(&self, n: PbNotification)`: a no-op while disabled; otherwise fills `otp` from `find_otp(title + " " + text)` and posts.
    - `fn notify_removed(&self, key: String)`
    - `fn notify_seed(&self, list: Vec<PbNotification>)`
    - `fn set_notify_enabled(&self, on: bool)` (default false until Kotlin sets it)
    - `fn set_notify_handler(&self, h: Option<Arc<dyn NotifyHandler>>)`
  - `Shared` gets `notify: NotifyOutbox`, `notify_enabled: AtomicBool` and `notify_handler: Mutex<Option<Arc<dyn NotifyHandler>>>`.
  - In `pump`, these new arms are handled; other new variants are ignored:
    - `NotifySyncRequest` → `answer_sync_request`;
    - `NotifyAction` → after the `RateWindow::new(MAX_ACTIONS_PER_SEC)` check (over the limit closes the session), call the handler on `spawn_blocking`, then send the result. With no handler: `ok: false`, reason "Notifications on PC is off on the phone".

- [ ] **Step 1: Write the tests** in `crates/pb-ffi/src/lib.rs` `#[cfg(test)]`, host-only, using the pure helper `fn to_wire(n: PbNotification) -> (Notification, Option<Vec<u8>>)`:
  - `to_wire_fills_otp_and_maps_fields`: title "Bank", text "Your OTP is 482913" → `otp == "482913"`, and each field is mapped.
  - `to_wire_without_code_leaves_otp_empty`.
- [ ] **Step 2: Run** `cargo test -p pb-ffi`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** the records, the trait, the methods, the `pump` arms and `to_wire`.
- [ ] **Step 4: Run** `cargo test -p pb-ffi && cargo clippy -p pb-ffi --all-targets -- -D warnings`. Expected: PASS. Then `pwsh scripts/android-build.ps1`. Expected: BUILD SUCCESSFUL; the regenerated Kotlin bindings contain `notifyPosted`.
- [ ] **Step 5: Commit** `ffi: notification posting, seeding, actions handler`.

### Task 5: Android notification listener and actions

**Files:**
- Create: `android/app/src/main/java/dev/phonebridge/app/NotifyListener.kt`
- Modify:
  - `AndroidManifest.xml`: the service with `android:permission="android.permission.BIND_NOTIFICATION_LISTENER_SERVICE"` and the `android.service.notification.NotificationListenerService` intent filter; `<uses-permission android:name="android.permission.ACCESS_NOTIFICATION_POLICY"/>`.
  - `Prefs.kt`: `notifyToPc: Boolean` (default true), `savedRinger: Int` (default −1).
  - `PbService.kt`: after the node starts, `setNotifyEnabled(prefs.notifyToPc)`, `setNotifyHandler(NotifyActions)`, `NotifyListener.instance?.seedAll()`. Restore `savedRinger` if it is ≥ 0.

**Interfaces:**
- Consumes: Task 4's Kotlin bindings (`notifyPosted`, `notifyRemoved`, `notifySeed`, `NotifyHandler`, `PbNotification`, `PbNotifyActionKind`).
- Produces:
  - `class NotifyListener : NotificationListenerService`, with `companion object { @Volatile var instance: NotifyListener?; val connected = mutableStateOf(false) }`;
  - `fun seedAll()`;
  - `object NotifyActions : NotifyHandler`;
  - `fun hasNotifyAccess(ctx: Context): Boolean` (`NotificationManager.isNotificationListenerAccessGranted(ComponentName)`).

**Listener behaviour:**
- **Skip:**
  - `sbn.isOngoing`;
  - `FLAG_GROUP_SUMMARY`;
  - `sbn.packageName == packageName`;
  - `!prefs.notifyToPc`;
  - `Bridge.node == null`.
- **quiet** = `ranking.importance < IMPORTANCE_DEFAULT || !ranking.matchesInterruptionFilter()`.
- **Text:**
  - title from `EXTRA_TITLE`; text from `EXTRA_BIG_TEXT ?: EXTRA_TEXT`;
  - `lines`: the last 5 `EXTRA_MESSAGES` bundles as `"sender: text"` (keys `"sender"` / `"sender_person"` → `Person.name`, and `"text"`);
  - `conversation` = `notification.shortcutId ?: EXTRA_CONVERSATION_TITLE ?: title`.
- **private:**
  - `VISIBILITY_PRIVATE` with a `publicVersion`: use the public version's title and text;
  - `VISIBILITY_SECRET`: title = app label, text = "New notification", no lines or picture.
- **call** = `category == CATEGORY_CALL`.
- **actions:** `notification.actions` → `label = title`, `reply = remoteInputs?.any { it.allowFreeFormInput } == true`.
- **Icon:** sent only the first time per package in this process: `packageManager.getApplicationIcon` → 96 px PNG, dropped if over 16 KiB.
- **Picture:** `EXTRA_PICTURE ?: EXTRA_LARGE_ICON` → longest side 256 px → JPEG at quality 80, stepping down by 10 until it is ≤ 48 KiB; dropped below quality 40.
- Keep `key → StatusBarNotification` for the active keys only. `onNotificationRemoved` → `notifyRemoved(key)` and drop the entry. If the removed notification is a call and `savedRinger ≥ 0`, restore it.
- `onListenerConnected`: `instance = this`, `connected = true`, `seedAll()` (`activeNotifications`, filtered and mapped with no icon or picture, → `notifySeed`).
- `onListenerDisconnected`: `connected = false`, `requestRebind(ComponentName(this, NotifyListener::class.java))`.

**`NotifyActions.onAction`** (any thread; throws `PbException.Failed(reason)` on failure):
- **Dismiss:** `instance.cancelNotification(key)`.
- **Button:** `actions[index].actionIntent.send()`.
- **Reply:** an `Intent()` with `RemoteInput.addResultsToIntent(remoteInputs, intent, bundleOf(resultKey to text))`, then `actionIntent.send(ctx, 0, intent)`.
- **Open:** `contentIntent.send()`. On API 34+, pass `ActivityOptions.makeBasic().setPendingIntentBackgroundActivityStartMode(MODE_BACKGROUND_ACTIVITY_START_ALLOWED).toBundle()`.
- **Silence:**
  - With `NotificationManager.isNotificationPolicyAccessGranted`: save `audioManager.ringerMode` to `savedRinger`, then set `RINGER_MODE_VIBRATE`.
  - Without it: post a phone notification "Allow Do Not Disturb access so your PC can silence calls" that opens `Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS`, and fail with reason "Allow Do Not Disturb access on the phone first".
- A missing key or action fails with "That notification was cleared on the phone".
- Logs carry only the package and `keyHash`.

- [ ] **Step 1: Implement** the files above.
- [ ] **Step 2: Run** `pwsh scripts/android-build.ps1`. Expected: BUILD SUCCESSFUL, with lint clean (NewApi checks guarded with `Build.VERSION.SDK_INT`).
- [ ] **Step 3: Commit** `android: notification listener, actions (reply, buttons, dismiss, open, silence)`.

### Task 6: Phone UI card

**Files:**
- Modify: `android/app/src/main/java/dev/phonebridge/app/MainActivity.kt`: a `NotificationsOnPc` composable next to `ControlFromPc`, in the same style.

**Interfaces:**
- Consumes: `Prefs.notifyToPc`, `NotifyListener.connected`, `hasNotifyAccess`, `PbNode.setNotifyEnabled`.

**Content:**
- Title "Notifications on PC".
- A switch, bound to `notifyToPc` and `Bridge.node?.setNotifyEnabled`.
- A status line: "Showing on your PC" / "Not allowed yet" / "Off".
- While access is missing, a "Grant" button. It opens `Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS` with `EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME` on API 30+, otherwise `ACTION_NOTIFICATION_LISTENER_SETTINGS`.
- Status is re-read in `onResume`.

- [ ] **Step 1: Implement.**
- [ ] **Step 2: Run** `pwsh scripts/android-build.ps1`. Expected: BUILD SUCCESSFUL.
- [ ] **Step 3: Commit** `android: Notifications on PC card`.

### Task 7: Windows toast layer

**Files:**
- Create: `crates/pb-tray/src/toast_xml.rs` (pure, not `cfg(windows)`, so it is tested on Linux CI too)
- Create: `crates/pb-tray/src/toast.rs` (`#[cfg(windows)]`, `#![allow(unsafe_code)]`)
- Modify:
  - `crates/pb-tray/src/main.rs`: `mod toast_xml;` and `#[cfg(windows)] mod toast;`. A `-ToastActivated` argument is accepted and ignored: COM delivers the activation.
  - `crates/pb-tray/Cargo.toml` `windows` features: add `UI_Notifications`, `Data_Xml_Dom`, `Win32_UI_Notifications`, `Win32_UI_Shell_PropertiesSystem`, `Win32_System_Com_StructuredStorage`, `Win32_System_Variant`, `Win32_System_RemoteDesktop`, `Win32_Storage_EnhancedStorage`, and whatever the `#[implement]` macro needs in 0.62 (check with `cargo build`).

**Interfaces:**
- `toast_xml.rs` produces:
  - `pub struct ToastSpec { pub tag: String, pub group: String, pub attribution: String, pub title: String, pub lines: Vec<String>, pub logo: Option<PathBuf>, pub picture: Option<PathBuf>, pub when_ms: i64, pub launch: String, pub buttons: Vec<(String, String)>, pub reply: Option<(String, String)>, pub menu: Vec<(String, String)>, pub popup: bool, pub silent: bool, pub call: bool }`. Each `(label, args)` pair is a button; `reply` is `(placeholder, send args)`.
  - `pub fn toast_xml(s: &ToastSpec) -> String`.
  - `pub enum ToastArgs { Open { tag: String }, Button { tag: String, index: u32 }, Reply { tag: String }, CopyOtp { tag: String }, Mute { tag: String }, Silence { tag: String } }`, plus `pub fn args(a: &ToastArgs) -> String` and `pub fn parse_args(s: &str) -> Option<ToastArgs>`. The format is `a=<open|btn|reply|otp|mute|silence>&t=<tag>[&i=<n>]`.
- `toast.rs` produces:
  - `pub fn register(on_activate: Box<dyn Fn(ToastArgs, Option<String>) + Send + Sync>, on_dismiss: Box<dyn Fn(String) + Send + Sync>) -> Result<(), String>`:
    - creates or updates the shortcut `Phone Bridge.lnk`, with `PKEY_AppUserModel_ID` = `PhoneBridge.Tray` and `PKEY_AppUserModel_ToastActivatorCLSID`;
    - writes `HKCU\Software\Classes\CLSID\{72c4c623-…}\LocalServer32` = `"<exe>" -ToastActivated`;
    - calls `CoRegisterClassObject` for an `INotificationActivationCallback` that parses the args and reads the `reply` input.
  - `pub fn show(s: &ToastSpec) -> Result<(), String>`: `XmlDocument::LoadXml(toast_xml(s))`, `ToastNotification` with `Tag`/`Group`/`SuppressPopup = !popup`, and a `Dismissed` handler that calls `on_dismiss(tag)` for `UserCanceled`.
  - `pub fn remove(tag: &str, group: &str)`.
  - `pub fn history_tags() -> Result<HashSet<String>, String>` (`History().GetHistoryWithId(AUMID)`).
  - `pub fn clear_all()`.

**XML rules:**
- `<toast launch=… displayTimestamp=…>` (ISO 8601 from `when_ms`), with `scenario="incomingCall"` when `call`.
- Inside it: `<visual><binding template="ToastGeneric">`. Title and lines as `<text>`; `<text placement="attribution">`; logo `placement="appLogoOverride" hint-crop="circle"`; the picture as an inline `<image>`; `file:///` URIs.
- `<actions>`:
  - the reply `<input id="reply" type="text">` and its Send button (`hint-inputId="reply"`);
  - up to 5 buttons in total, including Send;
  - menu items with `placement="contextMenu"`.
- `<audio silent="true"/>` when `silent`.
- Every attribute and text value is escaped (`& < > " '`).

- [ ] **Step 1: Write the tests in `toast_xml.rs`:**
  - `toast_xml_escapes_untrusted_text`: title `"<b>&\"Mom\"</b>"`, line `"]]><action content='x'/>"` → the output contains `&lt;b&gt;&amp;&quot;Mom&quot;&lt;/b&gt;`, contains no raw `<action content='x'`, and holds exactly the expected `<action` count.
  - `call_toast_has_scenario_and_three_buttons`.
  - `quiet_toast_is_silent`.
  - `buttons_are_capped_at_five_including_send`: a reply plus 6 buttons → 5 `<action ` that aren't `contextMenu`.
  - `args_round_trip`: every `ToastArgs` variant, with a tag that contains `&=`.
  - `parse_args_rejects_garbage`: `""`, `"a=btn&t=x&i=-1"` and `"a=nope&t=x"` → `None`.
- [ ] **Step 2: Run** `cargo test -p pb-tray toast_xml`. Expected: FAIL to compile.
- [ ] **Step 3: Implement `toast_xml.rs`, then `toast.rs`.**
- [ ] **Step 4: Run** `cargo test -p pb-tray && cargo clippy -p pb-tray --all-targets -- -D warnings`. Expected: PASS. Then manually: `target/spike/pb-tray.exe --dir <tmp>` plus a debug-only `--test-toast` flag (removed in Task 9) shows one toast that appears in the notification centre. Check the Start-menu shortcut exists.
- [ ] **Step 5: Commit** `tray: WinRT toasts with Phone Bridge identity and COM activation`.

### Task 8: Notification policy, mirror state and settings (PC, pure)

**Files:**
- Create: `crates/pb-tray/src/notify.rs` (pure, not `cfg(windows)`)
- Modify: `crates/pb-tray/src/settings.rs`: new fields with load, save and tests

**Interfaces:**
- **Settings fields and defaults:**
  - `notify_enabled: bool = true`;
  - `notify_content: u8 = 1` (0 Full, 1 Full hidden while locked, 2 Sender only);
  - `notify_lock_screen: bool = false`;
  - `notify_sound: u8 = 0` (0 Follow the phone, 1 Always alert, 2 Always quiet);
  - `notify_clear_on_phone: bool = true`;
  - `notify_open_on_phone: bool = true`;
  - `notify_otp_autocopy: bool = false`;
  - `notify_quiet_mirroring: bool = true`;
  - `notify_apps: Vec<(String, String, bool)>` as (package, name, muted), saved as repeated lines `notify_app=<0|1>\t<package>\t<name>`. Tabs and newlines in names become spaces.
- **`notify.rs` produces:**
  - `pub struct Ctx { pub locked: bool, pub mirroring_focused: bool }`
  - `pub struct Presentation { pub title: String, pub lines: Vec<String>, pub popup: bool, pub silent: bool, pub otp_label: Option<String>, pub show_picture: bool }`
  - `pub fn present(n: &Notification, s: &Settings, ctx: &Ctx) -> Option<Presentation>`: `None` when disabled or the package is muted.
  - `pub fn tag(peer: &PeerId, key: &str) -> String` (`key_hash(peer + "|" + key)`)
  - `pub fn group(peer: &PeerId, n: &Notification) -> String` (a hash of the peer and the conversation, or of the key when the conversation is empty)
  - `pub struct Mirror` (tag → `Entry { peer: PeerId, key: String, package: String, otp: String }`), with:
    - `insert(tag, entry)`;
    - `get(&tag) -> Option<&Entry>`;
    - `remove_key(peer, key) -> Option<String>` (the tag);
    - `retain_live(peer, live: &[String]) -> Vec<String>` (the removed tags);
    - `remove_peer(peer) -> Vec<String>`;
    - `remove_package(package) -> Vec<String>`;
    - `vanished(&HashSet<String>) -> Vec<Entry>`, which also drops them;
    - `is_empty()`.
  - `pub fn otp_should_clear(set_seq: u32, now_seq: u32, now_text: Option<&str>, code: &str) -> bool`: true only when `now_seq == set_seq && now_text == Some(code)`.

**`present` rules:**
- **Content:**
  - Full, or "hidden while locked" with `!locked`: title = the sender or title; lines = the conversation lines, or the text.
  - "Hidden while locked" with `locked`: title = app name; lines = `["New notification"]`; no picture; `otp_label = Some("Copy code")`.
  - Sender only: title = the title; no lines; no picture; `otp_label = Some("Copy code")`.
  - Otherwise `otp_label = Some(format!("Copy {otp}"))` when `otp` is non-empty.
- **popup / silent:**
  - Follow the phone: `popup = !quiet`, `silent = quiet`.
  - Always alert: `true` / `false`.
  - Always quiet: `false` / `true`.
  - Then `popup &= !(locked && !notify_lock_screen)` and `popup &= !(mirroring_focused && notify_quiet_mirroring)`.
  - A call is never silent: `silent = false` while `popup` is true.

- [ ] **Step 1: Write the tests** (table-style where useful):
  - `muted_app_is_not_shown`;
  - `disabled_shows_nothing`;
  - `locked_hides_content_in_default_mode`;
  - `full_mode_shows_content_even_when_locked`;
  - `sender_only_never_shows_text_or_code_digits`;
  - `follow_phone_keeps_quiet_ones_quiet`;
  - `always_alert_overrides_phone_quiet`;
  - `lock_screen_off_suppresses_popup_while_locked`;
  - `mirroring_focus_suppresses_popup`;
  - `tags_are_stable_and_16_hex`;
  - `same_conversation_shares_a_group`;
  - `retain_live_returns_stale_tags`;
  - `unpair_removes_that_peers_toasts` (`remove_peer` returns only that peer's tags, and `get` on them is `None`);
  - `vanished_reports_cleared_toasts_once`;
  - `otp_clear_only_if_unchanged` (an unchanged seq with the same text → true; a changed seq → false; the same seq with different text → false);
  - in `settings.rs`, extend `round_trip_and_tolerant_parsing` with every new field, including an app name containing a tab.
- [ ] **Step 2: Run** `cargo test -p pb-tray notify:: settings::`. Expected: FAIL to compile.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p pb-tray`. Expected: PASS.
- [ ] **Step 5: Commit** `tray: notification presentation policy, mirror state, settings`.

### Task 9: Tray integration (core, UI thread, own messages)

**Files:**
- Modify:
  - `crates/pb-tray/src/core.rs`
  - `crates/pb-tray/src/ui.rs`
  - `crates/pb-tray/src/main.rs` (remove `--test-toast`)

**Interfaces:**
- Consumes: Tasks 3, 7 and 8.
- Produces:
  - `UiEvent::Notify { peer: PeerId, peer_name: String, n: Notification, logo: Option<PathBuf>, picture: Option<PathBuf> }`
  - `UiEvent::NotifyRemoved { peer: PeerId, key: String }`
  - `UiEvent::NotifySync { peer: PeerId, keys: Vec<String> }`
  - `UiEvent::NotifyActionFailed { peer_name: String, reason: String, reply: Option<String> }`
  - `Cmd::NotifyAction { peer: PeerId, key: String, kind: NotifyActionKind, index: u32, text: String }`
  - `Cmd::NotifyPeerRemoved(PeerId)` is not needed: Unpair already goes through `Cmd::Unpair`, which also sends `UiEvent::NotifySync { keys: vec![] }` for that peer.

**Core (`core.rs`):**
- **At start:**
  - empty `<dir>/toast-images/`;
  - after `Ready`, for each paired peer, spawn `notify_sync_request`, then the existing `pump` on the returned session;
  - errors are logged once and otherwise ignored (the phone may be away).
- **In `pump`, with a `RateWindow::new(MAX_NOTIFY_PER_SEC)` per session (over the limit: log and close):**
  - `NotifyIcon`: write `<dir>/toast-images/icon-<hex>.png` if absent; remember hash → path for this tray run.
  - `Notification`: if a picture is present, write `<dir>/toast-images/<tag>.jpg`; then send `UiEvent::Notify`.
  - `NotifyRemoved` → `UiEvent::NotifyRemoved`. `NotifySync` → `UiEvent::NotifySync`.
  - Keep the folder under 50 MB: delete the oldest files when over.
- **`Cmd::NotifyAction`:** spawn `node.notify_action`. Any `Err`, or `ok == false`, sends `UiEvent::NotifyActionFailed`:
  - the reason is "Couldn't reach {phone}" for `Err`, or the phone's reason;
  - `reply` carries the text for a `Reply`.
- Log lines: `NOTIFY posted pkg=<package> key=<hash>`, `NOTIFY action kind=<k> key=<hash> ok=<b>`. Never any text.

**UI thread (`ui.rs`):**
- **State:** add `mirror: notify::Mirror`, `locked: bool`, `otp_pending: Option<(u32, String)>`, `toasts_ok: bool`.
- **In `create`:**
  - `toast::register` (`toasts_ok` = success; on failure, log it and keep balloons);
  - the activation and dismiss callbacks `post_event` back to the UI thread;
  - `WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)`. `WM_WTSSESSION_CHANGE` with `WTS_SESSION_LOCK` / `UNLOCK` sets `locked`.
- **`UiEvent::Notify`:**
  - record the app in `settings.notify_apps` if it is new, and save;
  - call `present`; if `None`, stop;
  - build a `ToastSpec`:
    - buttons: the app's non-reply actions; for a call, "Answer", "Decline" (the app's own actions, by label) and "Silence";
    - reply: when an action has `reply`;
    - menu: "Stop showing from {app}";
    - the OTP button, when `otp_label` is set;
  - then `toast::show` and `mirror.insert`.
  - **Auto-copy:** if `notify_otp_autocopy && !locked && !otp.is_empty()`: `set_clipboard_own(hwnd, &otp, true)`, `otp_pending = Some((clipboard_seq(), otp))`, `SetTimer(OTP_TIMER, 60_000)`.
- **`NotifyRemoved` / `NotifySync`:** `mirror.remove_key` / `retain_live`, then `toast::remove` for each tag.
- **Activation:**
  - `Open`: only if `notify_open_on_phone`.
  - `Button` / `Reply` / `Silence` → `Cmd::NotifyAction` with the matching kind and index.
  - `CopyOtp`: as auto-copy.
  - `Mute`: set muted in `notify_apps`, save, and `remove_package` → `toast::remove`.
  - Activations whose tag isn't in `mirror` are ignored.
- **Dismissed callback, and a 5 s timer** (`SetTimer(NOTIFY_TIMER, 5000)` armed only while `!mirror.is_empty()`): if `notify_clear_on_phone`, `mirror.vanished(&toast::history_tags()?)` (or the single dismissed tag) → `Cmd::NotifyAction { kind: Dismiss }` per entry.
- **`OTP_TIMER`:** if `otp_should_clear(set_seq, clipboard_seq(), current text, &code)`, empty the clipboard. Then `otp_pending = None`.
- **`NotifyActionFailed`:** a toast titled "Phone Bridge" with the reason, plus "Your reply: {text}" when present.
- **`balloon()`:** when `toasts_ok`, call `toast::show` with tag `pb-<counter>`, group `pb`, attribution "Phone Bridge", popup true. Otherwise keep the `Shell_NotifyIconW` path. Existing balloon-click handling (open file or link) moves to the toast's `launch` args with `ToastArgs::Open { tag: "pb-…" }`, looked up in the existing target list.

- [ ] **Step 1: Run the existing tests first** (`cargo test -p pb-tray`); they must stay green throughout.
- [ ] **Step 2: Implement the core changes**; `cargo clippy -p pb-tray --all-targets -- -D warnings` is clean.
- [ ] **Step 3: Implement the UI changes**; clippy is clean; `cargo test --workspace` passes.
- [ ] **Step 4: Manual smoke test with the phone:**
  - `pwsh scripts/android-build.ps1`;
  - `adb -s 79KRFEJVPVOBQCHQ install -r android/app/build/outputs/apk/debug/app-debug.apk`;
  - ask the user to grant Notification access (announce it first);
  - post a test notification: `adb -s 79KRFEJVPVOBQCHQ shell cmd notification post -S messaging --conversation Mom --message "Mom:Your code is 482913" pbtest "x"`.
  - **Expected:**
    - a toast "Mom" with "Copy 482913" appears and stays in the notification centre;
    - `tray.log` has a `NOTIFY posted pkg=com.android.shell` line and no text;
    - `adb shell cmd notification cancel pbtest` (or the matching shell command) removes the toast.
- [ ] **Step 5: Commit** `tray: phone notifications as toasts, actions, OTP copy, own messages as toasts`.

### Task 10: Settings page

**Files:**
- Modify:
  - `crates/pb-tray/src/settings_window.rs`
  - `crates/pb-tray/src/ui.rs` (`settings_model`, `apply_settings_action`)

**Interfaces:**
- `Key` gains `Notify`, `NotifyLock`, `NotifyClear`, `NotifyOpen`, `NotifyOtp`, `NotifyQuietMirror`.
- New `ChoiceKey { Content, Sound }`.
- `Row::Choice(ChoiceKey, &'static str, [&'static str; 3])` and `Row::App(usize)`, painted like the Theme segmented row and like a Toggle row with the app name.
- `Action::Choice(ChoiceKey, u8)` and `Action::Mute(usize)`.
- `Model` gains the matching bools, `content: u8`, `sound: u8` and `apps: Vec<(String, bool)>`.
- **Section "Phone notifications"**, after "Clipboard", in this order with these labels:
  - Toggle "Show phone notifications" / "Notifications from your phone appear here".
  - Choice "Content" / `["Full", "Hide when locked", "Sender only"]`.
  - Toggle "Show on the lock screen" / "Otherwise they go straight to the notification centre".
  - Choice "Sound and pop-ups" / `["Follow phone", "Always alert", "Always quiet"]`.
  - Toggle "Clearing here clears on the phone" / "Off: clears only on this PC".
  - Toggle "Clicking opens it on the phone" / "Also shows the phone screen when available".
  - Toggle "Copy one-time codes automatically" / "Cleared after 60 s if unchanged; never while locked".
  - Toggle "Quiet while the phone screen is open" / "Toasts still go to the notification centre".
  - Header "Apps", then a `Row::App` per seen app: "{name}", with the toggle on = shown, off = muted.

- [ ] **Step 1: Write the test** `rows_include_notification_section_in_order`: `rows(&Model { apps: vec![("WhatsApp".into(), true)], ..Default::default() })` → the titles between the "Phone notifications" and "Files" headers are in the order above, ending with one `App(0)`. This needs `rows` and `Row` to be testable without Win32: if `settings_window.rs` is `cfg(windows)`, move `Key`, `ChoiceKey`, `Row`, `Model` and `rows` into a new non-Windows module `settings_rows.rs`, re-exported from `settings_window`.
- [ ] **Step 2: Run** `cargo test -p pb-tray settings_rows`. Expected: FAIL.
- [ ] **Step 3: Implement** the rows, painting, hit-testing, actions and model mapping.
- [ ] **Step 4: Run** `cargo test -p pb-tray && cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS. Open the settings window: the new section renders in light and dark, and a toggle persists to `settings.txt`.
- [ ] **Step 5: Commit** `tray: Phone notifications settings page`.

### Task 11: Docs and decisions

**Files:**
- Modify:
  - `docs/THREAT_MODEL.md`: notification content as a new asset (in transit only to paired peers; logs carry hashes only; temp images; OTP clipboard handling; the Windows notification centre keeps toasts until cleared).
  - `docs/DECISIONS.md`: a "Phase 4.0" entry with decisions 1–4 from this plan's top section, plus WinRT toasts with a per-user COM activator.
  - The spec: one "Revised in the plan" paragraph under Status listing decisions 1–3.

- [ ] **Step 1: Edit the three docs.**
- [ ] **Step 2: Commit** `docs: notifications threat model, decisions, spec note`.

### Task 12: Device verification and push

Run on the realme with Android Studio closed, using the light sampler: `pwsh $env:CLAUDE_JOB_DIR/tmp/thrash_sample.ps1 -Step notify -Seconds 600`. Announce each permission prompt. Where a real app is needed (WhatsApp reply, calls), ask the user to send the message or make the call.

- [ ] **Step 1: Run the spec's device tests and record pass/fail with evidence:**
  - a message gives a toast; a reply from the PC arrives in the chat; a conversation updates one toast;
  - app icon, avatar and photo preview;
  - clearing in each direction, with the setting on and off;
  - a click opens it on the phone (setting on), and does nothing on the phone (off);
  - a locked PC (Win+L) hides the content; the lock-screen setting on and off;
  - the phone's Do Not Disturb keeps it quiet;
  - call toast: Answer, Decline, Silence (the ringer comes back after the call);
  - code copy, auto-copy, and the 60 s clear that leaves newer clipboard text alone;
  - a muted app stays muted;
  - a reconnect (turn Phone Bridge off and on) doesn't re-alert;
  - kill the tray, click an old toast's button: the tray starts and the action runs;
  - 10 minutes with message bursts: PSS growth under 5 MB, swap under 200 pages/s average.
- [ ] **Step 2: Fix what fails**, with a failing test first where the cause is in pure code. Re-run.
- [ ] **Step 3: Record the results** in `docs/DECISIONS.md` (Phase 4.0 results).
- [ ] **Step 4: Run every gate:** `cargo fmt --all -- --check`, clippy, `cargo test --workspace`, `pwsh scripts/android-build.ps1`.
- [ ] **Step 5: Commit, push and watch CI:** `git push`, then `gh run watch <id> --exit-status`. Expected: every job passes (rust ubuntu and windows, android, deny).
