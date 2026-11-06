# M4 Design System and Floot Prototypes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (the program's budget mode: inline, TDD, one reviewer at the end). Steps use checkbox (`- [ ]`) syntax.

**Goal:** one source of truth for how Relay looks and moves (colours, materials, elevation, springs, easing, modes), generated into Rust, Kotlin and CSS, proven to meet WCAG AA, and prototyped in Floot for every PC and Android screen before M5 and M6 build them.

**Architecture:** `design/tokens.toml` is read by `tools/gen-tokens`, which renders `crates/pb-tray/src/tokens.rs`, `android/.../ui/Tokens.kt` and `design/floot/base.css`. Contrast and mode rules are pure functions with tests in the generator; the platform code only calls the generated functions. The research and the principles are documents with cited sources.

**Tech Stack:** Rust 1.99.1 (`gen-tokens`, `toml`), Kotlin (generated `Tokens.kt`), CSS, the Floot connector for prototypes.

**Spec:** `docs/v2/SPEC.md`, milestone M4 (lines 228-253). Progress: `docs/v2/PROGRESS.md`.

## Global Constraints
- **Licence:** MIT OR Apache-2.0. Studying other apps' UIs is fine; no code from GPL or AGPL projects. Icons: Fluent System Icons (MIT) and Material Symbols (Apache-2.0).
- **Accessibility:** WCAG AA, 4.5:1 for text and 3:1 for icons over the worst-case backdrop (pure white and pure black). Honour reduced motion, reduced transparency, high contrast and font scale.
- **Modes:** light, dark or system; reduced transparency; reduced motion; lite.
- **Budgets:** effects run only while visible; no new idle CPU.
- **Process:** one commit per task with the task ID in the subject; PROGRESS.md ticked in the same commit; push to `cloud/v2-upgrade` after each; no pull request; no AI attribution; rulings as `Ruling: <what> — <why> — <cost if wrong>` in the commit message.
- **Gates before each commit:** `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace --no-fail-fast`; for Kotlin changes `gradlew :app:testDebugUnitTest :app:lintRelease`.
- **Floot:** the project stays a private preview. Never call `publish_app`.

## Review Focus
1. **Reduced transparency, high contrast or lite mode:** materials are fully opaque, blur and grain are 0, and text is still AA (M4.3 `reduced_transparency_is_opaque`, M4.4 `every_mode_meets_aa`).
2. **Relay glass over the worst backdrop** (pure white and pure black): text and muted text 4.5:1, and the colours icons are drawn in (`text`, `text_muted`, `primary`) 3:1, in both looks, light and dark (M4.4 `glass_text_is_aa_over_white_and_black`, `icons_are_3_to_1`). `outline` is a border, not an icon colour, and is not held to 3:1 (a deliberate choice: it would turn every border dark grey); whether component boundaries get WCAG 1.4.11 is a separate ruling in M4.2.
3. **Generated files drifting from the TOML:** the checked-in `tokens.rs`, `Tokens.kt` and `base.css` equal a fresh render (the drift tests).
4. **A missing or malformed token key:** the error names the key (`a_missing_material_key_is_an_error_naming_it`).
5. **Reduced motion:** springs are cut to a cross-fade, never a shorter spring (`reduced_motion_cuts_springs`).

---

### Task M4.1: research pass
**Files:** create `docs/v2/design/RESEARCH.md`.
- [ ] Run the deep-research skill over the SPEC's reference sets: Windows (Settings, Phone Link, Quick Share for Windows, PowerToys, Fluent 2), Android (Pixel Android 16 Settings and Quick Settings, Files by Google, Quick Share, LocalSend, Nothing OS), glass (iOS 26 and macOS Tahoe, including Liquid Glass's readability lessons).
- [ ] The deep-research skill writes under the main worktree's `research_notes/` and `reports/` (untracked on `feature/audio-sharing`); never `git add` anything there. Condense the result into this worktree's `RESEARCH.md`.
- [ ] At least 3 references per screen family. Every decision (material values, radii, spacing, type scale, springs, easing, accent) cites 2 sources: a guideline and a real app.
- [ ] A fresh agent opens every cited URL and marks it confirmed or dropped; replace or reopen the decision for each dropped one.
- [ ] Commit `M4.1: design research with cited decisions`.

### Task M4.2: principles and the anti-AI-look checklist
**Files:** create `docs/v2/design/PRINCIPLES.md`.
- [ ] The checklist verbatim from SPEC lines 235-241: no generic purple-blue gradients, emoji icons, or same-size rounded cards everywhere; real copy; a 4/8-pt grid with optical alignment; one accent colour; hierarchy from size and weight; motion only where it means something.
- [ ] The M4.1 decisions as one-line principles with their citations, and the AA rules above.
- [ ] Rule on "one accent colour" against the palette, which has `primary` (cobalt), `accent` (amber) and `warning`: name the one accent, say what the other is for (or drop it from the tokens), and rule on WCAG 1.4.11 for component boundaries. Without this the checklist's first review fails on our own palette.
- [ ] Commit `M4.2: design principles and the anti-AI-look checklist`.

