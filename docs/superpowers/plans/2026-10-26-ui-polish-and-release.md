# UI polish, Quick Settings style flyout and v0.1.0 release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Group and restyle the phone app and the PC windows, make PC drawing anti-aliased and animated, turn the tray flyout into Quick Settings style toggle tiles, and publish a private `v0.1.0` GitHub release.

**Architecture:** Pure models decide what is shown (group lists, tile states, easing), so they unit-test on the JVM and on any host; Win32 and Compose glue only draws them. PC shapes go through the one helper `fluent.rs`, so anti-aliasing is fixed once. The release reuses `scripts/make-portable.ps1` and the key in `%USERPROFILE%\.phonebridge-release`.

**Tech Stack:** Rust (windows crate 0.62, GDI + GDI+), Kotlin Jetpack Compose Material 3, PowerShell, `gh`.

**Spec:** `docs/superpowers/specs/2026-10-26-ui-polish-and-release-design.md`

## Global Constraints
- Groups, in this order, on both platforms: Connection, Camera, Screen, Files, Settings.
- Behavior, preference keys, stored setting names and wire protocol do not change. Window titles, crate names, package `dev.phonebridge.app` and the phone app label are not renamed.
- No `Co-Authored-By`/AI attribution; commit as Lin; never touch git config.
- Phone: always `adb -s 79KRFEJVPVOBQCHQ`; never `kill-server/usb/tcpip/root/reboot`, `svc`, `settings put`; no `KEYCODE_SLEEP`; no tethering/Wi-Fi/hotspot changes; no persistent logcat or uiautomator dumps; I do not tap consent/security prompts. Install only a freshly built APK and check its timestamp first.
- PC: window-only captures (never the full screen); no firewall or system setting changes without asking; stop `pb-tray.exe` before rebuilding it.
- One cargo/gradle command at a time. Gates before each commit: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and in `android/`: `./gradlew :app:testDebugUnitTest`, `./gradlew lintRelease`.
- The release key and `keystore.properties` are never printed, committed or copied into the repo. The GitHub repository stays private.

## Review Focus
- 200% display scale and a second monitor with another scale: no blur or clipped text (Task 5 capture at two scales).
- Long peer name or a very long last-event line in the flyout: ellipsis, no overlap (Task 6 test `long_text_is_clipped`).
- Animation timer left running after a window hides: must stop (Task 4 test `a_settled_value_needs_no_timer`).
- Phone in dark mode and at large font size: grouped cards stay readable (Task 3 device check).
- Keystore path missing on another machine: build falls back to debug signing (already in `build.gradle.kts`; Task 9 CI run proves it).

---

### Task 1: Phone settings groups (pure model)
**Files:** Create `android/app/src/main/java/dev/phonebridge/app/SettingsGroups.kt`; Test `android/app/src/test/java/dev/phonebridge/app/SettingsGroupsTest.kt`.
**Interfaces:**
- Produces: `enum class Group { Connection, Camera, Screen, Files, Settings }` (declaration order is display order); `fun groupFor(section: String): Group`; `fun groupTitle(g: Group): String`.
- The current section titles in `MainActivity.kt` are `Appearance`, `Clipboard`, `Sharing`, `Control from PC`, `Background`, `Devices`.
- [ ] **Step 1:** Write tests: `devices_belong_to_connection`, `control_from_pc_belongs_to_screen`, `sharing_belongs_to_files`, `appearance_clipboard_and_background_belong_to_settings`, `an_unknown_section_falls_back_to_settings`, `groups_are_listed_in_display_order` (asserts `Group.entries.map(::groupTitle)` equals `["Connection","Camera","Screen","Files","Settings"]`).
- [ ] **Step 2:** Run `./gradlew :app:testDebugUnitTest --tests '*SettingsGroupsTest*'` (from `android/`). Expected: compile failure, unresolved `groupFor`.
- [ ] **Step 3:** Implement the enum and the two functions in `SettingsGroups.kt`; camera settings (the per-PC "Always allow" revoke row) map to `Camera`.
- [ ] **Step 4:** Same command. Expected: PASS.
- [ ] **Step 5:** Commit `android: settings groups model`.

