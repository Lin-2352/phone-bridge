# M6 Android native redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:executing-plans (inline, TDD, one reviewer at the end of the milestone). Steps use checkbox (`- [ ]`) syntax. Run `advisor` where a step says **ADVISOR**. Run several Monitors at once (gradle, CI by full SHA); never wait on one job alone.

**Goal:** rebuild every Android screen on the M4 design tokens, with a Native-glass and a Relay-glass look, Haze backdrop blur on API 31+, an automatic lite mode for old phones, token motion, per-maker background guidance, Roborazzi screenshots, and an emulator smoke job.

**Architecture:** small pure Kotlin functions decide look, colour scheme, blur mode, lite mode and motion from settings and system signals (plain JUnit 4 tests); Compose code only calls them. Each screen is split into a stateful wrapper and a stateless `XContent(state, actions)` so Roborazzi can render it with fixed state. M4's `Tokens.kt` and `TokenRules.kt` supply the values.

**Tech Stack:** Kotlin, Jetpack Compose (BOM 2025.06.01 at the time of writing), Material 3, Glance 1.1.1, Haze (Apache-2.0), Roborazzi + Robolectric (screenshots only), Gradle with `verification-metadata.xml`.

**Spec:** `docs/v2/SPEC.md` lines 271–289 (plus 37, 40, 47, 54); rows M6.1–M6.R in `docs/v2/PROGRESS.md`; `docs/v2/design/PRINCIPLES.md`; Floot project "Relay design" page `/android`.

