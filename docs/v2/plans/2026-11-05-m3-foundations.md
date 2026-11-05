# M3 Foundations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Budget mode for this program: superpowers:executing-plans, inline.

**Goal:** lay the ground that the redesign (M4–M6) and the new features (M7–M9) build on: version compatibility without a breaking bump, capability names, peer ids instead of names, versioned settings with tested migrations, "Relay" folder names for new installs, and a way out when Android's keystore was reset.

**Architecture:** as in M1 and M2, every rule is a small pure function (Rust outside `#[cfg(windows)]`, or a Kotlin top-level function) with `cargo test` or plain JUnit 4 tests; the platform code only calls it. The protocol version stays 0: new `Hello` fields are optional, so Relay 0.2.0 and this build keep talking.

**Tech Stack:** Rust 1.99.1 (pb-protocol, pb-transport, pb-identity, pb-ffi, pb-tray), prost, uniffi, Kotlin and Compose, Gradle.

**Spec:** [docs/v2/SPEC.md](../SPEC.md), milestone M3. Progress: [docs/v2/PROGRESS.md](../PROGRESS.md).

## Global Constraints
- **Wire:** `PROTOCOL_VERSION` stays 0. New fields and capabilities are optional; a 0.2.0 peer that ignores them still works. Every wire change is written into `docs/WIRE_PROTOCOL.md`.
- **Security model:** "the PC can only ask". Nothing here lets the PC decide for the phone.
- **Never rename** what existing installs depend on: `dev.phonebridge.app`, the Keystore alias `phone_bridge_identity_v1`, DPAPI entropy and protocol labels, the pipe and mutex names, the AUMID `PhoneBridge.Tray`, `%ProgramData%\PhoneBridge\camera`, notification channel ids, files in the Android `filesDir`.
- **Tests:** plain JUnit 4 on pure functions (no Robolectric); PC tests in modules that build on Linux. A new rule gets a test that is seen failing first.
- **Before each commit:** `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`; for PC code also `cargo clippy -p pb-tray --target x86_64-pc-windows-gnu --all-targets -- -D warnings`; for Android code, in `android/`, `ANDROID_HOME=~/android-sdk bash gradlew :app:testDebugUnitTest :app:lintRelease --no-daemon`. After a pb-ffi change, regenerate the Kotlin bindings first (`cargo build -p pb-ffi` then `uniffi-bindgen generate`, see PROGRESS.md).
- **Settings rows (PC):** a new toggle also goes into `settings_groups.rs` (OPTIONS, `page_of`, at most 9 rows a page) and the KEYS count in `pages/settings.rs`.
- **Commits:** subject starts with the task ID; the same commit ticks PROGRESS.md and moves "Resume here". Push after each task.

## Review Focus
1. **Relay 0.2.0 on one side:** a 0.2.0-shaped `Hello` is accepted, and this build's `Hello` decodes as a 0.2.0 one (M3.8 tests, both directions).
2. **A PC that never saved its settings:** a new install keeps `Downloads\Relay` on the second start; an old install keeps `Downloads\Phone Bridge` (M3.4 `migration_is_saved_so_it_runs_once`).
3. **Two PCs with the same name:** only the one in the allow list starts without asking (M3.3 `same_name_other_id_is_not_allowed`).
4. **After a keystore wipe:** retries stop, and "Reset and pair again" keeps received files, the timeline and the logs (M3.7 `reset_keeps_everything_but_identity_and_pairings`).
5. **An older build saving after a newer one:** unknown settings keys survive a save (M3.4 `unknown_keys_survive_a_save`).

---

### Task 1 (M3.1): peers check compatibility by rule and name the device to update
**Files:** `crates/pb-protocol/src/lib.rs` (Hello, rule, tests), `crates/pb-transport/src/lib.rs` (`Session::start`, `closed_by_peer`, `Error::VersionMismatch`), `crates/pb-tray/src/transfers.rs` (`session_failure`, `describe`) and `transfers_tests.rs`, `crates/pb-tray/src/audio_state.rs` (`refused_text`), Android `OutdatedText.kt` + test and the send-failure texts (`EntryPoints.kt`, `SendClip.kt`, `ImageNotice.kt`, `ui/remote/RemoteLink.kt`), `.github/workflows/ci.yml`, `docs/WIRE_PROTOCOL.md`.

