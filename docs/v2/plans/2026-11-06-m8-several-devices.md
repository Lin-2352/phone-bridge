# M8 Several devices Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:executing-plans (inline, TDD, one reviewer at the end of the milestone). Steps use checkbox (`- [ ]`) syntax. Run `advisor` where a step says **ADVISOR** (M8.4 changes consent paths; also the `.R` task). Run several Monitors at once (cargo, gradle, CI by full SHA); never wait on one job alone.

**Goal:** let one phone work with several PCs and one PC with several phones: a device picker wherever a send starts, a default device, local nicknames and per-device status, per-device permissions (Ask / Always / Never) in place of global switches, and a migration that keeps every existing install behaving the same.

**Architecture:** a per-device store on each side (Android `PcStore` over SharedPreferences `"pcs"`; PC `DeviceStore` in `devices.txt`) holds each device's settings. Pure functions decide the send target, labels, the default after removal, the consent step and the migration; platform code only calls them. The ffi switches that were global (clipboard sync, notifications, auto-accept) take peer lists, so the transport filters per peer.

**Tech Stack:** Rust 1.99.1, uniffi, Kotlin and Compose, the M6 `DevicePickerSheet`.

**Spec:** `docs/v2/SPEC.md` lines 318–323; rows M8.1–M8.R in `docs/v2/PROGRESS.md`; `docs/DECISIONS.md` ("No master switch", Phase 9, line 428).