### Task M4.3: materials, elevation, springs, easing and modes as tokens
**Files:** modify `design/tokens.toml`, `tools/gen-tokens/src/lib.rs` (and `main.rs` if outputs change); regenerate `crates/pb-tray/src/tokens.rs` and `android/.../ui/Tokens.kt`.

**Interfaces (used by M5 and M6):**
- TOML: `[material.<look>.<mode>]` with look ∈ {`native`, `relay`}, mode ∈ {`light`, `dark`}, keys `tint` ("#RRGGBB"), `opacity`, `blur`, `highlight`, `border_alpha`, `grain` (floats; blur in dp). `[elevation]` `blur = [0,2,8,16]`, `alpha = [0.0,0.08,0.12,0.16]`. `[spring.<name>]` for `standard`, `emphasised`, `quick` with `stiffness`, `damping_ratio`. `[easing]` `standard`, `decelerate`, `accelerate` as 4 floats each.
- Rust: `pub struct Material { pub tint: u32, pub opacity: f32, pub blur: f32, pub highlight: f32, pub border_alpha: f32, pub grain: f32 }`; `pub struct Modes { pub reduced_transparency: bool, pub reduced_motion: bool, pub lite: bool, pub high_contrast: bool }`; `pub fn resolve(m: Material, modes: Modes) -> Material` (with reduced transparency, lite or high contrast: `opacity = 1.0`, `blur = 0`, `grain = 0`, `highlight = 0`); `pub struct Spring { pub stiffness: f32, pub damping_ratio: f32 }`; `pub fn spring_or_cut(s: Spring, modes: Modes) -> Option<Spring>` (`None` under reduced motion: cross-fade at `fast_ms`).
- The generator emits only data and the type definitions. The two rules (`resolve`, `spring_or_cut`) are written by hand next to the generated code, once in Rust (`crates/pb-tray/src/tokens_rules.rs`) and once in Kotlin (`ui/TokenRules.kt`), each with its own test in the crate that calls it (`reduced_transparency_is_opaque`, `reduced_motion_cuts_springs` in pb-tray and in a Kotlin JUnit test), so M5 and M6 test the code they call and the two cannot drift unseen. The generated `tokens.rs` gets `#![allow(dead_code)]`-style `#[allow(dead_code)]` on its items (pb-tray is a bin crate and nothing uses them until M5; clippy runs with `-D warnings`).
- Kotlin: the same names as `data class Material`, `data class Modes`, `data class Spring`, `fun resolve(...)`, `fun springOrCut(...)`.
- Starting values are only a first guess. Hand arithmetic says several will fail the AA tests (relay light 0.62 over black for `text_muted` is about 2.3:1; relay dark 0.58 over white for `text` is about 4.4:1 and for `text_muted` and `primary` about 2.5:1). M4.1 decides, with citations, which knob moves (opacity, darker muted text on glass, or a luminosity/scrim layer as Acrylic has, which would change what `over()` models and M5 reuses). The values come out of the M4.4 tests, so GREEN there is a deliberate palette decision, not a surprise. First guess: native light `#F3F3F3` 0.80 / 30 / 0 / 0.06 / 0; native dark `#202020` 0.80 / 30 / 0 / 0.08 / 0; relay light `#F8F9FF` 0.62 / 40 / 0.55 / 0.18 / 0.03; relay dark `#14172A` 0.58 / 40 / 0.14 / 0.14 / 0.04 (opacity / blur / highlight / border_alpha / grain); springs standard 380 / 0.86, emphasised 220 / 0.78, quick 800 / 1.0; easing standard `[0.8,0,0.2,1]`, decelerate `[0,0,0,1]`, accelerate `[1,0,1,1]`.

- [ ] **RED:** in `gen-tokens`, `materials_load_for_both_looks_and_modes`, `a_missing_material_key_is_an_error_naming_it` and the existing drift tests now covering the new tables; in pb-tray, `reduced_transparency_is_opaque` (also lite and high contrast) and `reduced_motion_cuts_springs` against the hand-written rules; the same two in Kotlin. Run `cargo test -p gen-tokens -p pb-tray` and the Kotlin test; expected FAIL.
- [ ] **GREEN:** parse and render the new tables; run `cargo run -p gen-tokens`; run `cargo test -p gen-tokens`, the full gates and the Android tests (they compile `Tokens.kt`).
- [ ] Commit `M4.3: materials, elevation, springs, easing and modes as tokens`.

