# M2 Review sweep Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Budget mode for this program: superpowers:executing-plans, inline.

**Goal:** close every finding of `docs/REVIEW-2026-11-04.md` that M1 left open and that the spec doesn't defer.

**Architecture:** as in M1, each fix puts its rule in a small pure function (Kotlin top-level function or Rust function outside `#[cfg(windows)]`), tested with plain JUnit 4 or `cargo test`. The platform code only calls it. Behaviour that needs a phone goes into the device checklist (M11.5).

**Tech Stack:** Rust 1.99.1 (pb-tray, pb-transport, pb-identity, pb-ffi), Kotlin and Compose (Android app), Gradle.

**Spec:** [docs/v2/SPEC.md](../SPEC.md), milestone M2. Findings: [docs/REVIEW-2026-11-04.md](../../REVIEW-2026-11-04.md). Progress: [docs/v2/PROGRESS.md](../PROGRESS.md).

## Global Constraints
- **Security model:** "the PC can only ask". Nothing here lets the PC decide for the phone.
- **Wire:** no wire change in M2. The protocol version stays 0.
- **Defaults:** text in PC toasts **hidden**; "Keep Control on between sessions" **off**.
- **Budget (new, recorded in DECISIONS by M2.19):** at most one Android wakeup per 15 min when idle with no PC.
- **Tests:** plain JUnit 4 on pure functions (no Robolectric; `android.*` calls throw in JVM tests). PC tests go in modules that build on Linux (the `netpath.rs` pattern: pure function plus a `#[cfg(windows)]` caller), so they run here and not only in CI's windows job.
- **Before each commit:**
  - `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`;
  - when PC code changed: `cargo clippy -p pb-tray --target x86_64-pc-windows-gnu -- -D warnings`;
  - when Android code changed: in `android/`, `ANDROID_HOME=~/android-sdk bash gradlew :app:testDebugUnitTest :app:lintRelease --no-daemon`.
- **Commits:** subject starts with the task ID (`M2.x`); the same commit ticks PROGRESS.md and moves "Resume here". Push after each task.

## Review Focus
1. **An old install that had the audio helper set up** updates to a release build: it starts, and the cleanup runs exactly once (M2.1 `helperCleanupDue` tests).
2. **Update with the old int clip fingerprint stored:** no resend loop; at most one resend of the current clip (M2.2 `legacy_value_never_matches_and_is_dropped`).
3. **Two Windows users with non-Latin names** on one PC get different pipes (M2.3 `cjk_names_without_sid_do_not_collide`).
4. **Ports 47200 and 47199 both taken:** Relay still starts, and the status says automatic discovery is off (M2.15 integration test).
5. **Stop followed at once by a new audio share:** the volume jobs run in order (M2.10 `jobs_run_in_submit_order`).

---

### Task 1 (M2.1): the audio helper ships only in debug builds (S5)
**Files:**
- Move to `android/app/src/debug/java/dev/phonebridge/app/`: `HelperStarter.kt`, `AdbBridge.kt`, `HelperAudio.kt`, the `helper/` package, and from `audio/HelperPlan.kt` the libadb-dependent `needsPairingAgain`.
- Create: `src/release/java/dev/phonebridge/app/HelperStub.kt` (same public names the main code calls, all no-ops reporting "not available"); `src/debug/AndroidManifest.xml` (`HelperPairReceiver`); `src/main/.../HelperCleanup.kt`.
- Modify: `app/build.gradle.kts` (libadb, Conscrypt, BouncyCastle → `debugImplementation`; `sourceSets["qa"].java.srcDir("src/release/java")`), `AndroidManifest.xml` (receiver out; stale `WRITE_SECURE_SETTINGS` comment out), `proguard-rules.pro` (helper-only rules out), `PbService.kt` (askAudio gate; cleanup call), `Prefs.kt` (`audioHelperSecret` moves to the debug side), `.github/workflows/ci.yml` (dependency guard).
- Tests: `HelperCleanupTest.kt`; tests that import libadb move to `src/testDebug`.

**Interfaces:**
- `const val HELPER_BUILT: Boolean`: `true` in the debug source set, `false` in the stub.
- `fun helperAskWaits(offered: Boolean, mic: Boolean, paired: Boolean, silence: Boolean): Boolean` = `offered && !mic && paired && silence`.
- `fun helperCleanupDue(helperBuilt: Boolean, done: Boolean): Boolean` = `!helperBuilt && !done`.
- `object HelperCleanup { fun runOnce(ctx: Context) }`: deletes `noBackupFilesDir/adb_identity` and the Keystore alias `relay_adb_identity` (the steps of `AdbBridge.forget`), clears `audioHelperPaired` and the helper secret, then sets pref `helper_cleaned`.