### Task 2: Phone settings screen regrouped
**Files:** Modify `MainActivity.kt` (the settings `LazyColumn`, sections near lines 551-669), `Theme.kt` if a shared group header is needed.
**Interfaces:** Consumes `Group`, `groupTitle` from Task 1.
- [ ] **Step 1:** Replace the six flat `Section(...)` headers by five group headers (`Section(groupTitle(g))`) in `Group` order; move each existing item under its group; no item changes behavior or key. Keep each group's items in a rounded card (`Card` with 16 dp corner radius, 16 dp padding, 12 dp gap between groups).
- [ ] **Step 2:** Add `animateContentSize()` to expandable rows and a 150 ms crossfade between `home` and `settings` in `App()`; no other transitions.
- [ ] **Step 3:** Run both Android gates. Expected: all pass.
- [ ] **Step 4:** Build the debug APK (`./gradlew :app:assembleDebug`), check `ls -l` shows a fresh timestamp, install with `adb -s ... install -r`, open Settings, look with a single window-scoped screenshot (`adb exec-out screencap -p`) in light and dark; no uiautomator.
- [ ] **Step 5:** Commit `android: settings grouped into Connection, Camera, Screen, Files, Settings`.

### Task 3: Phone home screen (new)
**Files:** Modify `MainActivity.kt` (`HomeScreen`); Create `HomeCards.kt` (pure `fun homeTiles(connected: Boolean, cameraOn: Boolean, screenOn: Boolean): List<HomeTile>`, `data class HomeTile(val group: Group, val label: String, val on: Boolean, val enabled: Boolean)`); Test `HomeCardsTest.kt`.
- [ ] **Step 1:** Tests: `tiles_are_disabled_while_not_connected`, `camera_tile_shows_on_while_streaming`, `tiles_follow_group_order` (Camera, Screen, Files), `send_file_tile_is_never_on`.
- [ ] **Step 2:** Run `--tests '*HomeCardsTest*'`. Expected: unresolved reference.
- [ ] **Step 3:** Implement `homeTiles`: tiles Camera, Screen, Clipboard (group Settings), Send file (Files); `enabled = connected`.
- [ ] **Step 4:** In `HomeScreen`, show a link card (existing `LinkIndicator`), then a 2-column tile grid drawn from `homeTiles` (Material 3 `Card`, accent container when `on`, 12 dp gaps), then the recent-items list. Tapping a tile calls the same action the existing buttons call; do not add new behavior.
- [ ] **Step 5:** Gates; install the fresh debug APK; check the home screen in light and dark via window-scoped screenshots. Large font size needs a system setting change, so it is checked only if the user changes it; otherwise record "large font not verified".
- [ ] **Step 6:** Commit `android: home screen with link card and tile grid`.

### Task 4: PC animation model (pure)
**Files:** Create `crates/pb-tray/src/ease.rs` (declared `mod ease;` in `main.rs`, host-independent); tests in the same file.
**Interfaces:** Produces `pub fn approach(current: f32, target: f32, dt_ms: u32) -> f32` (exponential ease, time constant 90 ms, snaps to `target` within 0.01) and `pub fn settled(current: f32, target: f32) -> bool`.
- [ ] **Step 1:** Tests: `moves_toward_the_target_without_overshoot`, `a_longer_step_moves_further`, `snaps_when_close`, `a_settled_value_needs_no_timer` (`settled(1.0, 1.0)`; `!settled(0.5, 1.0)`).
- [ ] **Step 2:** `cargo test -p pb-tray ease`. Expected: compile failure.
- [ ] **Step 3:** Implement both functions.
- [ ] **Step 4:** `cargo test -p pb-tray ease`. Expected: 4 passed.
- [ ] **Step 5:** Commit `tray: easing model for hover and press`.