**Interfaces:**
- `Hello` gains `#[prost(uint32, tag = "4")] min_compatible` and `#[prost(string, tag = "5")] app_version`. `Hello::new` keeps its signature and stamps `MIN_COMPATIBLE` (`pub const MIN_COMPATIBLE: u16 = 0`) and `env!("CARGO_PKG_VERSION")`.
- `pub enum UpdateSide { ThisDevice, OtherDevice, Both }` and `pub fn update_side(local: &Hello, remote: &Hello) -> Option<UpdateSide>`: `remote.protocol_version < local.min_compatible` → `OtherDevice`; `local.protocol_version < remote.min_compatible` → `ThisDevice`; otherwise `None` (compatible). Replaces `is_compatible`.
- Close reason on 426: `b"update:you"` (the receiver must update) or `b"update:me"`; `pub fn side_from_close_reason(reason: &[u8]) -> UpdateSide` (anything else, as from 0.2.0: `Both`).
- `Error::VersionMismatch(UpdateSide)`. Display: `ThisDevice` "this device runs an older version of Relay than the other device: update Relay here"; `OtherDevice` "the other device runs an older version of Relay: update Relay there"; `Both` keeps today's "update both" text.
- Tray: `ThisDevice` → "Update Relay on this PC: {name} needs a newer version"; `OtherDevice` → "{name} runs an older version of Relay. Update Relay on {name}"; `Both` unchanged.
- Kotlin `fun outdatedText(reason: String?, pc: String): String?` keyed on the two Rust phrases ("update Relay here", "update Relay there", "update both"); `null` otherwise. A Rust test pins those phrases.