## Global Constraints
- MIT OR Apache-2.0; no GPL/AGPL code; icons from Material Symbols (Apache-2.0).
- "The PC can only ask"; every live share keeps its visible Stop; the M1 consent guards (obscured-touch refusal, hidden overlays, secure window, Relay's own remote input paused over every prompt) stay exactly as they are.
- `minSdk 29`, `targetSdk 36`, ABIs arm64-v8a, armeabi-v7a, x86_64. `compileSdk` rises only if a stable library requires it.
- No wire change. Android budgets: background PSS ≤ 60 MB, at most one wakeup per 15 min when idle with no PC; effects run only while visible.
- WCAG AA over pure white and black: text 4.5:1, icons and control boundaries 3:1. Honour reduced transparency, lite mode, animations off, font scale up to 200%. Every control has a TalkBack label.
- Never rename the package `dev.phonebridge.app`, the Keystore alias `phone_bridge_identity_v1`, the notification channel ids or the `filesDir` file names.
- New or moved strings go in `res/values/strings.xml` (snake_case names), English.
- One commit per task, subject starting with the task ID; the same commit ticks `docs/v2/PROGRESS.md` and moves "Resume here". Push after every task. No attribution lines of any kind.
- Gates before every commit: pb-ffi build and uniffi bindings regenerated if pb-ffi changed; `cd android && ./gradlew :app:testDebugUnitTest :app:lintRelease --no-daemon --console=plain` ends in `BUILD SUCCESSFUL`; `cargo fmt/clippy/test` only if Rust changed. Only one gradle or cargo command at a time. CI is checked by full SHA under a Monitor.

## Review Focus
1. **A 10-year-old phone (API 29, 2 GB RAM):** lite mode turns on by itself, no blur, nothing animating while idle. Tests: `small_memory_turns_lite_on`, `old_android_turns_lite_on`, `blur_needs_api_31_and_no_lite` (M6.2/M6.3).
2. **Blur too slow on a mid phone:** it drops to lite within 120 frames and stays dropped. Test: `a_quarter_slow_frames_trip` (M6.2).
3. **Font scale 200%:** nothing is clipped on Home, Settings or the consent sheet. Tests: the Roborazzi `*_font2x` captures, inspected (M6.15).
4. **TalkBack:** every new control (Appearance switches, picker rows, Transfers rows, consent buttons) has a content description. Test: `every_new_control_has_a_label` (M6.15).
5. **Background PSS ≤ 60 MB with Haze present:** the Haze code is unused in the background; the emulator job's `pss.txt` records it and M6.R compares it with the budget (M6.16).

6. **Consent buttons in an unguarded window:** a bottom sheet drawn by `ModalBottomSheet` lives in its own window, so `SecureWindow.apply`, `setHideOverlayWindows` and the obscured-touch filter would not cover Allow/Deny (the M1.5 regression). Test: `consent_buttons_are_in_the_guarded_window` (M6.11).

## Rulings already made
- Ruling: strings move to resources as each screen is rewritten (M6.1–M6.14 move their own screen's strings) — SPEC global constraint; moving all at once would touch every file twice — cost if wrong: a task forgetting its strings; M6.R greps rewritten files for hard-coded `Text("…")`.
- Ruling: Roborazzi setup moves from M6.15 to M6.1 (M6.15 completes the matrix and the inspection) — each screen task needs it — none.
- Ruling: Roborazzi goldens are recorded on CI's OS (download the `android` job artifact, read several PNGs with the Read tool, then commit them) — Robolectric native rendering and fonts differ between Windows and Linux, so local goldens would fail `verifyRoborazziDebug` on every CI run — cost if wrong: one re-record.
- Ruling: every screenshot test class carries `@RunWith(AndroidJUnit4::class)`, `@GraphicsMode(GraphicsMode.Mode.NATIVE)` and `@Config(qualifiers = "w412dp-h915dp")` (confirm the annotation names against the pinned Roborazzi version in M6.1) — native graphics mode is what Roborazzi needs for Compose — none.
- Ruling: the consent prompt is drawn as a bottom-aligned surface inside the consent activity's own translucent window, not as `ModalBottomSheet` — the guards apply per window and these activities exist only for the prompt — cost if wrong: the sheet looks less like a stock sheet.
- Ruling: Robolectric is added for Roborazzi only; logic tests stay plain JUnit (the M3 plan said no Robolectric) — none.
- Ruling: dynamic colour schemes are not contrast-tested (they come from the OS at runtime); the glass material over them is tested with the relay schemes — cost if wrong: a vendor palette with poor contrast; the device checklist covers one dynamic wallpaper per theme.
- Ruling: new prefs keys (`theme_look`, `reduce_transparency`, `lite_forced`, `lite_auto_frames`) need no `PREFS_VERSION` bump, because they come with defaults — none.
- Ruling: the lite switch can force lite on but cannot force it off while an automatic reason applies — a phone that needs lite should not be talked out of it — cost if wrong: a user wants blur on a small phone; the switch text names the reason.
- Ruling: Haze is pinned by excluding its group from the trust regex in `verification-metadata.xml` and adding sha256 entries — the regex trusts everything else — none.
- Ruling: the Roborazzi mode matrix is cut to Relay dark for reduced-transparency, lite and font-scale — every mode resolves the same rules on every look — cost if wrong: a mode bug only on Native; the device checklist covers one.

## Today in the code (checked 2026-11-06; paths under `android/app/src/main/java/dev/phonebridge/app/`)
- Build: all versions inline in `android/app/build.gradle.kts` (no version catalog): Compose BOM `2025.06.01`, AGP 9.4.1, Kotlin compose plugin 2.2.10, Glance 1.1.1, navigation-compose 2.9.0; tests only JUnit 4.13.2 and org.json (no Robolectric, Roborazzi, compose ui-test or androidTest).
- `android/gradle/verification-metadata.xml` trusts every group except `com.github.MuntashirAkon` via a regex and pins only libadb and spake2; a new library is unchecked unless the regex excludes its group.
- `ui/Theme.kt`: `relayLightScheme()` (:47), `relayDarkScheme()` (:66), `ThemeChoice.mode` (:109; 0 system, 1 light, 2 dark), `isDark()` (:114), `RelayTheme(choice, content)` (:121), `Amber = Tokens.Light.accent` (:18); no dynamic colour anywhere.
- `ui/Tokens.kt` (generated): `GlassMaterial`, `TokenSpring`, `Tokens.Materials` (order native light, native dark, relay light, relay dark), `Tokens.Springs`. `ui/TokenRules.kt`: `Modes`, `resolve`, `springOrCut`, with no callers yet. `Ui.kt` `Spacing` (:18) and `Radius` (:27) duplicate the token scales.
- `ui/RelayNavHost.kt`: `enum class Tab { Home, Timeline, Remote, Settings }`, `RelayNavHost(start, startKey)` (:78) with 150 ms fades, `TAB_EXTRA` (:57). `MainActivity : ComponentActivity`.
- Screens: Home (`ui/home/HomeTab.kt`), Settings (`ui/settings/SettingsTab.kt`, 876 lines, rows inline in `when (page)` :615–805; an Appearance section with System/Light/Dark exists at :690), Timeline, Remote (`RemoteTab.kt`, `TouchpadPad.kt`), pairing (`ui/pairing/*`, `ScanActivity.kt`). No Transfers screen and no device picker exist; transfers appear only as notifications (`TransferText.kt`).
- Consent: `Ui.kt:78` `ConsentDialog(title, body, onAllow, onDeny, onAlways?)` (an M3 `AlertDialog`) used by `AudioConsentActivity`, `ScreenConsentActivity`, `CameraConsentActivity`, `ControlRequestActivity`; guards `ConsentGuard`, `SecureWindow.apply`, `setHideOverlayWindows`. The SAS check is a `RelayDialog` in `MainActivity.App()`.
- Widget `widget/RelayWidget.kt` (Glance, `ColorProviders(relayLightScheme(), relayDarkScheme())`), tiles `Quick.kt:87` and `SendClip.kt:22`, quick panel `Quick.kt:168` (blur-behind 80 on API 31+).
- Lite signals: only `ANIMATOR_DURATION_SCALE` via `ui/components/Blocks.kt:53` `animationsEnabled(ctx)`. `isLowRamDevice`, `isPowerSaveMode`, `MemoryInfo.totalMem` are unused. No OEM guidance except the generic "Keep running in background" row (`SettingsTab.kt:769–786`).
- Strings: `res/values/strings.xml` has 5 strings and Kotlin uses 0 `R.string`; `ThemeTest.no_user_visible_string_says_phone_bridge` reads it.
- CI: job `android` runs `:app:compileReleaseKotlin :app:compileQaKotlin :app:lintRelease :app:testDebugUnitTest`; `android-apk` builds the qa APKs; no emulator or screenshot job. Every action is pinned by SHA and the `deny` job runs `check-pinned-actions.sh`.

## Shared interfaces (defined in M6.1)
```kotlin
// ui/Appearance.kt (pure)
enum class Look { Native, Relay }                 // prefs theme_look: 0 native (default), 1 relay
data class Appearance(val look: Look, val mode: Int /*0 system,1 light,2 dark*/, val reduceTransparency: Boolean, val liteForced: Boolean)
fun materialFor(look: Look, dark: Boolean): GlassMaterial        // Tokens.Materials[look.ordinal * 2 + (if (dark) 1 else 0)]
fun colorSchemeFor(look: Look, dark: Boolean, sdk: Int, dynamic: (Boolean) -> ColorScheme?): ColorScheme
    // Native && sdk >= 31 -> dynamic(dark) ?: relay scheme; otherwise relayLightScheme()/relayDarkScheme()
// ui/components/GlassSurface.kt
@Composable fun GlassSurface(modifier: Modifier = Modifier, shape: Shape = RoundedCornerShape(Tokens.Radius[3]), content: @Composable BoxScope.() -> Unit)
@Composable fun SolidPanel(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit)
val LocalModes = staticCompositionLocalOf { Modes() }
val LocalGlass = staticCompositionLocalOf { Tokens.Materials[0] }
```

---

### Task M6.0a: Refresh at start
- [ ] M4.9 R-11 (deferred to here): `design/tokens.toml` has no Android shapes; PRINCIPLES says cards 16, sheets and dialogs 28, buttons and chips full. Add them test-first in gen-tokens (a `[shape_android]` table with `card = 16`, `sheet = 28`, rendered into `Tokens.kt` as `Tokens.Shape` and into the Floot CSS), and make `GlassSurface` default to `Tokens.Shape.Card`, not `Tokens.Radius[3]` (12). Also decide the Android type ramp (Material 3 scale) in the same pass.
- [ ] Re-check every item under "Today in the code" with Grep (line numbers drift). Known M4.9 token changes: amber `accent` is now `#955700` in light; `success_text`/`warning_text`/`danger_text` exist (text on a status tint, ≥ 4.5:1); Compose status chips use them.
- [ ] `GlassSurface` content rule (M4.R review I-1): status words, badges and amber text sit on `SolidPanel` inside the glass, never directly on it; add `status_text_is_on_a_solid_panel` as a source-scan or semantics test in M6.1.
- [ ] Custom Compose controls (the Appearance switches, picker rows) must keep a 3:1 off-state boundary (`outline_strong`) and a 3:1 focus indicator; add `off_switch_boundary_is_3_to_1` to M6.1's `ContrastTest` (M4.9 R-2, R-3). Read the M4.R DECISIONS "Design tokens" entry and `design/tokens.toml`; record any change as a `Ruling:` in this plan before M6.1. Confirm what M5 changed in `tokens.toml` (the `accent` → `attention` rename also changes `Tokens.kt` and `Theme.kt:18-19`).

### Task M6.1: Theme system, Settings → Appearance, Roborazzi setup
**Files:** create `ui/Appearance.kt`, `ui/components/GlassSurface.kt`; modify `ui/Theme.kt` (`RelayTheme(appearance, content)` provides `LocalModes`, `LocalGlass = resolve(materialFor(...), modes)`, uses `colorSchemeFor` with `dynamicLightColorScheme`/`dynamicDarkColorScheme`), `Prefs.kt` (`theme_look` Int 0, `reduce_transparency` Bool, `lite_forced` Bool), `ui/settings/SettingsTab.kt` (Appearance section: Theme Native glass / Relay glass; Mode System / Light / Dark; "Reduce transparency" switch; "Lite mode" switch whose supporting text names the automatic reason), `SettingsGroups.kt` (`settingEntries`), `res/values/strings.xml`, `android/app/build.gradle.kts`.
- Roborazzi dependencies: `org.robolectric:robolectric`, `io.github.takahirom.roborazzi:roborazzi`, `roborazzi-compose`, `roborazzi-junit-rule`, `androidx.compose.ui:ui-test-junit4`, `debugImplementation androidx.compose.ui:ui-test-manifest` (latest stable on Maven Central at execution time, MIT/Apache-2.0), plugin `io.github.takahirom.roborazzi`, `testOptions.unitTests.isIncludeAndroidResources = true`; goldens in `android/app/src/test/screenshots/`.
- [ ] **Step 1: failing tests.** `AppearanceTest`: `material_index_matches_look_and_mode` (all four indices equal `Tokens.Materials[i]`), `native_below_31_uses_the_relay_scheme`, `native_on_31_uses_dynamic_when_available`, `relay_ignores_dynamic_colour`, `defaults_are_native_system_full_effects`. `SettingsGroupsTest`: `every_appearance_row_can_be_found_by_search`. `ContrastTest`: `glass_text_is_aa_on_every_material_over_white_and_black` (composite each resolved material over white and black with `Color.compositeOver`; text ≥ 4.5, muted ≥ 4.5, icons ≥ 3.0).
- [ ] **Step 2:** `./gradlew :app:testDebugUnitTest --tests '*AppearanceTest*' --tests '*SettingsGroupsTest*' --tests '*ContrastTest*'` — Expected: FAIL (unresolved `Appearance`, `materialFor`).
- [ ] **Step 3: implement** the types, theme wiring, prefs, Appearance rows and Roborazzi setup.
- [ ] **Step 4:** same command — Expected: PASS.
- [ ] **Step 5:** full Android gates, PROGRESS, commit `M6.1: theme system, Appearance settings and screenshot tests`, push, CI.

### Task M6.2: Backdrop blur with Haze
**Files:** modify `android/app/build.gradle.kts`, `android/gradle/verification-metadata.xml`, `docs/DECISIONS.md`; create `ui/Blur.kt`; modify `ui/components/GlassSurface.kt`, `Quick.kt` (blur-behind follows `blurMode`).
**Interfaces:** `enum class BlurMode { Haze, Scrim, Opaque }`; `fun blurMode(sdk: Int, modes: Modes): BlurMode` (reducedTransparency or highContrast → `Opaque`; lite or sdk < 31 → `Scrim`; otherwise `Haze`); `class FrameBudget(intervalNs: Long, window: Int = 120, overRatio: Float = 0.25f) { fun onFrame(frameNs: Long): Boolean }` (true once more than 25% of the last 120 frames took more than 1.5 × `intervalNs`).
- [ ] **Step 1 (ADVISOR):** before adding Haze and changing the verification trust regex, call `advisor`.
- [ ] **Step 2:** pick the newest Haze release without `-alpha`, `-beta` or `-rc` from `https://repo1.maven.org/maven2/dev/chrisbanes/haze/haze/maven-metadata.xml`; if it needs a newer Compose than BOM 2025.06.01, bump to the oldest stable BOM that satisfies it and record a ruling. Change the trust regex to `^(?!com\.github\.MuntashirAkon|dev\.chrisbanes\.haze).*$`, add sha256 entries for each Haze artifact (generate on a copy with `./gradlew --write-verification-metadata sha256 :app:dependencies --dry-run`, then copy only the Haze components in).
- [ ] **Step 3: failing tests:** `blur_needs_api_31_and_no_lite`, `reduced_transparency_is_opaque_on_every_api`, `steady_frames_do_not_trip`, `a_quarter_slow_frames_trip`, `one_spike_does_not_trip`.
- [ ] **Step 4:** `./gradlew :app:testDebugUnitTest --tests '*BlurTest*'` — Expected: FAIL; implement; same command — Expected: PASS.
- [ ] **Step 5:** `GlassSurface` uses `Modifier.hazeEffect`/`hazeSource` (names of the pinned version) in `Haze` mode, draws the tint at the resolved opacity in `Scrim`, the solid surface in `Opaque`. A `LaunchedEffect` with a `withFrameNanos` loop runs only while a Haze surface is visible; on trip it sets `lite_auto_frames`.
- [ ] **Step 6:** DECISIONS entry "Haze for backdrop blur" (version, licence, why); full gates; commit `M6.2: backdrop blur with a scrim fallback`, push, CI.

### Task M6.3: Lite mode
**Files:** create `ui/Lite.kt`, `LiteMonitor.kt`; modify `ui/components/Blocks.kt`, `LinkIndicator.kt` (static when lite), `Prefs.kt` (`lite_auto_frames`).
**Interfaces:** `data class LiteSignals(val forced: Boolean, val lowRam: Boolean, val totalMemBytes: Long, val sdk: Int, val powerSave: Boolean, val animatorScale: Float, val slowFrames: Boolean)`; `enum class LiteReason { Forced, LowRam, SmallMemory, OldAndroid, BatterySaver, AnimationsOff, SlowFrames }`; `fun liteReason(s: LiteSignals): LiteReason?` (checked in enum order; SmallMemory = `totalMemBytes < 3L shl 30`; OldAndroid = `sdk < 31`; AnimationsOff = `animatorScale == 0f`); `fun liteText(r: LiteReason): String`.
- [ ] **Step 1: failing tests:** one per reason (`forced_turns_lite_on`, `low_ram_turns_lite_on`, `small_memory_turns_lite_on`, `old_android_turns_lite_on`, `battery_saver_turns_lite_on`, `animations_off_turns_lite_on`, `slow_frames_turn_lite_on`), plus `no_signal_means_full_effects`, `forced_is_reported_first`, `lite_text_names_the_reason`.
- [ ] **Step 2:** `./gradlew :app:testDebugUnitTest --tests '*LiteTest*'` — Expected: FAIL; implement; Expected: PASS.
- [ ] **Step 3:** `LiteMonitor` reads `ActivityManager.isLowRamDevice`, `MemoryInfo.totalMem`, `PowerManager.isPowerSaveMode` (receiver for `ACTION_POWER_SAVE_MODE_CHANGED` registered only while the UI is visible) and `animationsEnabled`; it feeds `Modes.lite`. Lite means: opaque surfaces (via `resolve`), `tween(Tokens.Motion.FAST_MS)` fades, nothing animating while idle.
- [ ] **Step 4:** gates; commit `M6.3: lite mode`, push, CI.

### Task M6.4: Motion
**Files:** create `ui/Motion.kt`; modify `ui/RelayNavHost.kt`, sheets and list items to use it.
**Interfaces:** `fun <T> tokenSpec(s: TokenSpring, modes: Modes): FiniteAnimationSpec<T>` (`springOrCut` non-null → `spring(dampingRatio = s.dampingRatio, stiffness = s.stiffness)` with **named arguments**, because `TokenSpring(stiffness, dampingRatio)` is the reverse order of Compose's `spring(dampingRatio, stiffness)`; else `tween(Tokens.Motion.FAST_MS)`).
- [ ] **Step 1:** run `./gradlew :app:dependencies --configuration releaseRuntimeClasspath | grep material3`; use `MotionScheme.expressive()` only if that version is stable and the API non-experimental, otherwise token springs; record the ruling.
- [ ] **Step 2: failing tests:** `normal_modes_give_the_token_spring`, `reduced_motion_gives_a_fast_tween`, `lite_gives_a_fast_tween`.
- [ ] **Step 3:** `./gradlew :app:testDebugUnitTest --tests '*MotionTest*'` — Expected: FAIL; implement; Expected: PASS. Replace the 150 ms fade literals in `RelayNavHost.kt`.
- [ ] **Step 4:** gates; commit `M6.4: token motion`, push, CI.

### Screen tasks M6.5–M6.13: the common pattern
Every screen task does these five things, in this order:
1. **Split** the screen into a stateful wrapper and a stateless `XContent(state: XUiState, actions: XActions)`.
2. **Restyle on tokens:** use `Tokens.Space`/`Tokens.Radius` in that screen (`Spacing`/`Radius` in `Ui.kt` are deleted in M6.13); glass only for bars, the nav bar, sheets and panels, with controls on `SolidPanel`; copy as in the Floot `/android` page.
3. **Move** the screen's strings to `strings.xml`.
4. **Add** `XScreenshotTest` with `@Config(qualifiers = "w412dp-h915dp")` capturing Native light, Native dark, Relay light and Relay dark.
5. **Keep** every existing logic test green.

Commands for every screen task (the screenshot test is the new failing test; tasks M6.5, M6.8, M6.9, M6.10 and M6.12 have no other new test, so do not invent one):
- [ ] **Failing step:** write `XScreenshotTest` first; `./gradlew :app:testDebugUnitTest --tests '*XScreenshotTest*'` — Expected: FAIL (`XContent` does not exist, so the class does not compile).
- [ ] **Implement** steps 1–3 above.
- [ ] **Record on CI:** push the branch, download the `android` job's screenshot artifact, read a few PNGs with the Read tool, commit them under `android/app/src/test/screenshots/`.
- [ ] **Verify:** `./gradlew :app:verifyRoborazziDebug :app:testDebugUnitTest` — Expected: PASS (`BUILD SUCCESSFUL`).
- [ ] **Commit** `M6.k: <screen> on tokens` with the full Android gates, PROGRESS, push, CI by full SHA.

- **M6.5 Home:** `HomeContent(state: HomeUiState)` over `HeroCard`, `NeedsCard`, `QuickGrid`, `SessionCard`; existing `AttentionTest`, `HomeCardsTest`, `ForgetPcTest`, `PcStatusTest` stay green.
- **M6.6 Device picker** (UI only; M8.1 wires it). Interfaces: `data class PickerRow(val id: String, val label: String, val status: String, val isDefault: Boolean, val live: Boolean)`; `fun pickerRows(peers: List<PbPeer>, live: Set<String>, lastSeen: Map<String, Long>, defaultId: String?, nowMs: Long): List<PickerRow>`; `@Composable fun DevicePickerSheet(rows: List<PickerRow>, onPick: (String) -> Unit, onDismiss: () -> Unit)`. Failing tests first: `default_comes_first`, `live_before_offline`, `ties_sort_by_name_ignoring_case`, `duplicate_names_get_a_fingerprint_suffix` (the suffix is " (" + first 4 characters of the fingerprint + ")").
- **M6.7 Transfers.** Interfaces: `data class TransferItem(val id: Long, val peerId: String, val peerName: String, val name: String, val size: Long, val done: Long, val outgoing: Boolean, val state: TransferState)`; `enum class TransferState { Waiting, Running, Done, Failed, Cancelled }`; `object Transfers { val items: SnapshotStateList<TransferItem> }` fed where `PbService` posts transfer notifications; `fun transferRows(items: List<TransferItem>): List<TransferRow>`; a new route `transfers` reached from a Home card and the transfer notification, and `Destination.Transfers`. Failing tests: `running_before_waiting_before_finished`, `progress_text_is_rounded_to_whole_percent`, `empty_list_text`.
- **M6.8 Timeline:** `TimelineContent`; keep `DayExpansionTest`, `EntryActionsTest`, `TimelineGroupingTest`.
- **M6.9 Remote and touchpad:** `RemoteContent`; keep `RemoteLinkTest`, `RemoteStateTest`, `TouchpadMathTest`.
- **M6.10 Settings:** `SettingsContent` for each page; Appearance already done in M6.1; keep `SettingsLogicTest`, `RestrictedAccessTest`, `SettingsGroupsTest`.
- **M6.11 Consent sheets.** `ConsentDialog` becomes `ConsentSheet(title, body, onAllow, onDeny, onAlways?)`, drawn as a bottom-aligned surface **inside the consent activity's own window** (see the ruling), not as `ModalBottomSheet`; it cannot be dismissed by dragging or tapping outside. Every guard (`ConsentGuard`, `SecureWindow.apply`, hidden overlays, obscured-touch filter, Relay's own remote input paused) stays on that same window. **Step 0:** read `ConsentGuard.kt` and `SecureWindow.apply` to see exactly which window each guard is applied to today (the current `AlertDialog` is also its own window). Failing tests first: `consent_sheet_cannot_be_dismissed_without_a_choice` (`fun sheetDismissAllowed(reason: DismissReason): Boolean` is false for outside tap and drag, true only after Allow or Deny) and `consent_buttons_are_in_the_guarded_window` (a Robolectric test that launches each of the four consent activities, finds the Allow and Deny nodes, and asserts they belong to the activity's decor window that has `FLAG_SECURE` / the hide-overlay setting applied, and that no extra dialog or popup window is attached). **ADVISOR** before this task's commit (it touches the consent path).
- **M6.12 Pairing and SAS:** `PairingContent`, the SAS check drawn inside the main window (a plain composable surface, not a separate window, so the same guards apply); keep `PairingTextTest`.
- **M6.13 Widget, quick panel and tile.** Glance uses `GlanceTheme` with dynamic colours on API 31+ in the Native look, relay schemes otherwise (no blur); the quick panel uses `GlassSurface` and follows `blurMode`; tile subtitles come from `strings.xml`; delete `Spacing` and `Radius` from `Ui.kt`. Failing test first: `no_file_uses_the_old_spacing_object` (a source scan of `src/main/java`).

### Task M6.14: Per-maker background guidance
**Files:** create `OemGuide.kt`; modify `ui/settings/SettingsTab.kt` ("Keep running in background" row opens a card with the steps and an "Open settings" button), `strings.xml`.
**Interfaces:** `enum class Oem { Xiaomi, Samsung, Huawei, OppoRealme, Other }`; `fun oemOf(manufacturer: String, brand: String): Oem` (lowercase "contains": xiaomi/redmi/poco, samsung, huawei/honor, oppo/realme/oneplus); `data class OemGuide(val steps: List<Int>, val intents: List<ComponentName>)`; `fun oemGuide(oem: Oem): OemGuide`.
Intents are tried in order with `resolveActivity`, falling back to `ACTION_APPLICATION_DETAILS_SETTINGS`; they only open settings screens, Relay changes nothing: Xiaomi `com.miui.securitycenter/com.miui.permcenter.autostart.AutoStartManagementActivity`; Samsung `com.samsung.android.lool/com.samsung.android.sm.ui.battery.BatteryActivity`; Huawei `com.huawei.systemmanager/.startupmgr.ui.StartupNormalAppListActivity`; Oppo/realme `com.coloros.safecenter/.permission.startup.StartupAppListActivity`.
- [ ] **Step 1: failing tests:** `oem_detection_by_maker_and_brand`, `oneplus_counts_as_oppo_realme`, `every_oem_has_steps_and_a_fallback`.
- [ ] **Step 2:** `./gradlew :app:testDebugUnitTest --tests '*OemGuideTest*'` — Expected: FAIL; implement; Expected: PASS.
- [ ] **Step 3:** gates; commit `M6.14: per-maker background guidance`, push, CI.

### Task M6.15: Roborazzi matrix and inspection
- [ ] Screens: Home, Transfers, Timeline, Remote, Settings, Settings→Appearance, Consent sheet, Pairing/SAS, Device picker, Quick panel. Full matrix Native and Relay × light and dark (40 images), plus Relay dark × {reduced transparency, lite, font scale 2.0} (30 images), 70 in total.
- [ ] Failing test first: `every_new_control_has_a_label` (a semantics-tree assertion over the new controls in the Roborazzi tests). Add `:app:verifyRoborazziDebug` to the CI `android` job.
- [ ] Record the goldens, then open at least one image of every screen and every font-scale-2.0 image with the Read tool; fix anything clipped before committing. Commit `M6.15: screenshots of every screen`, push, CI.

### Task M6.16: CI emulator smoke job
**Files:** modify `.github/workflows/ci.yml`, `scripts/check-pinned-actions.sh` expectations if needed; create `scripts/emulator-smoke.sh`.
- A new job `android-emulator`, `continue-on-error: true` (informational), needing `android-apk`; matrix `api-level: [29, 36]`, `target: google_apis`, `arch: x86_64`; `reactivecircus/android-emulator-runner` (Apache-2.0) pinned by commit SHA; KVM enabled with the udev rule step.
- `scripts/emulator-smoke.sh`: `adb install` the qa x86_64 APK; `am start -W -n dev.phonebridge.app/.MainActivity`; for each tab route start with `--es tab <route>` (`TAB_EXTRA`, `RelayNavHost.kt:57`), `screencap` into the artifact folder and check `pidof dev.phonebridge.app` is non-empty; `dumpsys meminfo dev.phonebridge.app | grep 'TOTAL PSS'` in the foreground; press HOME, wait 30 s, take PSS again; write `pss.txt`; upload screenshots, `pss.txt` and `logcat -d`. The job fails its step on a crash; `continue-on-error` keeps the run green.
- [ ] Failing check first: run `bash -n scripts/emulator-smoke.sh` and `bash scripts/check-pinned-actions.sh` — Expected: pin check FAILS until the action is pinned by SHA, then PASSES. Commit `M6.16: emulator smoke job`, push, watch the job's first run by full SHA.

### Task M6.R: Milestone review
- [ ] One reviewer (opus) over the M6 diff against the Review Focus; **ADVISOR** when re-grading findings; fix serious findings test-first.
- [ ] Grep rewritten files for hard-coded `Text("` strings and fix them; docs: DECISIONS (Haze, Roborazzi/Robolectric, lite rules, dynamic colour), THREAT_MODEL (the consent sheet keeps every M1.5 guard), WIRE_PROTOCOL (no change).
- [ ] CI green by full SHA with the emulator job informational; compare `pss.txt` with the 60 MB budget; **ADVISOR** before declaring M6 done. Commit `M6.R: milestone review`.

## Self-Review
- Spec coverage: themes and Appearance (M6.1), Haze with scrim and frame fallback (M6.2), lite triggers and toggle (M6.3), motion (M6.4), all screens (M6.5–M6.13), per-maker guidance (M6.14), Roborazzi matrix (M6.15), emulator smoke on API 29 and 36 with PSS (M6.16).
- Type consistency: `Look`, `Appearance`, `materialFor`, `blurMode`, `liteReason`, `tokenSpec`, `PickerRow` are defined once and reused unchanged (M8 consumes `PickerRow` and `DevicePickerSheet`).
- Proportion: bodies are left to the implementer; interfaces, test names and commands are fixed here.
