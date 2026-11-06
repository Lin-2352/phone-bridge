# M5 PC native redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:executing-plans (inline, TDD, one reviewer at the end of the milestone). Steps use checkbox (`- [ ]`) syntax. Run `advisor` where a step says **ADVISOR**. Run several Monitors at once (gates, CI by full SHA); never wait on one job alone.

**Goal:** redraw every Relay PC surface on the M4 design tokens, with Mica, Acrylic and Relay glass materials, solid fallbacks, token motion, Narrator support for the flyout, an offscreen snapshot CI job, and re-measured budgets.

**Architecture:** one small pure module (`look.rs`) decides which material a surface gets from the look setting and the system state, and the Win32/Direct2D code only calls it. M4's `tokens.rs` and `tokens_rules.rs` supply the values and the opaque/motion rules. Each surface keeps its draw function and its existing tests.

**Tech Stack:** Rust 1.99.1, `windows` 0.62 (DWM, Direct2D, DirectWrite, UIA, WIC), the M4 token generator.

**Spec:** `docs/v2/SPEC.md` lines 254–269 and key choice 2 (lines 74–76); progress rows M5.1–M5.R in `docs/v2/PROGRESS.md`; design rules in `docs/v2/design/PRINCIPLES.md`.

## Global Constraints
- MIT OR Apache-2.0; no GPL/AGPL code; icons from Fluent System Icons (MIT).
- "The PC can only ask"; every live share keeps its visible Stop. No consent is weakened.
- `PROTOCOL_VERSION` stays 0; M5 has no wire change.
- PC budgets: idle tray 0 CPU, ≤ 40 MB working set, ≤ 15 MB private; main window open ≤ 70 MB private. Effects run only while visible.
- WCAG AA over pure white and pure black: text 4.5:1, icons and control boundaries 3:1. Glass that carries text has alpha ≥ 0.70, icon-only glass ≥ 0.50 (PRINCIPLES).
- Honour reduced motion, reduced transparency, high contrast and DPI scale; every control has a Narrator name.
- Never rename the pipe/mutex names, the AUMID `PhoneBridge.Tray`, the DPAPI entropy, `%ProgramData%\PhoneBridge\camera`.
- New strings go in `strings.rs`, English.
- One commit per task, subject starting with the task ID; the same commit ticks `docs/v2/PROGRESS.md` and moves "Resume here". Push after every task. No attribution lines of any kind.
- Gates before every commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --no-fail-fast`, and from M5.10 also `cargo clippy -p pb-tray --features test-hooks --all-targets -- -D warnings`. The two `pb-platform-win` clipboard tests may fail locally only while Relay runs (CI decides). CI is checked by full SHA under a Monitor.

## Review Focus
1. **DWM refuses a backdrop** (Windows 10, before 22H2, Remote Desktop): every surface draws solid with readable text. Tests: `old_windows_without_backdrops_is_solid` (M5.1) and a hidden-window draw test per surface.
2. **Transparency effects switched off while Relay is open:** the backdrop rebuilds as solid without a restart. Test: `on_theme_change_rebuilds_backdrops` (M5.1).
3. **High contrast on:** system colours, no glass. Tests: `high_contrast_palette_uses_system_colours`, `high_contrast_is_solid_everywhere` (M5.1).
4. **Windows animations off:** no spring and no timer left running. Tests: `reduced_motion_makes_no_spring`, `the_dialog_timer_stops_when_settled` (M5.2).
5. **Idle with nothing animating:** 0 CPU, measured by `scripts/pc-budget.ps1`; test `budget_table_flags_each_overrun` (M5.11).

## Rulings already made
- Ruling: this plan (M5.0) is written before M4.9 and M4.R finish, because M4.9 waits on the owner's open preview tab and the owner asked for unblocked work to continue; the rulings marked "made in M5.0 ahead of M4.R" are recorded in PRINCIPLES and DECISIONS by M4.R, and Task M5.0a re-checks them — cost if wrong: one rewrite of this file.
- Ruling (made in M5.0 ahead of M4.R; M4.R records it): the amber token is renamed `attention` and `Palette.accent`/`on_accent` become `primary`/`on_primary` in M5.1 — PRINCIPLES says one accent (`primary`); the PC already draws cobalt at all 22 + 11 reads and never draws amber — if wrong, a rename-only revert.
- Ruling (made in M5.0 ahead of M4.R; M4.R records it): springs (ease.rs) are used only where motion follows the hand (a released drag, a fling); transitions use `tokens::easing` with `tokens::motion` durations — SPEC M5.2 asks for springs, research found no published Windows springs for transitions — if wrong, swap curves in `ease.rs` only.
- Ruling: "quick panel" on the PC means the tray flyout (Relay's Quick-Settings-style panel) — the PC has no other — if wrong, UIA for another window is a later task.
- Ruling: the Transfers page is new (the main window has none today).
- Ruling: snapshot renders use WARP — deterministic pixels on CI — hardware differences are not tested.
- Ruling: a missing Segoe UI Variable falls back to Segoe UI in the product, not only in tests — the README promises Windows 10 2004, which lacks the variable font — none.
- Ruling: lite mode is Android-only; on the PC `Modes.lite` stays false — SPEC defines lite for Android.

## Today in the code (checked 2026-11-06; paths under `crates/pb-tray/src/`)
- Backdrops: `fluent::set_backdrop` (fluent.rs:825), `fluent::frame(hwnd, mica)` (:800, every caller passes false), `d2d::Backdrop { Mica, Opaque }` (d2d.rs:71), `choose` (:80), `Surface::build` (:321, hard-codes `DWMSBT_MAINWINDOW` at :336), `make_window` (:563, used only by flyout.rs:968 and main_window.rs:2145). Other surfaces call `Surface::new(hwnd, Backdrop::Opaque)`: dialog_window.rs:761, menu.rs:450, qr_window.rs:138, transfer_window.rs:240, status.rs:72, screen_window.rs:717, camera_window.rs:221.
- System state: `windows_dark` (fluent.rs:98), `transparency_on` (:103), `reduced_motion` (:535). High contrast is never detected. `WM_SETTINGCHANGE` is handled only for "ImmersiveColorSet" (ui.rs:2979). `settings.rs:32` has only `theme: u8`.
- Palette (fluent.rs:117–157): `accent` holds `primary`; `amber` holds the amber token and is never drawn; no `outline_strong`; `hover` hard-coded.
- Glass alpha below the 0.70 floor: flyout `MICA_ALPHA = 0.6` (flyout.rs:65); main window content layer 0.5 (main_window.rs:1256).
- Motion: ease.rs has exponential `approach`, `Anim`, `STANDARD = (0.2,0,0,1)` which differs from `tokens::easing::STANDARD`, and no springs. Frame timers: main window `FRAME_TIMER` + `sync_timer`; flyout `ANIM_TIMER`; the dialog timer runs for the dialog's whole life (dialog_window.rs:797, killed :660); `widgets::skeleton` pulses forever and ignores reduced motion (widgets.rs:301–305).
- Fonts: "Segoe UI Variable Text/Display", "Segoe Fluent Icons", "Consolas" (fluent.rs:241–256); nothing checks that a font exists. `d2d::Surface` renders only to an HWND swap chain.
- Surfaces: flyout (flyout.rs, 8 tests; flyout_tiles.rs, 17), main window (`Page { Home, Timeline, Devices, Settings }`, no Transfers; titlebar.rs 4 tests; pages/*), qr_window.rs (always black on white, opaque), dialog_window.rs (16 tests), screen_window.rs (bars use emoji labels at :675/:677; 0 tests), toast.rs/toast_xml.rs (WinRT) and status.rs (D2D pill), transfer_window.rs, menu.rs/menu_model.rs, camera_window.rs (hard-coded 0x00B050/0xE81123/0xFFB900).
- UIA: `uia.rs` serves the main window only, with one global `SNAPSHOT` (:65); `a11y::tree` (a11y.rs:36). The flyout has no UIA.
- CI: job `rust` has no features; `windows-zip` scans the exe for the hard-coded strings `'test-auto-accept','test-audio-synth'`; the only cargo feature is `test-hooks`.
- Budgets: working set is logged (`applog_sys::working_set_bytes`), but budgets are written in private bytes; no PC budget script exists.

## Shared interfaces (defined in M5.1)
```rust
// crates/pb-tray/src/look.rs (pure; compiles and tests on Linux)
pub enum Look { Native, Relay }
pub enum SurfaceKind { MainWindow, Flyout, Menu, Dialog, Qr, Transfers, StatusPill, ScreenBars, Camera }
pub struct SysState { pub dark: bool, pub transparency_on: bool, pub high_contrast: bool, pub animations_on: bool, pub dwm_backdrop_ok: bool }
pub enum Material { Mica, Acrylic, RelayGlass, Solid }
pub fn material_for(kind: SurfaceKind, look: Look, sys: SysState) -> Material
pub fn modes(sys: SysState) -> crate::tokens_rules::Modes
pub fn glass(look: Look, dark: bool, sys: SysState) -> crate::tokens::Material
pub fn rebuild_needed(old: SysState, new: SysState) -> bool
pub const TEXT_GLASS_MIN_ALPHA: f32 = 0.70;
pub const ICON_GLASS_MIN_ALPHA: f32 = 0.50;
```
`material_for`: any of `!transparency_on`, `high_contrast`, `!dwm_backdrop_ok` gives `Solid`. Native: `MainWindow` = `Mica`; `Flyout`, `Menu`, `Dialog` = `Acrylic`; everything else `Solid`. Relay: those four = `RelayGlass` (DWM Acrylic base plus Direct2D tint, highlight and grain); everything else `Solid`. `Qr` is always `Solid`.

---

### Task M5.0a: Refresh at start
- [ ] Re-read `docs/v2/design/PRINCIPLES.md`, the M4.R DECISIONS entry "Design tokens" and `design/tokens.toml`. Confirm each ruling marked "made in M5.0 ahead of M4.R" matches what M4.R wrote; if not, record a new ruling and edit this plan before M5.1.
- [ ] Check the M4.R type-ramp decision (PRINCIPLES says 18/24, `tokens.toml` has 12/14/16/20/28) and use whatever `tokens::ty` contains.
- [ ] Check M4.9 token changes (`git log --grep '^M4.9'`) and re-run `cargo test -p gen-tokens`. Known M4.9 changes: the amber `accent` token is now `#955700` in light (it was `#C9820F`, 3.1:1) and new `success_text`/`warning_text`/`danger_text` tokens exist for text on a status tint; M5.1's rename `accent` → `attention` must carry these three new names across unchanged. Badge-style status chips on the PC use the `*_text` colours for their text and the status colour for the icon.
- [ ] M4.9 R-15: `tokens::motion` is 120/200/300 ms, not Microsoft's published 83/167/250/333 ms. Decide here: either add PC duration tokens (83, 167, 250, 333) to `design/tokens.toml` through the gen-tokens tests and use them for transitions, or change the ruling text to say 120/200/300 are Relay's own durations. Record the decision as a `Ruling:`.
- [ ] Re-check the "Today in the code" line numbers with Grep; the facts were verified on 2026-11-06.

