# Android UI references for Relay (Compose, Material 3)

Research date 2026-11-06. Method note: m3.material.io is JS-rendered and WebFetch returned empty pages, so token values below come from the AndroidX source mirror (raw.githubusercontent.com/androidx/androidx, androidx-main). That is the shipped implementation, but it can lag or differ from the m3.material.io spec text. Items I could not source are in Gaps, not stated as fact.

## 1. Material 3 / M3 Expressive tokens and Compose stability

### Takeaway
Stable Compose Material3 is 1.4.0; 1.5.0-rc01 landed 2026-11-04, so 1.5.0 stable is imminent. In the 1.5.0 pre-releases MotionScheme, the expressive theme and ButtonGroup were promoted to stable, while LoadingIndicator/MaterialShapes promotion was reverted. Relay can adopt the token values (shapes, springs, type) today without opting into the unstable parts.

### Cited Findings
Stability / versions
- Stable 1.4.0; RC 1.5.0-rc01 (2026-11-04); beta 1.5.0-beta02 (2026-10-21); alpha 1.5.0-alpha29 (2026-09-23) — [Compose Material3 releases](https://developer.android.com/jetpack/androidx/releases/compose-material3) (page is 345k chars; only first 100k read)
- MotionScheme graduated from experimental in 1.5.0-alpha15 (2026-02-25) — same page
- Expressive theme/colour scheme promotion ("materialExpressiveTheme / expressiveLightColorScheme") in 1.5.0-alpha18 (2026-04-22); the page text is garbled, so the exact API name is uncertain — same page
- ButtonGroup promoted to stable in 1.5.0-alpha22 (2026-06-17) — same page
- 1.5.0-alpha19 (2026-05-06): "Revert MaterialShapes and LoadingIndicator promotions to stable" — same page
- The page does not say which APIs are in 1.4.0 stable; I did not verify this (see Gaps) — same page

Shape scale (dp), from ShapeTokens.kt — [ShapeTokens.kt](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/ShapeTokens.kt)
- None 0; ExtraSmall 4; Small 8; Medium 12; Large 16; LargeIncreased 20; ExtraLarge 28; ExtraLargeIncreased 32; ExtraExtraLarge 48; Full = circle
- Directional variants exist: LargeTop/LargeStart/LargeEnd 16; ExtraLargeTop 28 (top corners only); ExtraSmallTop 4

Type scale (sp size / line height / tracking / weight), from TypeScaleTokens.kt — [TypeScaleTokens.kt](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/TypeScaleTokens.kt)
- Display L/M/S: 57/64/-0.2, 45/52/0, 36/44/0 (Regular)
- Headline L/M/S: 32/40, 28/36, 24/32 (Regular, tracking 0)
- Title L: 22/28/0 Regular; Title M: 16/24/0.2 Medium; Title S: 14/20/0.1 Medium
- Body L: 16/24/0.5; Body M: 14/20/0.2; Body S: 12/16/0.4 (Regular)
- Label L: 14/20/0.1; Label M: 12/16/0.5; Label S: 11/16/0.5 (Medium)
- Emphasized variants exist (same sizes, heavier weight: Medium for Display/Headline/Title L/Body, Bold for Label and Title M/S); source marks them "TODO update with the generated tokens", so they may change — same file

Motion: springs, from ExpressiveMotionTokens.kt — [ExpressiveMotionTokens.kt](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/ExpressiveMotionTokens.kt)
- Spatial: Default damping 0.8 / stiffness 380; Fast 0.6 / 800; Slow 0.8 / 200
- Effects (colour, opacity): Default 1.0 / 1600; Fast 1.0 / 3800; Slow 1.0 / 800 (critically damped, no overshoot)
- Matches the split reported by third-party ports (spatial = springs that may overshoot, effects = no overshoot) — [M3EMotion (pub.dev)](https://pub.dev/documentation/material_3_expressive/1.0.3/foundations_m3e_motion/M3EMotion-class.html)

Motion: durations and easing, from MotionTokens.kt — [MotionTokens.kt](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/MotionTokens.kt)
- Durations ms: Short1-4 = 50/100/150/200; Medium1-4 = 250/300/350/400; Long1-4 = 450/500/550/600; ExtraLong1-4 = 700/800/900/1000
- Easing cubic-bezier: Emphasized (0.2,0,0,1); EmphasizedAccelerate (0.3,0,0.8,0.15); EmphasizedDecelerate (0.05,0.7,0.1,1); Standard (0.2,0,0,1); StandardAccelerate (0.3,0,1,1); StandardDecelerate (0,0,0,1); Legacy (0.4,0,0.2,1); Linear (0,0,1,1)
- Source note: the fetch tool's summary said the file does not state units; ms is the established M3 convention, not stated in that file

### Inferences
- Relay should define its own shape/motion tokens mapping to these values (cards 16 or 20, dialogs/sheets 28, chips/buttons full) rather than depend on unstable APIs. A pragmatic path: pin stable 1.4.0 (or 1.5.0 once it goes stable, likely within weeks given rc01) and hand-roll springs with `spring(dampingRatio, stiffness)` from the table.
- Use spatial springs for size/position and effects springs for colour/alpha: avoids overshoot on opacity, which would look broken.
- Spacing: the 4/8 dp grid is standard M3 practice, but I found no source this session (see Gaps).

### Gaps
- Official m3.material.io pages (corner radius scale, elevation, spacing, motion) could not be fetched; the values above are AndroidX implementation values, not the spec page.
- Elevation levels (0/1/3/6/8/12 dp and tonal overlays) not verified; no source retrieved.
- The 4/8 dp spacing grid and M3 layout margins (16 compact / 24 medium) not verified.
- Whether MotionScheme and expressive theme are in 1.4.0 stable or only 1.5.0 is unconfirmed; the release notes page was only read up to 100k of 345k chars.
- Whether 1.5.0 final has shipped: as of the page read, only rc01 (2026-11-04).

## 2. Real shipping references: home, settings, consent

### Takeaway
Official Android docs give strong, citable rules for consent (in-context ask, explain, no nagging, never block UI) and MediaProjection (per-session consent, status-bar chip, auto-stop on lock). Reference apps were harder to document: I have only thin sourced evidence for Quick Share, Pixel Settings and Quick Settings tiles, and none for LocalSend's UI, Nothing OS or Files by Google, so those are listed as gaps rather than invented.

### Cited Findings
(a) Home / status / quick actions
- Quick Settings tile states are exactly three: active, inactive, unavailable; the public Tile API has label, subtitle, icon, state and content description — [AOSP qs-tiles.md](https://android.googlesource.com/platform/frameworks/base/+/HEAD/packages/SystemUI/docs/qs-tiles.md)
  - Copy for Relay: model every connection/share toggle with three explicit states (on / off / unavailable) plus a one-line subtitle explaining state ("Connected to Harsh-PC", "Phone not on same network").
- Boolean tiles can expose switch accessibility class so TalkBack announces current and new state — same AOSP doc
- Android 16 QPR1 Quick Settings tiles can appear in different sizes (search-result summary, not primary source) — [9to5Google, Android 16 M3 Expressive redesign](https://9to5google.com/2025/05/13/android-16-material-3-expressive-redesign/)
- LocalSend: open-source (Apache-2.0) local-network file sharing, "securely share files and messages with nearby devices over your local network"; supports a hidden/tray start flag on desktop. UI details (tabs, device-info card, quick-save) NOT confirmed from the README — [LocalSend repo](https://github.com/localsend/localsend)
- Quick Share: visibility choices per Google's current help are "Your devices", "Contacts", and "Everyone for 10 minutes" (auto-reverts) — [Google support: Quick Share](https://support.google.com/android/answer/15728591), [PhoneArena](https://www.phonearena.com/news/quick-share-may-be-closing-the-airdrop-gap-between-android-and-iphone-in-your-own-home_id180921); contacts visibility works only while screen is on and unlocked — [Files by Google help](https://support.google.com/files/answer/7577382). Conflicting reports that persistent "Everyone" was removed — [WebProNews](https://www.webpronews.com/google-quietly-kills-androids-everyone-sharing-mode-signaling-a-privacy-first-pivot-for-quick-share) (secondary; no dated Google announcement found)
  - Copy for Relay: time-boxed exposure ("visible for 10 minutes") rather than a permanent open state.

(b) Settings screens
- Android 16 QPR1 brought the Material 3 Expressive redesign, including Settings, to Pixel (beta May 2025, stable Sept 2025); Google calls it an evolution of M3, not a new system — [9to5Google: Android 16 settings redesign](https://9to5google.com/2025/05/20/android-16-settings-redesign/), [Android Authority: more settings](https://www.androidauthority.com/material-3-expressive-more-android-settings-3574011) (from search summaries; pages not opened)
- Reviewers' criticism: some found the redesign childish at first — [Android Headlines](https://www.androidheadlines.com/2025/05/i-wanted-to-roast-android-16s-redesign-now-i-love-it.html) (opinion)
- Compose guidance for settings rows: lift the toggle to the parent row via `toggleable(role = Role.Checkbox/Switch)` and set the inner control's callback to null, so the whole row is the touch target — [Compose accessibility API defaults](https://developer.android.com/develop/ui/compose/accessibility/api-defaults)

(c) Consent / permission sheets and dialogs
- Ask in context, when the user starts the feature; never block the user; always give a cancel on educational UI; degrade gracefully if denied — [Request runtime permissions](https://developer.android.com/training/permissions/requesting)
- Explain why before calling the system dialog (the system dialog says what, not why); the app cannot customise that dialog — same page
- On denial: be specific about what no longer works, highlight the affected spot with an error icon/colour, no full-screen blockers, no persistent nagging, do not deep-link to Settings to pressure — same page
- Two denials = permanent ("don't ask again"), so only prompt at the moment of need — same page
- Android 11+ location/mic/camera dialogs add "Only this time"; revoking a one-time permission kills the app process — same page
- Android 12+ privacy indicators show whenever mic or camera is accessed; Google suggests showing a visible in-app/notification indicator when capturing — same page
- MediaProjection: consent is required before each session (a session = one `createVirtualDisplay()`); token single-use; on Android 14+ a second use throws SecurityException — [Media projection](https://developer.android.com/media/grow/media-projection)
- Android 14+: user can pick one app window or the entire screen; Android 15 QPR1+ shows a prominent status-bar chip that stops sharing when tapped; projection auto-stops when the screen locks; register `MediaProjection.Callback.onStop()` and update UI — same page
  - Copy for Relay: UI must reflect stop events from the chip/lock instantly (do not keep showing "Sharing").
- Quick Share: even in everyone mode, the recipient must accept each incoming transfer — [Android Authority](https://www.androidauthority.com/nearby-share-3197368/)
- LocalSend accept dialog: not verified (see Gaps).

### Inferences
- For an "audio sharing" consent in Relay, the pattern is: (1) in-app explainer sheet saying what is shared and in which direction, with "Not now"; (2) system dialog if one exists; (3) a persistent, visible "sharing" indicator in app and notification, with a one-tap Stop; (4) re-consent per session (mirrors MediaProjection) with an explicit time-box.
- Per-device accept (Quick Share model) should be the default for any new PC pairing, even on the same network.

### Gaps
- Nothing OS Settings: no sources found this session.
- Files by Google home/settings UI: only a help-page URL on Quick Share visibility; no UI description.
- LocalSend UI (home tabs, accept dialog, quick-save): README does not describe it; would need the `app/` directory or localsend.org screenshots.
- Pixel Settings structure (grouped rounded cards, corner values): confirmed only that a redesign exists; no source describes its layout or radii.
- Quick Share accept sheet's visual layout (preview, Accept/Decline) not sourced.
- Official developer.android.com design page for Quick Settings tile states not found; only AOSP docs.

## 3. Accessibility (touch targets, font scale, labels, reduced motion, contrast)

### Takeaway
Hard rules: minimum 48dp interactive size (Material components do this automatically only when interactive; use `minimumInteractiveComponentSize` for small visuals), unique meaningful content descriptions, merged semantics on grouped rows, state beyond colour. Reduced motion is read via the animator duration scale and Compose honours it through MotionDurationScale, but community reports on consistency conflict.

### Cited Findings
- Minimum 48dp for clickable elements; Checkbox/RadioButton/Switch/Slider/Surface add it internally only when `onCheckedChange` etc. is non-null — [Compose API defaults: touch targets](https://developer.android.com/develop/ui/compose/accessibility/api-defaults)
- `minimumInteractiveComponentSize` reserves 48dp of layout space, centres the element, and must be placed before size modifiers; it does not itself expand the touch area — same page
- Compose expands touch targets outside bounds when a clickable is smaller than the minimum; overlapping areas are possible, so use `sizeIn(minWidth = 48.dp, minHeight = 48.dp)` — same page
- Icon/Image need `contentDescription` (localized); decorative ones use `null` — same page
- Merge related content with `semantics(mergeDescendants = true)`; make only the parent clickable; don't aggregate child text into the parent description — [Principles for improving app accessibility](https://developer.android.com/guide/topics/ui/accessibility/principles)
- Mark headings with `semantics { heading() }`; set unique `paneTitle` on window-like panes such as bottom sheets (API 28+; reusing generic titles like "Details" causes unpredictable TalkBack behaviour) — same page
- Provide `onClickLabel` / `onLongClickLabel` so TalkBack says "Double tap to open this article" instead of "Double tap to activate"; use `customActions` for swipe/drag alternatives — same page and API defaults page
- Use cues other than colour: shapes, text, icons, haptics — same principles page
- Font scaling is non-linear in Android; the official doc recommends testing against system font/display settings and says layouts must not break, overlap or hide content — [Support user-scalable content](https://developer.android.com/develop/ui/compose/accessibility/scalable-content). The doc's sample uses typical scale limits ~0.75x to 3.5x for in-app scaling (that is app-defined, not the system maximum)
- Reduced motion: Android has no direct "reduce motion" flag exposed to apps; the common workaround reads the animator duration scale (0 when "Remove animations" is on). Compose exposes `MotionDurationScale`; scale 0 completes motion in the next frame — [DEV: Android animations and reduced motion](https://dev.to/eevajonnapanula/android-animations-and-reduced-motion-2em2), [MotionDurationScale](https://composables.com/docs/androidx.compose.ui/ui/interfaces/MotionDurationScale). Conflicting: Kotlin Slack reports some Compose animations unaffected, and infinite animations stop at 0 leaving e.g. a dot — [Kotlinlang Slack](https://slack-chats.kotlinlang.org/t/16196587/i-m-seeing-conflicting-information-regarding-whether-develop?t=v2) (anecdotal)

### Inferences
- Relay settings rows: whole-row toggleable, switch with null callback, row min height 56dp (two-line 72dp is typical M3 list but unsourced) so 48dp is always exceeded.
- Test at the largest system font size (Settings > Display > Font size, and Display size) and with long strings; avoid fixed heights on text containers; prefer `heightIn(min = ...)`.
- Respect reduced motion explicitly: read the scale, and when 0 replace springs with instant/fade changes; never rely on animation to convey state.
- Status colours (connected/error) need an icon and/or text, per the cues-other-than-colour rule.

### Gaps
- The "200% font scale" figure: the Android system's maximum font scale is not stated on any page I fetched. I did not find a primary source for 200% (Android 14 offers nonlinear scaling up to 200% in Settings; recalled, unverified).
- Contrast ratios (WCAG 4.5:1 body text, 3:1 large text/UI components) and Android's "High contrast text" setting: not retrieved from a primary source this session.
- TalkBack-specific label wording conventions beyond the pages above: not covered.

## 4. One accent colour practice: primary vs tertiary vs error vs a second status colour

### Takeaway
M3 offers primary (key actions, active states), secondary (lower-emphasis), tertiary (a contrasting accent to balance or draw attention), and error (errors only). M3 has no built-in warning or success role, so a second status colour is something the app must define itself.

### Cited Findings
- Primary anchors key components such as prominent buttons, active states, and icons on tonal buttons; tertiary is a contrasting accent that balances primary and secondary or draws attention to an element such as an input field — [Wear Material 3 colour roles](https://developer.android.com/design/ui/wear/guides/styles/color/roles-tokens) (Wear flavour of M3; not the phone page)
- Container roles are fills for foreground elements and should not be used for text or icons; use the matching `on*` colour — same page
- Error with onError is for error messages/outlines — [Taler wallet colour doc adopting M3 semantics](https://docs.taler.net/design-documents/066-wallet-color-scheme.html) (third party)
- M3 does not prescribe warning/success roles; some guides map them onto error/tertiary — [search summary of third-party guides](https://react-native-dynamic-theme.vercel.app/docs/color-system) (opinion)
- Compose `ColorScheme` lists all roles including `*Fixed` and `*Dim` variants — [ColorScheme API](https://developer.android.com/reference/kotlin/androidx/compose/material3/ColorScheme)
- Android's guidance for denied permissions: show a different button "that contains an error icon and color" at the affected spot — [Request runtime permissions](https://developer.android.com/training/permissions/requesting)

### Inferences
- Single-accent plan for Relay: one primary hue drives actions, selection and "connected" accents; neutral surfaces carry everything else; `error` is reserved for failure/blocked states.
- A second status colour is typically for non-error but attention-worthy states (warning/"degraded", "sharing live"). Define it as an extended colour with its own container/on-container pair (harmonised to the primary via `Color` blending) rather than reusing tertiary or error, and always pair it with an icon and text.
- "Sharing audio/mic live" is a privacy-relevant state: a distinct, consistently used indicator colour, plus shape (dot/waveform icon), parallels OS mic/camera indicators.

### Gaps
- m3.material.io's own colour-role usage pages were not retrievable (JS-rendered); the Wear page is a proxy.
- No primary source on extended-colour/warning guidance for M3 on phones.

## 5. Blur on Android (RenderEffect, Haze, fallbacks)

### Takeaway
Compose `Modifier.blur` renders only on API 31+ (uniform radius); varying blur needs API 33. Haze 2.1.0 (2026-11-02) is the de-facto library, Apache-2.0, with Glass now stable, but it still carries performance warnings and an RenderScript path below API 31. For Relay, treat blur as optional decoration with an opaque or scrim fallback.

### Cited Findings
- `Modifier.blur` uniform radius needs Android 12 (API 31); spatially varying radii need API 33; below that the modifier is ignored — [Compose blur modifier docs (composables.com mirror)](https://composables.com/jetpack-compose/androidx.compose.ui/ui/modifiers/blur), summarised in search results
- Cloudy library: RenderEffect on 31+, CPU path on 30 and below disabled by default with a semi-transparent scrim fallback; RenderScript deprecated since Android 12 — [Cloudy](https://github.com/amanna13/Cloudy), [klibs.io listing](https://klibs.io/project/skydoves/Cloudy)
- Haze latest release 2.1.0 on 2026-11-02; Apache-2.0; platforms Android, Desktop JVM, iOS, macOS, Wasm/JS — [Haze repo](https://github.com/chrisbanes/haze), [Haze CHANGELOG](https://raw.githubusercontent.com/chrisbanes/haze/main/CHANGELOG.md)
- In 2.1.0 Glass is stable in `haze-glass` and `haze-glass-material3` (experimental in 2.0.0); the native Backdrop backend on Android and `HazeInputSharing` remain experimental — Haze CHANGELOG
- Below API 31 Haze blur uses RenderScript (fixes for stale captures in 2.0.1 and a teardown race in 2.0.0-beta03); Android 12/12L have special handling for progressive blur — Haze CHANGELOG
- Fallbacks: with reduced transparency Glass uses stronger blur and a more opaque look (custom opaque colour supported); `HazeInput.Backdrop` has a portable captured-source fallback and can be set to leave content unchanged where native rendering is unavailable — Haze CHANGELOG
- Performance: "Glass containers can add substantial cost in dense scrolling layouts"; Pixel 8a sample numbers: Glass peak GPU memory ~7% lower than 2.0.1, Blur `Quality` at least 9% lower; `HazePerformanceMode.Default` = fixed `Balanced`; Haze README advises measuring a release-like build on representative devices before choosing Quality/Performance/Fixed — Haze CHANGELOG and [Haze README](https://github.com/chrisbanes/haze)
- Camera previews: CameraX needs `PreviewView.ImplementationMode.COMPATIBLE` because SurfaceView cannot be captured — Haze README
- M3 Expressive on Android 16 includes background blur effects (summary-level) — [9to5Google](https://9to5google.com/2025/05/13/android-16-material-3-expressive-redesign/)

### Inferences
- Relay should not make blur load-bearing: gate on `Build.VERSION.SDK_INT >= 31`, honour the user's transparency/reduce-effects preference, and make the fallback an opaque `surfaceContainer` tone. This also guarantees text contrast, since blurred backgrounds are hard to guarantee.
- Use blur only on small, static or overlay surfaces (a sheet scrim, a header), not on lists that scroll. The realme/OEM devices the project tests on are the likely weak spot; the repo's own notes flag realme as fragile.
- Haze 2.1.0 is a fresh release (days old); pin an exact version and test before upgrading.

### Gaps
- Haze minimum API level is not stated in the README or changelog I read.
- No measurements for low-end devices; only the Pixel 8a sample numbers.
- The official developer.android.com RenderEffect page returned 404, so RenderEffect behaviour is sourced from the Compose mirror and third parties.

## 6. Per-decision citation table (one official guideline + one real app, as requested)

### Takeaway
Only some decisions have both. Where the real-app half is missing, I say so.

### Cited Findings
- Three-state toggles with subtitle — guideline: [AOSP qs-tiles](https://android.googlesource.com/platform/frameworks/base/+/HEAD/packages/SystemUI/docs/qs-tiles.md); real app: Android Quick Settings tiles (same doc).
- Time-boxed visibility / per-transfer accept — guideline: [Request runtime permissions](https://developer.android.com/training/permissions/requesting) (ask in context, don't block); real app: Quick Share "Everyone for 10 minutes" ([Google support](https://support.google.com/android/answer/15728591)).
- Per-session consent plus visible stop affordance — guideline and real app: [Media projection](https://developer.android.com/media/grow/media-projection) (Android's own screen-share chip and auto-stop on lock).
- Whole-row toggle in settings with 48dp min — guideline: [API defaults](https://developer.android.com/develop/ui/compose/accessibility/api-defaults); real app: Pixel Android 16 Settings (existence of redesign only, row mechanics not verified).
- Spatial vs effects springs — guideline: AndroidX tokens ([ExpressiveMotionTokens.kt](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/ExpressiveMotionTokens.kt)); real app: Android 16 Pixel UI ([9to5Google](https://9to5google.com/2025/05/13/android-16-material-3-expressive-redesign/), physics-based animation mentioned).
- Optional, fallback-safe blur — guideline: Compose blur docs (API 31/33); real app: Android 16 Pixel UI uses background blur per 9to5Google (summary-level).

### Gaps
- LocalSend, Files by Google, Nothing OS: no verified specific UI pattern to pair with a guideline; do not cite them for UI details until someone opens their screenshots or source.

## Overall caveats
- Licensing: LocalSend is Apache-2.0 per its repo page, compatible with study-only use; no code was copied or needed.
- Several claims rest on search-result summaries rather than full pages (Android 16 Settings, Quick Share visibility, Modifier.blur docs). Treat those as leads.