### Task M4.4: CSS output and WCAG tests in the generator
**Files:** modify `tools/gen-tokens/src/lib.rs` and `main.rs`; create generated `design/floot/base.css`.

**Interfaces:**
- `pub fn render_css(t: &Tokens) -> String`: `:root` custom properties (`--color-<name>`, `--space-N`, `--radius-N`, `--type-<name>`, `--motion-<name>`), `[data-theme="dark"]` overrides, `[data-look="relay"]` material variables, and `@media (prefers-reduced-motion: reduce)` and `(prefers-reduced-transparency: reduce)` blocks applying `resolve`'s rules.
- `pub fn contrast(a: u32, b: u32) -> f64` (WCAG relative luminance); `pub fn over(tint: u32, opacity: f32, backdrop: u32) -> u32` (composite).

- [ ] **RED:** `checked_in_css_matches_the_toml`, `contrast_of_black_on_white_is_21`, `glass_text_is_aa_over_white_and_black` (text and text_muted against the material over 0xFFFFFF and 0x000000, every look and mode, ≥ 4.5), `icons_are_3_to_1` (`text`, `text_muted` and `primary` ≥ 3.0 against the same composites; before writing it, grep what pb-tray and Compose draw icons in and use exactly those colours), `every_mode_meets_aa` (the same after `resolve` with each mode). Expected FAIL.
- [ ] **GREEN:** implement. Before the TOML values are final, run the tests against a ten-line script to see which fail. If a value fails AA, change the TOML value or the model (per the M4.1 ruling), never the threshold, and record the ruling.
- [ ] Commit `M4.4: tokens emit Floot CSS; WCAG checked over white and black`.

### Task M4.5: Floot project and principles
- [ ] `get_guides("floot-overview")` then `get_guides("design")`, `create_project` "Relay design", and share the preview link exactly as returned. Set the project's design principles from `PRINCIPLES.md`; add `design/floot/base.css` (mapped onto Floot's variables if its guide requires); add a shell with switches for look, light/dark, reduced motion and reduced transparency (they set `data-look` / `data-theme`).
- [ ] Read Floot's design guide before replacing its `base.css`: if Floot expects its own variables there, map the tokens onto them instead of overwriting the file. The preview link carries a login-free token, so it goes to the owner in chat and never into the repo; PROGRESS.md records only the project name or id.
- [ ] Commit `M4.5: Floot design project`.

### Task M4.6: Floot PC screens
- [ ] Floot budget: build one shared component kit page (buttons, rows, cards, dialogs, toasts) plus the look and mode shell first; the screens below reuse it. About 25 bespoke pages would use up the session before M5.
- [ ] Pages: tray flyout; main window (Home, Devices, Transfers, Timeline, Settings); pairing and QR; consent dialog; screen-window chrome; toast; and the screens M7 to M10 add (unlock prompt, Disconnect everything, device picker, transfer queue with resume, update available, public-network warning). Token variables only, no literal colours; real copy from `strings.rs`; Fluent System Icons.
- [ ] Commit `M4.6: PC screens in Floot`.

### Task M4.7: Floot Android screens
- [ ] Pages: Home, device picker, Transfers, Timeline, Remote and touchpad, Settings with Appearance, consent sheets, pairing and SAS, widget, quick panel and tile, plus the M7 to M10 screens (unlock gate, Disconnect everything, update available). Copy from the Android string resources; Material Symbols.
- [ ] Commit `M4.7: Android screens in Floot`.

### Task M4.8: motion demos
- [ ] One page with the three springs and three easings on real components (flyout open, sheet, list insert, progress) and a reduced-motion switch that shows the cross-fade.
- [ ] Commit `M4.8: motion demos`.

### Task M4.9: screenshots and the design review
- [ ] Screenshot every page in each look and mode with Floot's `screenshot_preview` into `docs/v2/design/review/`. Ruling: this replaces Playwright (the same result without a new dependency); M5 adds Playwright if it falls short.
- [ ] A fresh reviewer (design:design-critique and design:accessibility-review skills) scores each page against `PRINCIPLES.md` and the contrast rules into `docs/v2/design/REVIEW.md`. Important findings are fixed in Floot or the tokens (token changes go back through M4.3 and M4.4's tests).
- [ ] Commit `M4.9: design review and screenshots`.

### Task M4.R: milestone check
- [ ] One reviewer over the M4 diff against the Review Focus; re-grade by effect; fix the serious findings test-first.
- [ ] DECISIONS (tokens are the single source; the material values with their citations); THREAT_MODEL "no change"; CI green by full SHA; tick M4.R in PROGRESS.md.
- [ ] Commit `M4.R: milestone review`.