## Global Constraints
- MIT OR Apache-2.0; no GPL/AGPL code.
- "The PC can only ask": a per-device "Always" lets that device's request skip the prompt only where it does today (never while the phone is locked); "Never" refuses without a prompt. No consent is weakened. The M7 unlock gate guards every change to Always and every auto-accept switch.
- No wire change: `PROTOCOL_VERSION` stays 0, `a_0_2_0_peer_still_talks` stays green. Nicknames are local labels and are never sent.
- Budgets unchanged (Android PSS ≤ 80 MB, one idle wakeup per 15 min; PC idle 0 CPU).
- Never rename the package, Keystore alias, DPAPI entropy, pipe/mutex names, AUMID, notification channel ids or `filesDir` file names. Old pref keys stay for one release, unread, so a downgrade to 0.2.x still works.
- New strings go in `strings.xml` / `strings.rs`, English.
- One commit per task, subject starting with the task ID; the same commit ticks `docs/v2/PROGRESS.md` and moves "Resume here". Push after every task. No attribution lines of any kind.
- Gates: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --no-fail-fast` (two `pb-platform-win` clipboard tests may fail locally only while Relay runs); regenerate the uniffi bindings after ffi changes; Android: `./gradlew :app:testDebugUnitTest :app:lintRelease --no-daemon --console=plain`. One cargo or gradle command at a time. CI by full SHA under a Monitor.

## Review Focus
1. **Two PCs with the same name:** the picker and the settings tell them apart. Test: `duplicate_names_get_a_fingerprint_suffix` (M8.2).
2. **Removing the default PC (and re-pairing it later):** the next one becomes the default; with none left, a send asks to pair; and removing a device purges its per-device permissions and legacy allow-list entries, so re-pairing starts at fresh Ask defaults. Tests: `removing_the_default_picks_the_next_by_label`, `no_pc_means_pair_first`, `re_pairing_a_removed_device_starts_at_fresh_defaults` (M8.2, M8.1, M8.4).
3. **An old single-PC install updating:** nothing it allowed or refused changes. Tests: `old_single_pc_install_migrates_to_its_pc`, `old_settings_become_each_phones` (M8.5).
4. **A PC set to Never asks for the screen:** no prompt appears and the timeline records it (rate-limited to once a minute per peer and kind, so a looping peer cannot flood the timeline). Tests: `never_denies_without_asking`, `repeated_never_blocks_do_not_flood_the_timeline` (M8.4).
5. **A locked phone with a PC set to Always:** it still asks (the M1.6 rule). Test: `always_still_asks_while_locked` (M8.4).

## Rulings already made
- Execution order: Ruling: M8.2 runs first (it creates the per-device store the other tasks need), then M8.1, M8.3, M8.4, M8.5; each PROGRESS row is still ticked and committed as its own task — dependency order — none.
- Ruling: nicknames are local labels, never sent — renaming another device's own name is that device's business — none.
- Ruling: settings are copied to every paired device on migration, not only the first — today's globals already applied to all of them; copying to one would silently change behaviour for the others — none.
- Ruling: old pref keys are kept for one release, unread, for a downgrade to 0.2.x, but **revocations and unpairs write through**: any change away from Always, or unpairing a device, also removes the id from the matching legacy set (`allow_control_pcs`, `allow_screen_pcs`, `allow_camera_pcs`) and purges `"<peerId>/*"` from `"pcs"` / `DeviceStore`, and turns the legacy `auto_accept` / `clipboard_sync` / `audio_skip_confirm_phone_to_pc` off when no PC still has them on; grants never write through — otherwise a PC set to Never or unpaired in 0.3 would be always-allowed again after a downgrade or a re-pair — cost if wrong: a downgraded 0.2.x loses a grant until it is set again.
- Ruling: the PC has no "locked" rule for consent (M1.6 is phone-only), so `consent_step` on the PC is `Never` → refuse, `Always` → allow, `Ask` → ask — none.
- Ruling: `pickerRows` (M6.6) takes its labels from `labels(...)` (M8.2), so nicknames and the fingerprint-suffix rule live in one place — none.
- Ruling: the widget gets a small "Choose PC" chevron button that opens the picker — Glance offers no long-click — cost if wrong: one more touch target on the 2x1 widget, checked for 48 dp.
- Ruling: PC events gain `peer_id` (`AskControl { from, peer_id }`, `IncomingOffer { from, peer_id, … }`), closing the M3.3 deferral, and each consent dialog carries its own `peer_id` rather than reading a global — none.
- Ruling: new prefs need no `PREFS_VERSION` bump except the migration key `pcs_migrated` (idempotent, guarded by itself); the PC settings step is v3 → v4 (after M5's v1 → v2 and M7's v2 → v3) — none.

## Today in the code (checked 2026-11-06)
- Android single-PC assumption: `Bridge.firstPeer()` (`PbService.kt:164`) and `peers().firstOrNull()` at `SendClip.kt:85`, `ClipImages.kt:87`, `TouchpadPad.kt:149`, `MainActivity.kt:188`, `PbService.kt:533`, `HomeTab.kt:548`, `SettingsTab.kt:527`; `Bridge.linked: MutableState<String?>` holds one name; `ShareActivity` (root `EntryPoints.kt:26`) uses a platform `AlertDialog` "Send to ‹first PC›?" and sends files one after another in a loop (`:151`).
- Android storage: one SharedPreferences file `"settings"`; per-PC data is only the string sets `allow_control_pcs`, `allow_screen_pcs`, `allow_camera_pcs` with helpers in `AllowList.kt`; global keys that are really per PC: `auto_accept`, `clipboard_sync`, `notify_to_pc`, `audio_skip_confirm_phone_to_pc`, `last_pc_name`, `last_seen_ms`; `PrefsMigration.kt` (`PREFS_VERSION = 1`, `migratePrefs`) and `Prefs.migrateAllowances(pairedIds)` at node start (`PbService.kt:502`).
- ffi (`crates/pb-ffi/src/lib.rs`): `PbPeer { id, name, fingerprint }` with no status; `sync_clipboard(text, sensitive)` and `set_auto_accept(on)` are global; no rename API.
- PC: `devices.rs` (`DeviceStore`, `devices.txt`, `version=1`, `MAX_KEYS = 32`, `MAX_VALUE = 256`); `save` does not fsync; the only key used is `app_version`; `settings.rs` globals `allow_control`, `auto_accept`, `universal_clipboard`, `notify_enabled`, `audio_skip_confirm_pc_to_phone`; `UiEvent`s are keyed by name (`AskControl{from}`, `IncomingOffer{from}`).
- Widget and tiles: the widget's status text, "Send clipboard" and "Send file" go to the first PC; tile long-press opens `QuickPanelActivity` (`QS_TILE_PREFERENCES`).
- DECISIONS: "No master switch" at line 428 under "Phase 9 (2026-11-03): audio sharing, both directions".

## Shared interfaces
```kotlin
// devices/PcStore.kt (pure helpers take Map<String, Any?>; SharedPreferences "pcs", keys "<peerId>/<key>")
enum class Allow { Ask, Always, Never }
data class PcSettings(val nickname: String?, val control: Allow, val screen: Allow, val camera: Allow, val audio: Allow,
                      val clipboardSync: Boolean, val notifications: Boolean, val autoAccept: Boolean, val lastSeenMs: Long)