### Task 5: Anti-aliased shapes in fluent.rs
**Files:** Modify `crates/pb-tray/Cargo.toml` (add `Win32_Graphics_GdiPlus` to the windows features), `crates/pb-tray/src/fluent.rs` (`round_rect` at line 218, `toggle` at 238, `button` at 270).
**Interfaces:** Keeps every existing signature; `round_rect(hdc, &r, fill, stroke, radius, dashed)` now draws with a GDI+ `Graphics` on `hdc`, smoothing mode anti-alias, pixel-offset half; started once with `GdiplusStartup` (guarded by `std::sync::Once`) and never shut down while the process runs.
- [ ] **Step 1:** Add a pure `pub fn corner_radius(s: f32, logical: i32) -> i32` (rounds `logical*s`, at least 1) with tests `radius_scales_with_dpi` (96 dpi: 8 to 8; 192 dpi: 8 to 16) and `radius_is_never_zero`; run `cargo test -p pb-tray corner_radius` (RED, then GREEN after adding it).
- [ ] **Step 2:** Re-implement the three drawing functions on GDI+ with the same colors and radii; dashed outline keeps its dash pattern.
- [ ] **Step 3:** Gates (`cargo fmt --check`, clippy, tests). Expected: all pass.
- [ ] **Step 4:** Rebuild and relaunch the tray, open the flyout and camera window, capture each window only; compare edges with the earlier capture (`scratchpad\win2.png`); repeat at a second display scale if the machine has one, otherwise record "second scale not verified".
- [ ] **Step 5:** Commit `tray: anti-aliased rounded shapes`.

### Task 6: Quick Settings style flyout
**Files:** Create `crates/pb-tray/src/flyout_tiles.rs` (pure); Modify `flyout.rs` (paint and hit-test, `Model` at line 46, `Action` at line 56), `ui.rs` where the flyout `Model` is filled.
**Interfaces:**
- Produces: `pub struct Tile { pub kind: TileKind, pub label: String, pub on: bool, pub enabled: bool }`, `pub enum TileKind { Camera, Screen, Clipboard, SendFile, Pair }`, `pub fn tiles(m: &Model) -> Vec<Tile>` (this order), `pub fn status_line(m: &Model, max_chars: usize) -> String`.
- Consumes the existing `flyout::Model` (`peer`, `clipboard_on`, `screen_on`, `camera_on`, `camera_fps`, `last`) and `flyout::Action`.
- [ ] **Step 1:** Tests in `flyout_tiles.rs`: `five_tiles_in_order`, `camera_tile_is_on_while_streaming`, `tiles_that_need_a_phone_are_disabled_without_one` (Camera, Screen, SendFile disabled when `peer` is `None`; Clipboard and Pair stay enabled), `status_line_names_the_phone`, `long_text_is_clipped` (a 200 character `last` yields at most `max_chars` characters ending with an ellipsis).
- [ ] **Step 2:** `cargo test -p pb-tray flyout_tiles`. Expected: compile failure.
- [ ] **Step 3:** Implement `tiles` and `status_line` (clip with `…`).
- [ ] **Step 4:** In `flyout.rs`, draw a 3-column tile grid from `tiles(&model)` (rounded 8 px logical radius, accent fill when `on`, dimmed when not `enabled`), group dividers with small captions (Connection, Camera/Screen, Files), a status line under the grid, hover/press eased with `ease::approach` driven by a 16 ms `SetTimer` that is killed when every value is settled; mouse clicks on tiles raise the same `Action` values as today (`ToggleClipboard`, `SendFiles`, `Pair`, `ControlPhone`, and the camera and screen actions that exist now). Drag and drop of files keeps working.
- [ ] **Step 5:** Gates; rebuild, relaunch, capture the flyout window only in idle, hover and camera-on states.
- [ ] **Step 6:** Commit `tray: flyout as Quick Settings style tiles`.