- [ ] **RED:** `cleanup_runs_in_builds_without_the_helper`, `cleanup_runs_only_once`, `debug_builds_keep_their_helper`, `ask_never_waits_when_the_helper_is_not_offered`, `ask_waits_only_for_a_paired_silenced_share`.
- [ ] **GREEN:** the move, the stub, the gate in `askAudio`, `HelperCleanup.runOnce` from `PbService.onCreate`.
- [ ] **CI guard:** after the Android tests, `gradlew :app:dependencies --configuration releaseRuntimeClasspath` must not mention `libadb`, `conscrypt` or `bcpkix`; the step fails if it does.
- [ ] **Verify:** Android checks, plus `assembleQa` here when possible (otherwise CI).
- [ ] **Checklist note (M11.5):** an old install's device-idle allow-list entry can only be removed over adb.
- [ ] **Commit** `M2.1: the audio helper ships only in debug builds`.

### Task 2 (M2.2): the clip fingerprint is a keyed hash (S9)
**Files:** Create `ClipFingerprint.kt`, `ClipFingerprintTest.kt`. Modify `Prefs.kt:41-45`, `PbService.kt` (RemoteClipboard), `Quick.kt` (`shareClipboardIfChanged`).

**Interfaces:**
- `fun clipFingerprint(text: String, mac: (ByteArray) -> ByteArray): String`: lowercase hex of `mac(text.toByteArray(UTF_8))`.
- `fun clipUnchanged(text: String, stored: String?, mac: (ByteArray) -> ByteArray): Boolean`.
- `object ClipKey { fun mac(): ((ByteArray) -> ByteArray)? }`: Keystore HmacSHA256 key, alias `relay_clip_mac_v1`; `null` when the Keystore fails.
- `Prefs.lastClipMac: String?` (key `last_clip_mac`); the old `last_clip` int is removed on first use.
- With no key, the fingerprint lives in memory only (`@Volatile` in `ClipKey`), never in prefs.