fun freshDefaults(): PcSettings                                   // Ask everywhere; clipboard, notifications and auto-accept off
fun pcSettings(all: Map<String, Any?>, id: String): PcSettings    // missing keys -> freshDefaults()
fun displayName(nickname: String?, name: String): String          // trimmed nickname (<= 64, no control chars) or name
fun labels(peers: List<PbPeer>, nick: Map<String, String?>): Map<String, String>   // duplicate display names get " (ABCD)" = first 4 of the fingerprint
fun defaultAfterRemoval(remaining: List<PbPeer>, removed: String, current: String?): String?   // keep current unless removed; else first by label
// prefs "settings": default_pc (peer id)
sealed interface SendTarget { data class One(val id: String) : SendTarget; data class Ask(val rows: List<PickerRow>) : SendTarget; object None : SendTarget }
fun sendTarget(peers: List<PbPeer>, explicit: String?, defaultId: String?): SendTarget
enum class ConsentStep { AutoAllow, Ask, AutoDeny }
fun consentStep(perm: Allow, keyguardLocked: Boolean): ConsentStep
```
```rust
// crates/pb-tray/src/devices.rs additions
pub enum Allow { Ask, Always, Never }                      // stored "ask" / "always" / "never"; unknown reads as Ask
pub struct DevicePerms { pub control: Allow, pub audio_skip_confirm: bool, pub clipboard: bool, pub notifications: bool, pub auto_accept: bool, pub nickname: Option<String> }
impl DeviceStore { pub fn perms(&self, id: &str) -> DevicePerms; pub fn set_perms(&mut self, id: &str, p: &DevicePerms) -> bool; pub fn remove_device(&mut self, id: &str) -> bool }
// settings.rs: `default_device: String` (hex id or empty)
// share.rs: pub fn send_target(peers: &[PairedPeer], explicit: Option<&str>, default: Option<&str>) -> SendTarget
```

---

### Task M8.0a: Refresh at start
- [ ] Re-check every line under "Today in the code" with Grep. Confirm the M6 `PickerRow`, `pickerRows` and `DevicePickerSheet` signatures (M6.6), the M7 settings version (v3) and the M7 gate call sites (`Guarded.AlwaysAllow`, `Guarded.AutoAccept`, `Guarded.ClipboardSync`); record drift as `Ruling:` lines here before M8.2. Re-check M7.4's key lists (`alwaysAllowKeysToClear`, `always_allow_off`) against the per-device store: after M8 they must reset the per-device values (see M8.4). Make `pickerRows` call `labels(...)` for its labels.

### Task M8.2: Default device, renaming and status per device (runs first)
**Files:** Android: create `devices/PcStore.kt`; modify `Prefs.kt` (`default_pc`), `ui/settings/SettingsTab.kt` (Connection page: Rename dialog, default star, status per PC from `pcStatus(live, lastSeenMs, nowMs, paired)`), `PbService.kt` (`last_seen` per PC), `strings.xml`. ffi: `PbNode.connected_peers(): List<String>` from the transport `live` list (regenerate bindings). PC: `devices.rs` (`nickname` key; `remove_device(id)`; `save` gains `sync_all` before the rename, as `peers.txt` does), `settings.rs` (`default_device`), the M5 Devices page (default star, Rename, status).
- [ ] **Step 1: failing tests.** Kotlin: `nickname_wins_over_name`, `blank_nickname_falls_back`, `duplicate_names_get_a_fingerprint_suffix`, `removing_the_default_picks_the_next_by_label`, `removing_another_keeps_the_default`, `status_per_pc_uses_its_own_last_seen`. Rust: `nickname_is_cleaned_and_capped`, `save_survives_a_crash_between_write_and_rename` (a temp file left behind is ignored on load), `default_is_dropped_when_its_device_is_unpaired`.
- [ ] **Step 2:** `./gradlew :app:testDebugUnitTest --tests '*PcStoreTest*'` and `cargo test -p pb-tray devices::` — Expected: FAIL (unresolved `labels`, `nickname`).
- [ ] **Step 3: implement.**
- [ ] **Step 4:** same commands — Expected: PASS; full gates on both platforms.
- [ ] **Step 5:** PROGRESS, commit `M8.2: default device, nicknames and status per device`, push, CI.

### Task M8.1: Device picker on every send
**Files:** Android: `EntryPoints.kt` (`ShareActivity` hosts the M6 `DevicePickerSheet` in a Compose host when the target is `Ask`, keeps the confirm line for `One`), `SendClip.kt:85`, `ClipImages.kt:87`, `widget/PickFileActivity` (`sendPickedFiles(ctx, uris, target)`), `TouchpadPad.kt:149` (the default PC, or the picker when there is no default). PC: `share.rs`, `ipc.rs`/`core.rs` (`--send`, `--send-clipboard`), `menu.rs` (a popup listing phones by label, default first), `core.rs` events (`AskControl { from, peer_id }`, `IncomingOffer { from, peer_id, … }`).
- [ ] **Step 1: failing tests.** Kotlin: `one_pc_needs_no_picker`, `several_pcs_ask_with_the_default_first`, `an_explicit_pc_skips_the_picker`, `an_unpaired_explicit_pc_falls_back_to_asking`, `no_pc_means_pair_first`. Rust (same names in `share.rs`): `one_phone_needs_no_picker`, `several_phones_ask_with_the_default_first`, `an_explicit_phone_skips_the_picker`, `an_unpaired_explicit_phone_falls_back_to_asking`, `no_phone_means_pair_first`.
- [ ] **Step 2:** `./gradlew :app:testDebugUnitTest --tests '*SendTargetTest*'` and `cargo test -p pb-tray share::` — Expected: FAIL.
- [ ] **Step 3: implement.** Replace each `firstOrNull()` call site with `sendTarget(...)`; the picker opens only for `Ask`.
- [ ] **Step 4:** same commands — Expected: PASS; full gates.
- [ ] **Step 5:** PROGRESS, commit `M8.1: device picker on every send`, push, CI.

### Task M8.3: Tile and widget act on the default device; long-press opens the picker
**Files:** `SendClip.kt` (`SendClipTileService`), `Quick.kt` (`QuickPanelActivity` gains a PC chip that opens the picker), `widget/RelayWidget.kt`, `widget/WidgetState.kt`, `strings.xml`.
- [ ] **Step 1: failing tests:** `tile_click_targets_the_default`, `widget_label_names_the_default`, `no_default_with_several_pcs_opens_the_picker`.
- [ ] **Step 2:** `./gradlew :app:testDebugUnitTest --tests '*TileLogicTest*' --tests '*WidgetStateTest*'` — Expected: FAIL; implement; Expected: PASS.
- [ ] **Step 3:** tile click sends to the default; its long-press (`QS_TILE_PREFERENCES`) opens the quick panel with the PC chip; the widget buttons send to the default and the widget shows the "Choose PC" chevron (see the ruling); the status names the default PC.
- [ ] **Step 4:** full gates; PROGRESS; commit `M8.3: tile and widget use the default device`, push, CI.

### Task M8.4: Per-device permissions
**Files:** Android: `AllowList.kt`, `AudioConsentActivity`, `ScreenConsentActivity`, `CameraConsentActivity`, `ControlRequestActivity`, `PbService.kt`, `ui/settings/SettingsTab.kt` (each PC on the Connection page opens a "‹PC› permissions" page: Control, Screen, Camera, Audio as Ask / Always / Never; Clipboard sync, Notifications, Auto-accept as switches), `SettingsGroups.kt`. ffi: `set_clipboard_peers(ids: Vec<String>)`, `set_notify_peers(ids: Vec<String>)`, `set_auto_accept_peers(ids: Vec<String>)` replace the global booleans (regenerate bindings). PC: `devices.rs`, `settings.rs`, `core.rs` (per-phone `allow_control`, `auto_accept`, `universal_clipboard`, `notify_enabled`, `audio_skip_confirm_pc_to_phone`), a per-phone page in Settings. Docs: `docs/DECISIONS.md` — new entry `## Per-device permissions (v2 M8)`: "Supersedes 'No master switch' (Phase 9). With several PCs, 'Never ask from this PC' (Never) is a separate decision from answering No once; Never refuses without a prompt and is logged in the timeline. Ask stays the default for every new PC"; add `(superseded by "Per-device permissions", v2 M8)` to the line-428 bullet. Changing anything to Always, or turning auto-accept or clipboard sync on, goes through the M7 gate (`Guarded.AlwaysAllow`, `Guarded.AutoAccept`, `Guarded.ClipboardSync`).
- **Unpairing a device:** removes all `"<peerId>/*"` entries from `PcStore` / `DeviceStore` and writes through to the legacy sets, so re-pairing the same device later starts at `freshDefaults()`.
- **Disconnect everything (from M7.4) is rewritten here:** `disconnectEverything` and `always_allow_off` now set every device's control, screen, camera and audio to `Ask` and its auto-accept to off in `PcStore` / `DevicePerms`, and still call the legacy clears, so Disconnect keeps revoking everything after the settings move.
- `consentStep(perm, keyguardLocked)`: `Never` → `AutoDeny`; `Always` → `AutoAllow` unless locked, then `Ask` (the M1.6 rule is kept); `Ask` → `Ask`. An `AutoDeny` adds a timeline entry "Blocked ‹what› from ‹PC› (set to Never)" (rate-limited to once per `(peer_id, kind)` per 60 s so a looping peer cannot flood the timeline) and sends the PC the normal refusal with no prompt.
- [ ] **Step 1 (ADVISOR):** before the consent paths change.
- [ ] **Step 2: failing tests.** Kotlin: `never_denies_without_asking`, `repeated_never_blocks_do_not_flood_the_timeline`, `always_still_asks_while_locked`, `ask_asks`, `clipboard_goes_only_to_pcs_with_sync_on`, `a_new_pc_starts_at_ask`, `re_pairing_a_removed_device_starts_at_fresh_defaults`. Rust: `auto_accept_is_per_phone`, `allow_control_is_per_phone`, `perms_round_trip_through_devices_txt`, `unknown_values_read_as_ask`, `never_refuses_without_a_prompt` (`fn consent_step(perm: Allow) -> ConsentStep`; with `Never` no `UiEvent::AskControl` is raised and the normal refusal is sent), `unpairing_purges_device_perms`, `disconnect_resets_every_phone` (after `disconnect_everything` every phone's control is `Ask` and its auto-accept off). Kotlin additions: `disconnect_resets_every_pc_to_ask` (every PC's control, screen, camera and audio are `Ask` and auto-accept is off in `PcStore`), `a_revocation_also_clears_the_legacy_key`.
- [ ] **Step 3:** `./gradlew :app:testDebugUnitTest --tests '*ConsentStepTest*'` and `cargo test -p pb-tray devices::` — Expected: FAIL; implement; Expected: PASS.
- [ ] **Step 4:** full gates on both platforms; PROGRESS; commit `M8.4: per-device permissions`, push, CI.