- [ ] **RED (pb-protocol):** `same_versions_are_compatible`, `a_peer_below_my_minimum_must_update`, `this_device_below_the_peers_minimum_must_update`, `hello_carries_the_build_version`, `close_reasons_name_the_side`.
- [ ] **RED (tray):** the three `session_failure`/`describe` texts.
- [ ] **RED (Kotlin):** `outdatedText` for the three phrases and for an unrelated reason.
- [ ] **GREEN:** wire `Session::start` (close with the side's reason) and `closed_by_peer`; update the Android failure texts to put `outdatedText` first; a CI step fails when Gradle `versionName` differs from the Cargo workspace version.
- [ ] **Docs:** WIRE_PROTOCOL Hello table (fields 4, 5), the rule, the close reasons.
- [ ] **Commit** `M3.1: peers check compatibility by rule and name the device to update`.

### Task 2 (M3.2): every M7–M9 feature has a capability name
**Files:** `crates/pb-protocol/src/lib.rs`, `crates/pb-transport/src/lib.rs` (`Session::start`), `docs/WIRE_PROTOCOL.md`, `docs/DECISIONS.md`.

**Interfaces:**
- `CAP_CLIPBOARD = "clipboard"`, `CAP_AUDIO = "audio"` (advertised); reserved, advertised only when the feature ships: `CAP_DISCOVERY2 = "discovery2"` (M7.6) and `CAP_RESUME = "resume"` (M9.1, also covers the queue's pause and resume).
- `pub const RESERVED_CAPS: &[&str]`; `pub fn local_capabilities() -> Vec<String>` (clipboard, audio).
- The other M7–M9 features (unlock gate, Disconnect everything, pairing limits, quotas, several devices, history, drag-and-drop) change nothing on the wire: the table in WIRE_PROTOCOL says so, feature by feature.

- [ ] **RED:** `capability_names_are_unique_and_lowercase`, `reserved_capabilities_are_not_advertised_yet`, `session_announces_the_local_capabilities` (transport).
- [ ] **GREEN**, docs table and DECISIONS entry ("a capability is advertised only by the build that implements it").
- [ ] **Commit** `M3.2: every M7-M9 feature has a capability name`.

### Task 3 (M3.3): events carry the PC's id; allow lists no longer look ids up by name
**Files:** `crates/pb-ffi/src/lib.rs` (`PbEvent`, event builders, `progress_events`), `crates/pb-ffi/src/audio.rs` (`clone_event`, events), Android `AllowList.kt` + tests, `PbService.kt` (askScreen ~730, askCamera ~777, askControl ~926), `EventRouterTest.kt`.

**Interfaces:**
- Every `PbEvent` variant with `peer_name` gains `peer_id: String` (hex id) right after it. Recovered files with no session use the only paired peer's id, else "".
- `fun askPeerId(eventId: String, pairedIds: List<String>): String`: the id when still paired, else "". `uniquePeerId` and `UniquePeerIdTest` are deleted.
- Ruling: the PC has no allow lists and its core already holds `PeerId` per session; M8 adds ids to the PC's request events with per-device permissions. `InputListener.on_control` stays name-only (display).

- [ ] **RED (Rust):** over a real node pair, `ControlRequest`, `ScreenRequest` and `AudioRequest` carry the PC's `peer_id()`.
- [ ] **RED (Kotlin):** `askPeerId` (paired, unpaired, empty); `same_name_other_id_is_not_allowed` (two PCs named "DESK", one allowed).
- [ ] **GREEN:** regenerate bindings; the three ask paths use `askPeerId(e.peerId, pairedIds)`.
- [ ] **Commit** `M3.3: events carry the PC's id; allow lists no longer look ids up by name`.

### Task 4 (M3.4): PC settings are versioned; per-device facts get their own file
**Files:** `crates/pb-tray/src/settings.rs`, new `crates/pb-tray/src/devices.rs` (cross-platform), `main.rs` (load and migrate at start), `core.rs` (record per device; forget on unpair).

**Interfaces:**
- `pub const SETTINGS_VERSION: u32 = 1`; `version=1` is the first line written.
- `pub struct MigrateCtx { pub existing_install: bool, pub home: PathBuf }`; `existing_install` = `peers.txt` or `identity.sealed` in the data folder.
- `pub fn migrate(from: u32, pairs: Vec<(String, String)>, cx: &MigrateCtx) -> (u32, Vec<(String, String)>)`. Step v0→v1: an existing install with no `save_dir` gets `save_dir=<home>\Downloads\Phone Bridge` pinned (so M3.6 can change the default). A file from a newer version is left as it is.
- `pub fn load_and_migrate(path: &Path, cx: &MigrateCtx) -> Settings`: saves at once when it migrated, so the step runs once even if the user never changes a setting.
- Unknown keys are kept (`Settings.extra: Vec<(String, String)>`) and written back.
- `devices.rs`: `devices.txt`, `version=1`, then `<peer id hex>\t<key>=<value>` lines. `DeviceStore::{parse, to_text, get, set (-> changed), forget, load, save}` with limits (64 devices, 32 keys each, 256-char values, 64-hex ids). First use: each session records the peer's `app_version`; unpairing forgets the device.

- [ ] **RED:** `old_install_without_save_dir_keeps_phone_bridge`, `new_install_has_no_pin`, `current_version_is_unchanged`, `newer_file_is_left_alone`, `migration_is_saved_so_it_runs_once` (temp dir, two loads), `unknown_keys_survive_a_save`; the random-file test's key list fixed (`notify_app`, `version`). Devices: round trip, limits, bad lines skipped, `forget`, random text never panics.
- [ ] **GREEN.** **Commit** `M3.4: PC settings are versioned; per-device facts get their own file`.

### Task 5 (M3.5): Android settings are versioned, with stepwise migrations
**Files:** new `PrefsMigration.kt` + `PrefsMigrationTest.kt`, `Prefs.kt`, `PbService.kt` (after the node starts, where `migrateAllowances` runs today), `AllowList.kt`.

**Interfaces:**
- `const val PREFS_VERSION = 1`; `fun prefsVersion(all: Map<String, Any?>): Int` (no `prefs_version` key → 0).
- `fun migratePrefs(from: Int, all: Map<String, Any?>, pairedIds: List<String>): Pair<Int, Map<String, Any?>>` and `data class PrefsEdit(put, remove)` / `fun prefsDiff(before, after): PrefsEdit`, applied with the version in one `commit()`.
- Step v0→v1: remove `last_clip` and `allow_camera`; fold `allow_control` and `allow_screen` into the per-PC sets for the PCs paired now, then remove them. Ruling: a PC paired later is no longer allowed by an old global switch.

- [ ] **RED:** `each_legacy_key_is_handled`, `global_allow_reaches_only_pcs_paired_now`, `current_version_changes_nothing`, `future_version_is_left_alone`, `string_sets_are_copied`.
- [ ] **GREEN:** `migrateAllowances` is replaced by the step. **Commit** `M3.5: Android settings are versioned, with stepwise migrations`.

### Task 6 (M3.6): new installs use "Relay" folder names
**Files:** `crates/pb-tray/src/portable.rs` (or a new cross-platform `folders.rs`), `main.rs:204-213`, `settings.rs` (default `save_dir`), `app_name.rs` (guard list), `README.md`, `scripts/speed-matrix.ps1`.

**Interfaces:** `pub fn pick_default_dir(local_app_data: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf`: `PhoneBridge\tray` when it exists, else `Relay\tray`. The default `save_dir` becomes `Downloads\Relay` (old installs are pinned by M3.4). Ruling: the Run value name, AUMID, pipe and camera folder stay (not folders the user sees, or depended on).

- [ ] **RED:** `existing_folder_is_kept`, `new_install_uses_relay`, `default_save_dir_is_relay`.
- [ ] **GREEN.** **Commit** `M3.6: new installs use Relay folder names`.

### Task 7 (M3.7): after a keystore reset, Relay offers "Reset and pair again" (B5)
Two commits, both `M3.7`.

**Part 1, Rust.** `crates/pb-identity/src/vault.rs`, `crates/pb-ffi/src/lib.rs`.
- `pub fn is_unseal_error(e: &io::Error) -> bool` (a typed inner error); `pub fn reset_identity(dir: &Path) -> io::Result<()>` removes `identity.sealed`, `identity.key`, `peers.txt`, `peers.tmp` and nothing else.
- pb-ffi: `PbError::IdentityUnreadable { reason }` from `start_node` when unsealing fails (`fn start_error(e: io::Error) -> PbError`); exported `fn reset_identity(data_dir: String) -> Result<(), PbError>`.
- [ ] **RED:** `unseal_failure_is_typed`, `missing_file_is_not_an_unseal_error`, `reset_keeps_everything_but_identity_and_pairings` (incoming/, timeline.json and logs/ survive; a new identity differs), `start_error_maps_unseal_failures`.

**Part 2, Android.** New `IdentityReset.kt` + test, `PbService.kt` (start failure ~487), `Recovery.kt`, `Prefs.kt` (`identityLost`), `KeystoreWrapper.kt` (`forget()`), `ui/home/Attention.kt` and `HomeTab.kt`, `StatusLine.kt`.
- `enum class StartFailure { RETRY, KEYSTORE_RESET }`, `fun startFailure(e: Throwable): StartFailure`; `fun shouldRecover(enabled: Boolean, running: Boolean, identityLost: Boolean)`; `Need.IdentityLost` first in `needs(...)`; `relayStatus(..., identityLost)` says "Relay can't open its keys".
- On `KEYSTORE_RESET`: save `identityLost`, cancel the recovery job, post one notification.
- "Reset and pair again" asks first ("Your PCs will see this phone as a new device: remove it on each PC, then pair again"), then: FFI reset, Keystore alias deleted, allow lists and last-PC prefs cleared, flag cleared, Relay on, pairing opened.
- [ ] **RED:** `startFailure` mapping, `shouldRecover`, `needs` order, status text.
- [ ] **Commit** each part `M3.7: ...`.

### Task 8 (M3.8): Relay 0.2.0 and this build still talk
**Files:** `crates/pb-protocol/src/lib.rs` (tests), `crates/pb-transport/src/tests.rs`, `docs/WIRE_PROTOCOL.md`.
- A test-only `HelloV020` (tags 1–3, as shipped in 0.2.0).
- [ ] **pb-protocol:** `hello_from_0_2_0_reads_with_defaults` (min 0, empty version, compatible) and `this_hello_reads_as_0_2_0` (the old struct sees version 0, the name and capabilities).
- [ ] **Transport:** `a_0_2_0_peer_still_talks`: a raw dial sends the 0.2.0-shaped `Hello` (hand-encoded envelope); the PC side gets a session, and clipboard text goes both ways.
- [ ] **Commit** `M3.8: Relay 0.2.0 and this build still talk`.

### Task 9 (M3.R): milestone check
- [ ] CI green on the branch head.
- [ ] One reviewer agent for the M3 diff against this plan's Review Focus; re-grade findings by effect; fix the serious ones test-first.
- [ ] THREAT_MODEL (keystore reset; `devices.txt` limits) and DECISIONS (compatibility rule, capabilities, settings versions, folder names). PROGRESS.md: M3 done, next step is the M4 plan. Commit and push.