### Task M5.1: Materials, system state and fallbacks
**Files:**
- Create `crates/pb-tray/src/look.rs`; add `mod look;` in `main.rs`.
- Modify `d2d.rs`: `Backdrop` gains `Acrylic`; `choose`/`build` take a `DWM_SYSTEMBACKDROP_TYPE` from the material instead of the hard-coded `DWMSBT_MAINWINDOW`.
- Modify `fluent.rs`: add `pub fn high_contrast() -> bool` (`SPI_GETHIGHCONTRAST`, `HCF_HIGHCONTRASTON`), `pub fn sys_state() -> look::SysState`, `pub fn hc_palette(get: impl Fn(i32) -> u32) -> Palette` (COLOR_WINDOW, WINDOWTEXT, HIGHLIGHT, HIGHLIGHTTEXT, GRAYTEXT, HOTLIGHT, BTNFACE), `Palette.outline_strong`, rename `accent`/`on_accent`/`amber` to `primary`/`on_primary`/`attention`; `on_theme_change()` rebuilds backdrops when `look::rebuild_needed` is true.
- Modify `design/tokens.toml`, `tools/gen-tokens/src/lib.rs`, regenerate `tokens.rs`, `Tokens.kt`, `design/floot/base.css`; update Kotlin users of the old name (`ui/Theme.kt:18-19`, `RelayTint.Accent`).
- Modify `ui.rs`: `WM_SETTINGCHANGE` also handles `SPI_SETHIGHCONTRAST`, "WindowsThemeElement"; handle `WM_SYSCOLORCHANGE` and `WM_THEMECHANGED`.
- Modify `settings.rs` (`look: u8`, `SETTINGS_VERSION` 1 → 2 with `look=0`), `settings_groups.rs` and `pages/settings.rs` ("Appearance → Look": Native glass / Relay glass; update the KEYS count).
- Replace flyout `MICA_ALPHA` and the main-window 0.5 layer with `look::glass(...)`, alpha ≥ `TEXT_GLASS_MIN_ALPHA` for text layers; every `Surface::new(hwnd, Backdrop::Opaque)` site takes `material_for(kind, …)`.
- Tests: `look.rs`, `fluent.rs`, `settings.rs`, `tools/gen-tokens` drift tests.