### Task M8.5: Migration — today's settings become each PC's
**Files:** Android: `devices/PcMigration.kt`, `PbService.kt` (runs where `migrateAllowances` runs, at node start, when the paired ids are known). PC: `settings.rs` (step v3 → v4), `devices.rs` (`fn migrate_devices(s: &Settings, paired: &[PeerId], store: &mut DeviceStore) -> bool`, guarded by a per-device key `migrated=1`).
**Interfaces:** Kotlin `fun migrateToPerPc(all: Map<String, Any?>, pairedIds: List<String>): Map<String, Any?>`, idempotent, guarded by the key `pcs_migrated`. For every paired id, membership of `allow_control_pcs`, `allow_screen_pcs` and `allow_camera_pcs` becomes `Always` (otherwise `Ask`); `audio_skip_confirm_phone_to_pc` gives audio `Always`; `clipboard_sync`, `notify_to_pc` and `auto_accept` are copied; `last_pc_name` and `last_seen_ms` go to the first id; `default_pc` is the first paired id; the old keys stay (see the ruling). PC: `allow_control`, `audio_skip_confirm_pc_to_phone`, `universal_clipboard`, `notify_enabled` and `auto_accept` are copied to each paired phone, and `default_device` is the first.
- [ ] **Step 1: failing tests.** Kotlin: `old_single_pc_install_migrates_to_its_pc`, `allow_lists_become_always`, `migration_runs_once`, `a_pc_paired_later_gets_fresh_defaults`, `two_paired_pcs_both_keep_todays_behaviour`. Rust: `old_settings_become_each_phones`, `migrate_devices_runs_once`, `the_first_paired_phone_becomes_default`.
- [ ] **Step 2:** `./gradlew :app:testDebugUnitTest --tests '*PcMigrationTest*'` and `cargo test -p pb-tray settings:: devices::` — Expected: FAIL; implement; Expected: PASS.
- [ ] **Step 3:** full gates; PROGRESS; commit `M8.5: today's settings become each device's`, push, CI.

### Task M8.R: Milestone review
- [ ] One reviewer (opus) over the M8 diff against the Review Focus; **ADVISOR** when re-grading findings; fix serious findings test-first.
- [ ] Docs: DECISIONS (per-device permissions, nicknames local, the migration copy rule), THREAT_MODEL (Never as auto-deny; per-device clipboard and notification filtering), WIRE_PROTOCOL (no wire change; state it).
- [ ] CI green by full SHA; **ADVISOR** before declaring M8 done. Commit `M8.R: milestone review`.

## Self-Review
- Spec coverage: picker (M8.1), default device, renaming, status (M8.2), tile and widget (M8.3), per-device permissions with the DECISIONS entry (M8.4), migration (M8.5), review (M8.R).
- Type consistency: `PickerRow` and `DevicePickerSheet` come from M6.6; `Allow`, `PcSettings`, `DevicePerms`, `sendTarget` and `consentStep` are defined here once; the gate variants come from M7.3.
- Proportion: bodies are left to the implementer; interfaces, test names and commands are fixed here.