### Task 7: PC settings window groups
**Files:** Create `crates/pb-tray/src/settings_groups.rs` (pure); Modify `settings_window.rs`.
**Interfaces:** Produces `pub enum Group { Connection, Camera, Screen, Files, Settings }`, `pub fn groups() -> [(Group, &'static str); 5]` (display order and titles from Global Constraints), `pub fn group_of(option: &str) -> Group` for the option labels that exist in `settings_window.rs` today.
- [ ] **Step 1:** Tests: `groups_are_in_display_order`, `every_existing_option_has_a_group` (iterates the label list exported as `pub const OPTIONS: &[&str]` that Task 7 builds from the window's current labels), `unknown_options_fall_back_to_settings`.
- [ ] **Step 2:** `cargo test -p pb-tray settings_groups`. Expected: compile failure.
- [ ] **Step 3:** Implement; then lay the window's rows out under group captions using `group_of`, same spacing as Task 6, same stored setting names.
- [ ] **Step 4:** Gates; capture the settings window only.
- [ ] **Step 5:** Commit `tray: settings window grouped like the flyout`.

### Task 8: Camera and status windows use the shared spacing
**Files:** Modify `camera_window.rs`, `status.rs` (spacing constants only), `fluent.rs` (add `pub const GROUP_GAP: i32 = 16;`, `pub const ROW_PAD: i32 = 12;` logical).
- [ ] **Step 1:** Test `spacing_constants_are_on_the_4px_grid` in `fluent.rs` (both divisible by 4).
- [ ] **Step 2:** Run it (RED), add the constants (GREEN), use them in the two windows; keep row height `ROW_H` and window `H` consistent with the existing layout test in `camera_status.rs` if one exists.
- [ ] **Step 3:** Gates; capture the camera window only; commit `tray: camera and status windows share the spacing`.

### Task 9: Release v0.1.0
**Files:** Modify `android/app/build.gradle.kts` (`versionCode = 4`, `versionName = "0.1.0"`), `scripts/make-portable.ps1` (add `-Zip` switch that writes `dist\Relay-portable.zip`), `scripts/tests/make-portable.Tests.ps1`, `docs/DECISIONS.md`.
- [ ] **Step 1:** Extend `make-portable.Tests.ps1` first: with `-Zip` the zip exists and contains `Relay/Relay.exe`; run it (RED), add the switch with `Compress-Archive` (GREEN).
- [ ] **Step 2:** Bump the version; run all gates, push `phase3`, and wait for CI green on all four jobs (the unsigned fallback proves the missing-key case).
- [ ] **Step 3:** Build `./gradlew :app:assembleRelease`; verify the signer with `apksigner verify --print-certs` shows `CN=Phone Bridge, O=Lin`; rebuild the tray and run `make-portable.ps1 -Zip`.
- [ ] **Step 4:** Phone: uninstall the debug app (`adb -s ... uninstall dev.phonebridge.app`, clears pairing; the user agreed), install the signed APK, open it, and pair with the PC with the user (the user taps Pair and confirms the code; I do not tap consent prompts). Then repeat the camera check from before (stream at 29 to 30 fps).
- [ ] **Step 5:** Write `docs/DECISIONS.md` Phase 6 (UI results, the Windows Quick Settings limit, signing, the uninstall note) and the release notes file from it; commit.
- [ ] **Step 6:** Ask the user for the final yes (a release is outward-facing), then `git tag v0.1.0`, push the tag, and `gh release create v0.1.0 app-release.apk Relay-portable.zip --notes-file ... --repo Lin-2352/phone-bridge` (repository is private; assets named `PhoneBridge-0.1.0.apk` and `Relay-portable-0.1.0.zip`). Confirm with `gh release view v0.1.0`.

### Task 10: Whole-branch review (runs before Task 9 Step 6)
- [ ] One Opus review of `git diff 12496f5..HEAD` with the Review Focus section; fix Critical and Important only, each test first; re-run gates and CI before Task 9 Step 6.

## Deferred (not in this plan)
Real Windows Quick Settings tile (not possible for apps), renaming titles and the package, new features.