**Interfaces:** Consumes `tokens::{NATIVE_LIGHT, NATIVE_DARK, RELAY_LIGHT, RELAY_DARK}` and `tokens_rules::{Modes, resolve}`. Produces everything in "Shared interfaces".

- [ ] **Step 1 (ADVISOR):** before changing `d2d::build` (every window's creation path), call `advisor`.
- [ ] **Step 2: write failing tests.** In `look.rs`: `native_main_window_is_mica`, `native_flyout_menu_dialog_are_acrylic`, `relay_layers_are_relay_glass`, `transparency_off_is_solid_everywhere`, `high_contrast_is_solid_everywhere`, `old_windows_without_backdrops_is_solid`, `qr_is_always_solid`, `focus_ring_is_3_to_1` (the focus-ring colour, `Palette.primary`, has `contrast ≥ 3.0` against `card` and `bg` in light, dark and high contrast; the M4.9 review found the prototype's rings at 1.2:1, the shipping tray already draws cobalt, this test keeps it so), `text_layers_meet_the_glass_floor` (for every look × dark × `SysState`, the text-layer alpha is ≥ 0.70 or the material is `Solid`), `on_theme_change_rebuilds_backdrops` (`rebuild_needed` true when transparency, high contrast, dark or backdrop support changes; false otherwise). In `fluent.rs`: `high_contrast_palette_uses_system_colours`. In `settings.rs`: `v1_settings_gain_native_look`.
- [ ] **Step 3:** `cargo test -p pb-tray look::` — Expected: FAIL (module missing).
- [ ] **Step 4: implement** `look.rs`, then the renames (token first through gen-tokens, `cargo run -p gen-tokens`), then the Windows wiring.
- [ ] **Step 5:** `cargo test -p pb-tray` and `cargo test -p gen-tokens` — Expected: PASS.
- [ ] **Step 6:** gates, Android compile check (`Tokens.kt` changed: `gradlew :app:testDebugUnitTest`), PROGRESS, commit `M5.1: materials, system state and fallbacks`, push, CI by full SHA.

### Task M5.2: Motion
**Files:** modify `ease.rs`, `dialog_window.rs`, `widgets.rs`, `flyout.rs`, `main_window.rs`.

**Interfaces:** Produces `pub struct SpringAnim { x: f32, v: f32, target: f32, spring: tokens::Spring }` with `new(from, to, s) -> Self`, `step(&mut self, dt_ms: u32) -> f32` (semi-implicit Euler at sub-steps ≤ 4 ms; ω = √stiffness, ζ = damping_ratio), `settled(&self) -> bool` (|x−target| < 0.001 and |v| < 0.01); `pub fn needs_frame_timer(anim_running: bool) -> bool`. `ease::STANDARD` takes its value from `tokens::easing::STANDARD` (delete the stale comment at ease.rs:39).

- [ ] **Step 1: failing tests:** `a_spring_reaches_its_target_and_settles`, `the_effects_spring_never_overshoots` (ζ = 1.0), `an_emphasised_spring_overshoots_a_little` (passes the target by under 10%), `standard_matches_the_token_curve`, `reduced_motion_makes_no_spring` (via `tokens_rules::spring_or_cut`), `the_dialog_timer_stops_when_settled`, `skeleton_is_still_with_reduced_motion`.
- [ ] **Step 2:** `cargo test -p pb-tray ease::` — Expected: FAIL.
- [ ] **Step 3: implement.** Springs only for released drags (nav track, scrollbar thumb); transitions keep `tokens::easing` + `tokens::motion`. The dialog frame timer runs only while animating; a separate 500 ms timer polls `is_alive`. `skeleton` is static under reduced motion.
- [ ] **Step 4:** tests PASS; gates; commit `M5.2: token motion and timers that stop`.

### Task M5.3: Tray flyout
**Files:** modify `flyout.rs`, `flyout_tiles.rs`; test in `flyout.rs`.
**Interfaces:** Consumes `look::material_for(SurfaceKind::Flyout, look, sys)` and `look::glass`. Produces nothing new.
- [ ] **Step 1: failing tests** `status_text_sits_on_an_opaque_layer` (the flyout's warning line at `flyout.rs:347-348` and every status word or badge text is drawn on an opaque card, never directly on the glass backdrop; M4.R review I-1), `off_toggle_has_a_3_to_1_boundary` (`fluent::toggle` with `on = false, enabled = true` draws its edge with `Palette.text2`/`outline_strong`, and that colour has `contrast ≥ 3.0` against the track fill and the card in light and dark; M4.9 R-2) and `flyout_text_sits_on_an_opaque_layer_or_meets_the_floor`: for every look × dark × `SysState`, the flyout's text-layer alpha is `1.0` when the material is `Solid`, otherwise ≥ `look::TEXT_GLASS_MIN_ALPHA`; and tile boundaries use `Palette.outline_strong`.
- [ ] **Step 2:** `cargo test -p pb-tray flyout::` — Expected: FAIL (alpha is still `MICA_ALPHA = 0.6`).
- [ ] **Step 3: implement.** Delete `MICA_ALPHA`; draw the backdrop from `look::glass`; controls on opaque cards; radius 8, controls 4; layout from Floot `/pc` "Tray flyout".
- [ ] **Step 4:** `cargo test -p pb-tray flyout::` — Expected: PASS, with the 8 flyout and 17 tile tests unchanged.
- [ ] **Step 5:** gates, PROGRESS, commit `M5.3: tray flyout on tokens`, push, CI by full SHA.

### Task M5.4: Main window pages including Transfers
**Files:** modify `main_window.rs`, `pages/mod.rs`, `pages/{home,devices,settings,timeline}.rs`; create `pages/transfers.rs`.
**Interfaces:** Produces `Page::Transfers` (rail order Home, Devices, Transfers, Timeline, Settings), `pub struct TransfersModel { pub rows: Vec<TransferRow> }`, `pub fn pages::transfers::draw(c: &mut Canvas, r: RectF, view: RectF, m: &TransfersModel, hits: &[Hit])`. Consumes the transfer state in `transfers.rs`.
- [ ] **Step 1: failing tests:** `rail_has_five_pages_in_order` (labels equal `["Home","Devices","Transfers","Timeline","Settings"]`), `transfers_page_lists_active_then_finished` (running rows precede finished rows; finished are newest first), `an_empty_transfers_page_says_so` (the empty model yields the empty-state text from `strings.rs`).
- [ ] **Step 2:** `cargo test -p pb-tray pages::` — Expected: FAIL (no `Page::Transfers`).
- [ ] **Step 3: implement.** Glass (Mica or Relay) behind the rail only, panes opaque; restyle the four existing pages on tokens; keep all existing page tests (home 23, devices 4, settings 19, timeline 14).
- [ ] **Step 4:** `cargo test -p pb-tray` — Expected: PASS.
- [ ] **Step 5:** gates, PROGRESS, commit `M5.4: main window pages including Transfers`, push, CI.

### Task M5.5: Pairing and QR
**Files:** modify `qr_window.rs`, `dialog_window.rs` (SAS dialog).
- [ ] **Step 1: failing test** `qr_keeps_a_white_quiet_zone_in_dark_mode`: with `dark = true` the QR card is `0xFFFFFF` with a quiet zone ≥ 4 modules on all sides. `every_line_fits_the_window_at_100_percent` stays.
- [ ] **Step 2:** `cargo test -p pb-tray qr_window::` — Expected: FAIL.
- [ ] **Step 3: implement** the solid white card and token text styles.
- [ ] **Step 4:** same command — Expected: PASS.
- [ ] **Step 5:** gates, PROGRESS, commit `M5.5: pairing and QR on tokens`, push, CI.

### Task M5.6: Consent dialogs
**Files:** modify `dialog_window.rs`.
- [ ] **Step 1: failing test** `dialog_buttons_have_3_to_1_boundaries`: every button boundary colour against the dialog surface has `contrast ≥ 3.0` in light and dark, and only "Allow" is the primary-filled button.
- [ ] **Step 2:** `cargo test -p pb-tray dialog_window::` — Expected: FAIL.
- [ ] **Step 3: implement** `material_for(Dialog)` and `outline_strong` boundaries. Do not change `enter_never_answers_a_question_but_accepts_a_notice` (security behaviour).
- [ ] **Step 4:** same command — Expected: PASS, 16 existing tests unchanged.
- [ ] **Step 5:** gates, PROGRESS, commit `M5.6: consent dialogs on tokens`, push, CI.

### Task M5.7: Screen-window chrome
**Files:** modify `screen_window.rs`. **Interfaces:** Produces `pub struct BarItem { pub glyph: char, pub label: &'static str, pub essential: bool }` and `pub fn bar_labels(width: f32) -> Vec<BarItem>`.
- [ ] **Step 1: failing tests:** `no_emoji_in_bar_labels` (no label or glyph in any width is above U+1F000), `narrow_bars_keep_stop_visible` (at 240 px the list still contains the item with `essential = true` for Stop).
- [ ] **Step 2:** `cargo test -p pb-tray screen_window::` — Expected: FAIL.
- [ ] **Step 3: implement** with Segoe Fluent Icons glyph consts in place of the two emoji at screen_window.rs:675 and :677, and token colours.
- [ ] **Step 4:** same command — Expected: PASS.
- [ ] **Step 5:** gates, PROGRESS, commit `M5.7: screen window chrome without emoji`, push, CI.

### Task M5.8: Toasts, status pill, camera window colours
**Files:** modify `status.rs`, `toast_xml.rs`, `strings.rs`, `camera_window.rs`.
- [ ] **Step 1: failing tests:** `status_pill_uses_token_colours` (pill fill and text equal `Palette` values, no literals), `camera_window_has_no_literal_colours` (`include_str!("camera_window.rs")` contains none of `0x00B050`, `0xE81123`, `0xFFB900`).
- [ ] **Step 2:** `cargo test -p pb-tray status:: camera_window::` — Expected: FAIL.
- [ ] **Step 3: implement.** Camera colours become `success`, `danger`, `attention`; XML toasts take real copy from `strings.rs`; the 12 `toast_xml` tests stay green.
- [ ] **Step 4:** same command — Expected: PASS.
- [ ] **Step 5:** gates, PROGRESS, commit `M5.8: toasts and pill on tokens`, push, CI.

### Task M5.9: Narrator for the flyout
**Files:** modify `uia.rs`, `a11y.rs`, `flyout.rs`.
**Interfaces:** `uia::publish(hwnd: isize, s: Snapshot) -> bool`, `uia::root_for(hwnd, wparam, lparam) -> LRESULT` (per-window `HashMap<isize, Snapshot>`); `pub fn a11y::flyout_tree(tiles: &[Tile], m: &Model) -> Vec<Node>`.
- [ ] **Step 1: failing tests:** `flyout_tree_names_every_tile_and_its_state` (one node per tile, role toggle, name equals the tile label, `toggled` equals `Tile.on`), `two_windows_keep_their_own_trees` (publishing for hwnd 1 does not change hwnd 2's tree).
- [ ] **Step 2:** `cargo test -p pb-tray a11y:: uia::` — Expected: FAIL.
- [ ] **Step 3: implement.** The flyout answers `WM_GETOBJECT` and publishes its tree after each model change.
- [ ] **Step 4:** same command — Expected: PASS.
- [ ] **Step 5:** gates, PROGRESS, commit `M5.9: Narrator support for the flyout`, push, CI.

### Task M5.10: Offscreen snapshot CI job
**Files:** modify `d2d.rs`, `main.rs`, `fluent.rs`, `.github/workflows/ci.yml`; create `crates/pb-tray/snapshot/wallpaper.png`, `crates/pb-tray/snapshot/golden/`, `tools/snapshot-diff/`.
**Interfaces:** `pub fn d2d::offscreen(w: u32, h: u32) -> Result<Offscreen, String>` (WARP); `pub fn fluent::font_face(f: Font) -> &'static str`; `pub fn check_fonts(have: &dyn Fn(&str) -> bool) -> Result<(), String>`; `tools/snapshot-diff` CLI `snapshot-diff <actual-dir> <golden-dir>` exiting 1 beyond tolerance (per-channel 8, ≤ 0.5% of pixels differing).
- [ ] **Step 1: failing tests:** `snapshot_names_cover_every_surface_look_and_mode` (the name list equals 9 surfaces × 2 looks × 2 modes, no duplicates), `diff_accepts_within_tolerance_and_rejects_beyond` (an image with 8-level noise passes, one with a 9-level shift on 1% of pixels fails), `missing_fonts_fail_fast` (`check_fonts` returns `Err("missing font: …")` when neither text face exists).
- [ ] **Step 2:** `cargo test -p pb-tray snapshot::` and `cargo test -p snapshot-diff` — Expected: FAIL.
- [ ] **Step 3: implement.** `--snapshot <dir>` (only under `cfg(feature = "test-hooks")`) draws every surface over the fixed wallpaper and exits 3 with `missing font: <name>` per `check_fonts`. CI job `snapshots` (windows-latest): print `Get-ChildItem C:\Windows\Fonts\Segoe*`, `cargo build -p pb-tray --features test-hooks`, run `--snapshot`, `snapshot-diff`, upload artifact. Add `cargo clippy -p pb-tray --features test-hooks --all-targets -- -D warnings`; add `'--snapshot'` to the `windows-zip` release string scan. Check `cargo deny` for any new image crate licence.
- [ ] **Step 4:** unit tests PASS locally; push once without goldens to get the artifact, read several PNGs with the Read tool, commit them as goldens, push again — Expected: the `snapshots` job green.
- [ ] **Step 5:** gates, PROGRESS, commit `M5.10: offscreen snapshot job`, CI by full SHA.

### Task M5.11: Budgets re-measured
**Files:** create `scripts/pc-budget.ps1`; modify `tools/pb-bench` (threshold check), `ipc.rs` (an `--open-main` pipe command with a test if none exists), `docs/DECISIONS.md`.
**Interfaces:** `pub fn check_budget(row: &BudgetRow) -> Vec<Overrun>` in `tools/pb-bench`.
- [ ] **Step 1: failing test** `budget_table_flags_each_overrun`: rows with idle WS 41 MB, idle private 16 MB, window private 71 MB, idle CPU 0.06 s each produce exactly one `Overrun` naming the column; a row at the limits produces none.
- [ ] **Step 2:** `cargo test -p pb-bench` — Expected: FAIL.
- [ ] **Step 3: implement** the check and the script: start `target\release\pb-tray.exe --dir <temp> --port 47250` (never the owner's Relay), idle 60 s, sample `WorkingSet64`, `PrivateMemorySize64` and the `TotalProcessorTime` delta, open the main window, sample after 10 s, close it, wait 40 s, sample again; exit non-zero on any overrun (idle WS > 40 MB, idle private > 15 MB, window private > 70 MB, idle CPU delta ≥ 0.05 s).
- [ ] **Step 4:** `cargo test -p pb-bench` — Expected: PASS; run the script on the release build and write the table into DECISIONS "PC budgets after M5".
- [ ] **Step 5:** gates, PROGRESS, commit `M5.11: budgets re-measured`, push, CI.

### Task M5.R: Milestone review
- [ ] One reviewer (opus) over the M5 diff against the Review Focus; **ADVISOR** when re-grading findings; fix serious findings test-first.
- [ ] Docs: DECISIONS (materials per surface, fallbacks, spring use, font fallback, quick panel = flyout), THREAT_MODEL ("no change"; the snapshot flag is `test-hooks` only and scanned out of release), WIRE_PROTOCOL (no change).
- [ ] CI green by full SHA including the `snapshots` job; **ADVISOR** before declaring M5 done. Commit `M5.R: milestone review`.

## Self-Review
- Spec coverage: materials (M5.1), motion (M5.2), all six screens (M5.3–M5.8), Narrator (M5.9), snapshot job (M5.10), budgets (M5.11), review (M5.R).
- Type consistency: `look::SysState`, `Material`, `material_for`, `glass`, `rebuild_needed` are defined once in M5.1 and used unchanged by M5.3–M5.8 and M5.10.
- Proportion: bodies are left to the implementer; interfaces and test names are fixed here.