- [ ] **RED:** `same_text_same_key_matches`, `other_key_does_not_match` (JVM `Mac` with two `SecretKeySpec`s), `fingerprint_is_64_hex_chars`, `legacy_value_never_matches_and_is_dropped` (a stored int string doesn't count as unchanged).
- [ ] **GREEN:** wire both callers.
- [ ] **Commit** `M2.2: the clip fingerprint is a keyed hash under a Keystore key`.

### Task 3 (M2.3): the pipe name comes from the user's SID (S11)
**Files:** Create `crates/pb-tray/src/pipe.rs` (declared with the `cfg_attr(not(windows), allow(dead_code))` pattern). Modify `ipc.rs:18-25`, `camera_register.rs` (use `pipe::is_valid_user_sid`).

**Interfaces:**
- `pub fn is_valid_user_sid(s: &str) -> bool` (moved unchanged, with its tests).
- `pub fn pipe_name_for(sid: Option<&str>, user: &str) -> String`:
  - valid SID → `\\.\pipe\PhoneBridgeTray-{sid}`;
  - otherwise the user name filtered to ASCII letters, digits, `-` and `_`;
  - if that leaves nothing, the UTF-16 hex of the user name.

- [ ] **RED:** `sid_names_the_pipe`, `invalid_sid_falls_back_to_the_name`, `cjk_names_without_sid_do_not_collide` (`"张伟"` ≠ `"李娜"`), `ascii_name_fallback_unchanged`.
- [ ] **GREEN:** `ipc::pipe_name()` calls `pipe_name_for(user_sid().as_deref(), &USERNAME)`.
- [ ] **Commit** `M2.3: the pipe name comes from the user's SID`.

### Task 4 (M2.4): no shared text in toasts, no package names in logs (S12)
**Files:**
- PC: `settings.rs` (`toast_show_text: bool`, default `false`), `shared_text.rs`, `notify.rs`, `ui.rs` (Clipboard ~1191, Link ~1289, settings row), `core.rs:1419`.
- Android: `NotifyListener.kt` (135, 137, 372), `AppLog.kt:42`, `res/xml/data_extraction_rules.xml` and `backup_rules.xml`, `LogFields.kt`.

**Interfaces:**
- `shared_text::clipboard_toast_body(sensitive: bool, show_text: bool, text: &str) -> String`: the first 120 characters only when `show_text && !sensitive`, otherwise `"Click to see it on the clipboard."`-style text with no content.
- `shared_text::link_toast_body(show_link: bool, url: &str) -> String`: hidden form `"Click to open."`.
- `notify::posted_log_line(key_hash: &str) -> String`: no package.
- Kotlin `fun notifyLogLine(event: String, keyHash: String): String`; `fun logDirs(internal: File, external: File?): LogDirs(current: File, legacy: File?)` with `current = internal/logs`.

- [ ] **RED (Rust):** `clipboard_toast_hides_text_by_default`, `clipboard_toast_never_shows_sensitive_text`, `link_toast_hides_url_by_default`, `settings_default_hides_toast_text` plus a round trip, `posted_log_line_has_no_package`.
- [ ] **RED (Kotlin):** `notify_log_line_has_no_package`, `logs_live_in_internal_storage`, `old_external_logs_are_marked_for_removal`.
- [ ] **GREEN:** wire the callers; delete the legacy folder once at start; exclude `file`/`logs` from backup.
- [ ] **Commit** `M2.4: no shared text in toasts, no package names in logs`.

### Task 5 (M2.5): capture always covers the whole display (A4)
**Files:** Create `CaptureConfig.kt` and test. Modify `ScreenConsentActivity.kt:126-131`, `AudioConsentActivity.kt:88-94`.

**Interfaces:** `enum class CaptureConfig { NONE, DEFAULT_DISPLAY }`; `fun captureConfig(sdk: Int): CaptureConfig` (`DEFAULT_DISPLAY` from 34); `fun captureIntent(mpm: MediaProjectionManager): Intent` uses `createScreenCaptureIntent(MediaProjectionConfig.createConfigForDefaultDisplay())` when `DEFAULT_DISPLAY`.

- [ ] **RED:** `full_display_from_api_34`, `no_config_below_34`.
- [ ] **GREEN**, then **Commit** `M2.5: screen and audio capture always cover the whole display`.

### Task 6 (M2.6): "Open on phone" is honest about background limits (A5)
**Files:** Create `ActionSend.kt` and test. Modify `NotifyListener.kt:365-400` (`NotifyActions`), `strings.xml`.

**Interfaces:**
- `fun bgStartMode(sdk: Int): Int?`: `null` below 34; `1` (`MODE_BACKGROUND_ACTIVITY_START_ALLOWED`) on 34–35; `3` (`MODE_BACKGROUND_ACTIVITY_START_ALLOW_ALWAYS`) on 36+.
- `enum class OpenOutcome { SEND, TAP_ON_PHONE }`; `fun openOutcome(canStartActivity: Boolean): OpenOutcome`.
- `canStartActivity` = overlay permission granted, or Control bound, or Relay's UI visible.
- On `TAP_ON_PHONE`: post a phone notification "Tap to open ‹app›" whose content intent is the app's own, then fail with `"Tap the notification on your phone"` (the PC shows the reason).

- [ ] **RED:** `no_options_below_34`, `allowed_on_34_and_35`, `allow_always_from_36`, `open_needs_a_tap_when_relay_cannot_start_activities`.
- [ ] **GREEN:** BUTTON and OPEN both send with the options.
- [ ] **Commit** `M2.6: Open on phone opts in to background starts and tells the truth`.

### Task 7 (M2.7): partial photo access is explained (A6)
**Files:** Modify `AndroidManifest.xml`, `ClipImages.kt:51-55`, `ScreenshotWatcher.kt:28`, `ScreenshotFilter.kt`, `ui/settings/SettingsTab.kt` (~474, 710-726), `ui/home/HomeTab.kt:180`, `strings.xml`. Test: `ScreenshotFilterTest.kt`.

**Interfaces:** `fun imagePermissions(sdk: Int): Array<String>` (34+ adds `READ_MEDIA_VISUAL_USER_SELECTED`); `enum class ImageAccess { FULL, PARTIAL, NONE }`; `fun imageAccess(sdk: Int, full: Boolean, partial: Boolean): ImageAccess`. The watcher runs only on `FULL`; `PARTIAL` shows "Relay can only see the photos you picked, so new screenshots won't be sent" with "Allow all photos" (app settings).

- [ ] **RED:** `user_selected_is_requested_from_34`, `partial_grant_is_partial`, `full_wins_over_partial`, `nothing_granted_is_none`.
- [ ] **GREEN:** `RequestMultiplePermissions` for the switch.
- [ ] **Commit** `M2.7: partial photo access is explained`.

### Task 8 (M2.8): the link that was checked is the link that opens (B3)
**Files:** Modify `crates/pb-transport/src/files.rs` (~674-681, `is_web_link` ~1018, `link_tests`), `crates/pb-tray/src/ui.rs` (`open_link`).

**Interfaces:** `pub fn normalize_web_link(text: &str) -> Option<String>`:
- trims;
- matches `http://` or `https://` case-insensitively and lowercases only the scheme;
- rejects whitespace or control characters inside, and an empty host.

`Received::Link` carries the result; `is_web_link(t)` = `normalize_web_link(t).is_some()`.

- [ ] **RED:** `"  HTTPS://Example.com/A \n"` → `Some("https://Example.com/A")`; `"https://a b"` → `None`; `"https://"` → `None`; `"javascript:alert(1)"` → `None`; update the old upper-case case to the normalized value.
- [ ] **GREEN**, then **Commit** `M2.8: the link that was checked is the link that opens`.

### Task 9 (M2.9): a refused armed share puts the encoder back to idle (B6)
**Files:** Create `ArmedNext.kt` and test. Modify `ScreenShare.kt:508-533`.

**Interfaces:** `enum class ArmedNext { SHARE, BACK_TO_IDLE, DISARM }`; `fun armedAfterAnswer(restartOk: Boolean, answered: Boolean): ArmedNext`. Restart failed → `DISARM`; no session → `BACK_TO_IDLE` (calls `c.idle()`); else `SHARE`.

- [ ] **RED:** three cases. **GREEN.** **Commit** `M2.9: a refused armed share puts the encoder back to idle`.

### Task 10 (M2.10): volume work never sleeps on the main thread (B7)
**Files:** Create `audio/VolumeJobs.kt` and test. Modify `SpeakerSilencer.kt` (83, 111), `PhoneSpeaker.kt`, `AudioShare.kt` (launch, teardown), `PbService.kt` and `MainActivity.kt` (start-up restore).

**Interfaces:** `class VolumeJobs(private val executor: (Runnable) -> Unit) { fun submit(job: () -> Unit) }`. The app instance `VolumeJobs.shared` uses one single-thread executor named `pb-audio-vol`. Silencer start and stop, volume walks and restores all go through it, outside the AudioShare lock.

- [ ] **RED:** `jobs_run_in_submit_order`, `submit_returns_before_the_job_runs` (fake executor that queues).
- [ ] **GREEN:** move the calls; no `Thread.sleep` remains on a caller's thread.
- [ ] **Commit** `M2.10: volume work never sleeps on the main thread`.

### Task 11 (M2.11): big pictures are decoded at a safe size (B8)
**Files:** Modify `ClipImages.kt:94-106`; test `ClipImageCacheTest.kt` or a new `DecodeSizeTest.kt`.

**Interfaces:** `fun decodeSampleSize(w: Int, h: Int, maxPixels: Long = 16_777_216): Int`: smallest power of two `s` with `(w/s)*(h/s) <= maxPixels`; 1 for non-positive sizes.

- [ ] **RED:** 4000×3000 → 1; 12000×9000 → 4; 0×0 → 1; 4096×4096 → 1; 4097×4096 → 2.
- [ ] **GREEN:** `inJustDecodeBounds` first, then `inSampleSize`; `compress` inside the catch.
- [ ] **Commit** `M2.11: big pictures are decoded at a safe size`.

### Task 12 (M2.12): only what Relay uses is kept from each notification (B9)
**Files:** Create `LiveStore.kt` and test. Modify `NotifyListener.kt` (59, 96, 116, 123, 143, actions 365-404, `onListenerDisconnected`), `PbService.kt:444`.

**Interfaces:**
- `data class LiveAction<P>(val intent: P?, val replyKey: String?, val freeForm: Boolean)`
- `data class LiveNote<P>(val pkg: String, val category: String?, val content: P?, val actions: List<LiveAction<P>>)`
- `class LiveStore<P>(max: Int = 500) { put(key, note); get(key); remove(key); clear(); hasCall(): Boolean }`: least recently used dropped first.
- The device side stores `PendingIntent` and `RemoteInput` data only.

- [ ] **RED:** `oldest_entry_is_dropped_past_the_limit`, `clear_empties_the_store`, `call_category_is_found`.
- [ ] **GREEN.** **Commit** `M2.12: only what Relay uses is kept from each notification`.

### Task 13 (M2.13): PC audio on the phone respects calls and headphones (B10)
**Files:** Create `audio/FocusPlan.kt` and test. Modify `AudioPlayer.kt:42-85`, `AudioShare.kt` (`startPlayback`, stop).

**Interfaces:**
- `enum class FocusEvent { GAIN, LOSS, LOSS_TRANSIENT, LOSS_CAN_DUCK, NOISY }`; `fun focusEventOf(code: Int): FocusEvent?` (1, -1, -2, -3).
- `sealed class PlayerAction { Play; Pause; Duck(val gain: Float); End }`; `fun playerReaction(e: FocusEvent): PlayerAction`: GAIN→Play, LOSS→End, LOSS_TRANSIENT→Pause, LOSS_CAN_DUCK→Duck(0.2f), NOISY→End.
- A denied focus request ends playback with a reason; End goes through `AudioShare.stop(PC_TO_PHONE)`.

- [ ] **RED:** every mapping, plus `unknown_focus_code_is_ignored`.
- [ ] **GREEN.** **Commit** `M2.13: PC audio on the phone respects calls and headphones`.

### Task 14 (M2.14): the real LAN address comes first (B11)
**Files:** Create `crates/pb-transport/src/addr_rank.rs`. Modify `discovery.rs` (`local_ipv4_interfaces` or a sibling returning interface info), `lib.rs` (`Node::addrs`).

**Interfaces:**
- `pub struct IfaceInfo { pub ip: IpAddr, pub if_type: u32, pub names: String }` (names = name, friendly name, description from `netdev`).
- `pub fn addr_rank(ip: IpAddr, iface: Option<&IfaceInfo>) -> u8`, from best to worst:
  1. private IPv4 on Wi-Fi or Ethernet;
  2. other private IPv4;
  3. CGNAT and global IPv6;
  4. virtual adapters (names containing `Hyper-V`, `vEthernet`, `WSL`, `Docker`, `VirtualBox`, `VMware`, `TAP`, `WireGuard`, `VPN`);
  5. link-local.
- `pub fn rank_addrs(addrs: Vec<SocketAddr>, ifaces: &[IfaceInfo]) -> Vec<SocketAddr>`: stable sort by rank.

- [ ] **RED:** `wifi_lan_beats_docker_and_vethernet`, `link_local_goes_last`, `cgnat_after_lan`, `order_is_stable_within_a_rank`, `real_lan_survives_the_six_address_limit` (8 addresses, LAN last in input).
- [ ] **GREEN.** **Commit** `M2.14: pairing puts the real LAN address first`.

### Task 15 (M2.15): taken ports don't stop Relay (B12)
**Files:** Modify `crates/pb-transport/src/lib.rs` (`Node::bind` ~312-328), `discovery.rs` (`start_discovery` ~199), `tests.rs`; `crates/pb-ffi/src/lib.rs` (`PbNode::discovery_active`); `crates/pb-tray/src/pause.rs` and `pages/home.rs` through a cross-platform helper; Android `StatusLine.kt`, `PbService.kt`.

**Interfaces:**
- `pub(crate) fn bind_attempts(fixed: Option<u16>) -> Vec<Option<u16>>`: `[Some(p), None]` or `[None]`.
- `start_discovery(..) -> bool`, logged; `pub fn Node::discovery_active(&self) -> bool`; pb-ffi `PbNode::discovery_active()`.
- PC: `pub fn discovery_note(active: bool) -> Option<&'static str>` in a Linux-built module: `"Automatic finding is off: another app uses port 47199"`.
- Android: `relayStatus(..., discoveryOff: Boolean)`.

- [ ] **RED:** `bind_attempts` units; integration test `node_starts_when_its_port_is_taken` and `discovery_reports_a_taken_port` in `tests.rs`; `StatusLineTest` case.
- [ ] **GREEN.** **Commit** `M2.15: taken ports don't stop Relay, and the status says so`.

### Task 16 (M2.16): stopping audio capture never waits under the lock (B13)
**Files:** Create `audio/CaptureLifecycle.kt` and test. Modify `AudioShare.kt` (246-250, 340-355), `AudioStreamer.kt` (193-205).

**Interfaces:** `class CaptureLifecycle { fun beginTeardown(): Boolean; fun onSourceEnded(): Boolean }`: atomic; the first caller wins, so a source end after teardown returns false and a second teardown returns false. `teardownCapture` takes the streamer and encoder under the lock, then stops, joins and releases outside it.

- [ ] **RED:** `first_teardown_wins`, `source_end_after_teardown_is_ignored`, `teardown_after_source_end_is_ignored`.
- [ ] **GREEN.** **Commit** `M2.16: stopping audio capture never waits under the lock`.

### Task 17 (M2.17): alarms and ringtones are never silenced (B14)
**Files:** Modify `SpeakerSilencer.kt` (50-60); create `audio/SilenceLift.kt` and test.

**Interfaces:** `fun mustLiftSilencer(usages: List<Int>): Boolean`: true when any usage is `USAGE_ALARM` (4) or `USAGE_NOTIFICATION_RINGTONE` (6). An `AudioManager.AudioPlaybackCallback`, registered while the effect is on, disables the effect while it's true and enables it again after.

- [ ] **RED:** `alarm_lifts`, `ringtone_lifts`, `media_does_not_lift`, `empty_does_not_lift`.
- [ ] **GREEN.** **Checklist:** alarm during a share on a device (M11.5).
- [ ] **Commit** `M2.17: alarms and ringtones are never silenced`.

### Task 18 (M2.18): Control can stay on between sessions (QoL)
**Files:** Modify `ControlAutoOff.kt`, `ControlAutoOffTest.kt`, `ControlService.kt:190-198`, `Prefs.kt`, `ui/settings/SettingsTab.kt` (`ControlFromPc`), `SettingsGroups.kt`, `strings.xml`.

**Interfaces:** `fun shouldSwitchOff(keepOn: Boolean, sessionSeen: Boolean, controlledBy: String?, sharingWith: String?): Boolean`; `Prefs.keepControlOn` (key `keep_control_on`, default `false`). The row "Keep Control on between sessions" explains that remote input pauses while the phone is locked.

- [ ] **RED:** `keep_on_never_switches_off`; the existing four cases pass `keepOn = false`.
- [ ] **GREEN.** **Commit** `M2.18: Control can stay on between sessions`.

### Task 19 (M2.19): fewer idle wakeups
**Files:** Modify `AppLog.kt` (206-220), `PbService.kt` (270-276, 1003-1010, `Bridge`), `MainActivity.kt` (onStart/onStop), `StatusLine.kt` and `StatusLineTest.kt`, `docs/DECISIONS.md`.

**Interfaces:**
- `fun sampleIntervalMs(sessionActive: Boolean, uiVisible: Boolean, interactive: Boolean): Long?`: `null` (no sampling) when neither a session nor the UI is active; else 10 s when interactive, 60 s otherwise. The loop then waits for a state change, not a timer.
- `Bridge.uiVisible`: a counter from MainActivity's onStart and onStop.
- The PC-away status reads `"PC last seen at 14:05"` (today), `"yesterday at 14:05"`, then a date, so no timed redraw is needed; one redraw at the day change.
- `updateStatus` re-posts the notification, tile and widget only when the text changed.

- [ ] **RED:** `no_sampling_when_idle`, `sampling_during_a_session`, the new status texts, `unchanged_status_is_not_reposted`.
- [ ] **GREEN.** Write the DECISIONS entry: the idle budget, and what still wakes (RecoveryJob, the day change).
- [ ] **Commit** `M2.19: fewer idle wakeups; the idle budget is recorded`.

### M2.20: dropped
uniffi 0.32 copies the whole `ByteArray` into a Rust buffer on every `sendFrame` and only takes exact-size arrays, so a reused, larger array would copy more, not less. The device checklist measures memory churn while mirroring instead.

### Task 20 (M2.R): milestone check
- [ ] CI green on the branch head.
- [ ] One reviewer agent reviews the M2 diff against this plan's Review Focus. Re-grade findings by effect; fix the serious ones test-first.
- [ ] THREAT_MODEL: S5, S9, S11, S12 rows. PROGRESS.md: M2 done, next step is the M3 plan. Commit and push.
