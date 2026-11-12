# UI motion values for a Direct2D (Windows 11) + Jetpack Compose (Android) design system

Research date: 2026-11-06. Values below were read from primary sources (Google/Microsoft source files and docs) unless flagged. Where a number is my own calculation it is labelled "Inference" and is not cited.

## 1. Material 3 / Material 3 Expressive motion tokens, Compose springs, easing and durations

### Takeaway
M3 Expressive has two spring families. Spatial springs move things (position, size) and may overshoot. Effects springs animate colour and opacity and are critically damped (damping 1.0). Expressive spatial springs are softer and bouncier than the Standard ones (0.8/0.6 damping versus 0.9). Easing and duration tokens are unchanged between the two schemes.

### Cited Findings

**Spring tokens, Expressive scheme (damping ratio / stiffness)**
- Default spatial 0.8 / 380; fast spatial 0.6 / 800; slow spatial 0.8 / 200. — [ExpressiveMotionTokens.kt (androidx)](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/ExpressiveMotionTokens.kt)
- Default effects 1.0 / 1600; fast effects 1.0 / 3800; slow effects 1.0 / 800. — [ExpressiveMotionTokens.kt](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/ExpressiveMotionTokens.kt)

**Spring tokens, Standard scheme (damping ratio / stiffness)**
- Default spatial 0.9 / 700; fast spatial 0.9 / 1400; slow spatial 0.9 / 300. — [StandardMotionTokens.kt (androidx)](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/StandardMotionTokens.kt)
- Effects springs are identical to Expressive: default 1.0 / 1600, fast 1.0 / 3800, slow 1.0 / 800. — [StandardMotionTokens.kt](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/StandardMotionTokens.kt)
- The Material Components Android docs list the same Standard numbers (spatial 0.9 with 1400/700/300; effects 1 with 3800/1600/800) and do not distinguish Expressive. — [Motion.md (material-components-android)](https://raw.githubusercontent.com/material-components/material-components-android/master/docs/theming/Motion.md)
- Usage: "Spatial springs are used for animations that move something on screen - like the x & y position of a View." "Effects springs are used to animate properties such as color or opacity where the property's value should not be overshot." — [Motion.md](https://raw.githubusercontent.com/material-components/material-components-android/master/docs/theming/Motion.md)
- Compose `MotionScheme` has `standard` and `expressive` implementations. Each exposes default/fast/slow spatial and effects specs, backed by the token constants above. — [MotionScheme.kt (androidx)](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/MotionScheme.kt). The numbers come from the two token files above; the numeric values are not in MotionScheme.kt itself.

**Compose `Spring` constants**
- DampingRatioHighBouncy 0.2; MediumBouncy 0.5 (also the default damping ratio); LowBouncy 0.75; NoBouncy 1.0 (critically damped, "returns to equilibrium within the shortest amount of time without oscillating"). — [Spring (Compose reference)](https://developer.android.com/reference/kotlin/androidx/compose/animation/core/Spring)
- StiffnessHigh: no value shown on the page (described as "extremely stiff"); StiffnessMedium 1500 (default stiffness); StiffnessMediumLow 400 (default for enter/exit transitions, added in 1.1.0); StiffnessLow 200; StiffnessVeryLow 50. — [Spring (Compose reference)](https://developer.android.com/reference/kotlin/androidx/compose/animation/core/Spring)
- DefaultDisplacementThreshold = 0.01, the "default cutoff for rounding off physics based animations". — [Spring (Compose reference)](https://developer.android.com/reference/kotlin/androidx/compose/animation/core/Spring)
- Interruption: with `animate*AsState`, a new target mid-flight starts "from its current value (and velocity)". `Animatable.animateTo` likewise cancels the running animation and continues from the current value and velocity. — [Value-based animations](https://developer.android.com/develop/ui/compose/animation/value-based)

**Easing curves (M3)**
- Standard `cubic-bezier(0.2, 0, 0, 1)`; Standard decelerate `(0, 0, 0, 1)`; Standard accelerate `(0.3, 0, 1, 1)`. — [Motion.md](https://raw.githubusercontent.com/material-components/material-components-android/master/docs/theming/Motion.md); [MotionTokens.kt](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/MotionTokens.kt)
- Emphasized is a two-segment path, not a single cubic: `M 0,0 C 0.05,0, 0.133333,0.06, 0.166666,0.4 C 0.208333,0.82, 0.25,1, 1,1`. — [Motion.md](https://raw.githubusercontent.com/material-components/material-components-android/master/docs/theming/Motion.md)
- Emphasized decelerate `(0.05, 0.7, 0.1, 1)`; emphasized accelerate `(0.3, 0, 0.8, 0.15)`; linear `(0, 0, 1, 1)`. — [Motion.md](https://raw.githubusercontent.com/material-components/material-components-android/master/docs/theming/Motion.md)
- Compose's `MotionTokens` defines `EasingEmphasizedCubicBezier` as `(0.2, 0, 0, 1)`, the same as Standard. It is a single-cubic stand-in for the two-segment path. Legacy easings are also present: `(0.4, 0, 0.2, 1)`, accelerate `(0.4, 0, 1, 1)`, decelerate `(0, 0, 0.2, 1)`. — [MotionTokens.kt](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/MotionTokens.kt)

**Duration tokens (ms)**
- Short1 50, Short2 100, Short3 150, Short4 200; Medium1 250, Medium2 300, Medium3 350, Medium4 400; Long1 450, Long2 500, Long3 550, Long4 600; ExtraLong1 700, ExtraLong2 800, ExtraLong3 900, ExtraLong4 1000. — [Motion.md](https://raw.githubusercontent.com/material-components/material-components-android/master/docs/theming/Motion.md); [MotionTokens.kt](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/tokens/MotionTokens.kt)
- Rule: "duration should increase as the area/traversal of an animation increases." — [Motion.md](https://raw.githubusercontent.com/material-components/material-components-android/master/docs/theming/Motion.md)

**Contradiction to flag**
- Motion.md states container transform defaults as incoming `motionDurationLong1`, "300ms", and outgoing `motionDurationMedium2`, "250ms". Those labels conflict with the token table on the same page (Long1 = 450, Medium2 = 300). The page also cites a "standard" easing `(0.4, 0, 0.2, 1)` that differs from its own token table. Treat the container-transform numbers as unreliable. — [Motion.md](https://raw.githubusercontent.com/material-components/material-components-android/master/docs/theming/Motion.md)

### Inferences
- A custom renderer can use the springs directly for spatial properties: expressive fast/default/slow = (0.6, 800) / (0.8, 380) / (0.8, 200). Use effects springs (damping 1.0, stiffness 3800/1600/800) for opacity and colour, so they never overshoot.
- Stiffness/damping tokens are mass-1 spring parameters in Compose (Inference; I did not confirm mass = 1 in the Compose source). If so, ω_n = sqrt(k) and the spring maps to a physical simulation with no extra constants.

### Gaps
- m3.material.io pages (motion/overview/specs, easing-and-duration/tokens-specs) are JS-rendered and returned only a title through the fetch tool. I could not read the official Material guidance on which duration pairs with which easing. The numbers above come from source code and the Android docs instead.
- StiffnessHigh numeric value was not shown on the reference page.

## 2. Fluent 2 / Windows 11 motion (durations, easing, springs)

### Takeaway
Fluent has two overlapping vocabularies. WinUI 3 control durations are 83/167/250 ms. The Fluent UI token set has eight durations from 50 to 500 ms plus nine bezier curves. Enter/decelerate is `cubic-bezier(0,0,0,1)` and exit/accelerate is `cubic-bezier(1,0,1,1)`. Windows does have spring animations (`SpringNaturalMotionAnimation`), but Microsoft publishes no spring parameter table equivalent to M3's.

### Cited Findings
- WinUI 3 standard durations: ControlNormalAnimationDuration 250 ms; ControlFastAnimationDuration 167 ms; ControlFasterAnimationDuration 83 ms. — [Timing and easing, Microsoft Learn](https://learn.microsoft.com/en-us/windows/apps/design/motion/timing-and-easing)
- "Fast Out, Slow In" `cubic-bezier(0, 0, 0, 1)`: for objects or UI entering the scene (navigating or spawning). — [Timing and easing](https://learn.microsoft.com/en-us/windows/apps/design/motion/timing-and-easing)
- "Slow Out, Fast In" `cubic-bezier(1, 0, 1, 1)`: for UI or objects exiting the scene. — [Timing and easing](https://learn.microsoft.com/en-us/windows/apps/design/motion/timing-and-easing)
- Fluent UI token durations: UltraFast 50, Faster 100, Fast 150, Normal 200, Gentle 250, Slow 300, Slower 400, UltraSlow 500 ms. — [fluentui durations.ts](https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/global/durations.ts)
- Fluent UI token curves: accelerateMax `(0.9,0.1,1,0.2)`; accelerateMid `(1,0,1,1)`; accelerateMin `(0.8,0,0.78,1)`; decelerateMax `(0.1,0.9,0.2,1)`; decelerateMid `(0,0,0,1)`; decelerateMin `(0.33,0,0.1,1)`; easyEaseMax `(0.8,0,0.2,1)`; easyEase `(0.33,0,0.67,1)`; linear `(0,0,1,1)`. — [fluentui curves.ts](https://raw.githubusercontent.com/microsoft/fluentui/master/packages/tokens/src/global/curves.ts)
- Fluent 2 principles are Functional, Natural, Consistent, Appealing. For top-level navigation "use a quick fade rather than sliding large elements". Linear easing is for constant-rate motion such as rotation. Staggering is preferred; non-staggered is for very large groups. Keep motion limited to the element in focus. — [Fluent 2 Motion](https://fluent2.microsoft.design/motion)
- Springs on Windows: `NaturalMotionAnimation` / spring animations let developers "define characteristics of the motion (for example, damping ratio for Springs)" instead of time. Properties: start/end values, `InitialVelocity`, and motion-specific properties such as `DampingRatio`. The doc's example sets `Period = 0.07 s`. The doc says natural motion is not a replacement for keyframes; "there are still places in the Fluent design language where KeyFrames are recommended". — [Natural motion animations](https://learn.microsoft.com/en-us/windows/apps/develop/composition/natural-animations); [Composition animations](https://learn.microsoft.com/en-us/windows/apps/develop/composition/composition-animation)

### Inferences
- A Windows token set can reuse M3 spring numbers for physics and Fluent durations for tween fallbacks: 83/167/250 ms for fast/normal/slow, decelerate `(0,0,0,1)` for enter, accelerate `(1,0,1,1)` for exit.
- Fluent's fade-for-top-level-navigation guidance matches the reduced-motion replacement in section 5.

### Gaps
- The `SpringNaturalMotionAnimation` API page (learn.microsoft.com/uwp/api/windows.ui.composition.springvectornaturalmotionanimation) returned 404, and the spring-animations walkthrough was not fetched. I have no verified DampingRatio/Period ranges or defaults. Only the 0.07 s Period in the doc example is confirmed.
- The Fluent 2 page gives no millisecond durations. The 50-500 ms values come from the Fluent UI token package, not from the design guidance page.

## 3. Apple spring conventions (cross-reference)

### Takeaway
SwiftUI uses `duration` and `bounce` (-1..1) as its modern parameterisation, with the older `response` and `dampingFraction` as the legacy form. I verified the preset names and the bounce guidance but not the numeric bounce values of `.smooth`, `.snappy` and `.bouncy`.

### Cited Findings
- WWDC23 "Animate with springs": three presets (smooth, snappy, bouncy), tunable via `duration` or `extraBounce` (for example `.snappy(duration: 0.4)`). Bounce range is -1.0 to 1.0. About 15% barely feels bouncy; about 30% is noticeably bouncy; avoid values above roughly 0.4 for UI elements; bounce 0 is the default and most versatile. — [WWDC23 10158 notes](https://wwdcnotes.com/documentation/wwdc23-10158-animate-with-springs/)
- "You shouldn't wait for the settling duration for user-facing changes." SwiftUI completions use a perceptual duration. Core Animation exposes `CASpringAnimation(perceptualDuration:bounce:)`. — [WWDC23 10158 notes](https://wwdcnotes.com/documentation/wwdc23-10158-animate-with-springs/)
- Third-party conversion (mass 1): stiffness = (2π / duration)²; damping = 1 - 4π·bounce / duration (for bounce >= 0). This is a library doc, not Apple. — [Remix SpringPreset docs](https://api.remix.run/api/remix/ui/animation/type/SpringPreset.md)
- Legacy `.spring` defaults reported as response 0.55, dampingFraction 0.825. This is a community source. — [Create with Swift](https://www.createwithswift.com/understanding-spring-animations-in-swiftui/)

### Inferences
- The repo's tokens could map to Apple terms as ζ = dampingFraction, and response ≈ 2π / ω_n (Inference, standard second-order relation). Under this mapping M3 expressive default spatial (k = 380) is about 0.32 s response. This is my calculation.

### Gaps
- Apple developer.apple.com pages (Spring.smooth, HIG Motion, accessibilityReduceMotion) are JS-rendered and returned only titles. I have no verified numeric values for `.smooth`/`.snappy`/`.bouncy`. Commonly quoted values (0 / 0.15 / 0.3 bounce, 0.5 s duration) come from model memory and should not be relied on without checking Xcode docs.
- Apple's wording on purposeful motion in the HIG could not be read.

## 4. Converting a spring to a duration-less step function in a custom renderer

### Takeaway
Do not precompute a duration. Each frame, advance the closed-form damped-oscillator solution (or sub-stepped integration) from the current value and velocity, and stop when both displacement and velocity are below thresholds. Retargeting mid-flight keeps position and velocity, so there is no discontinuity.

### Cited Findings
- Equation of motion: m·x'' + c·x' + k·x = 0, normalised x'' + 2ζω_n·x' + ω_n²·x = 0; ω_n = sqrt(k/m); c_c = 2·sqrt(k·m); ζ = c / c_c; roots s = -ω_n(ζ ± i·sqrt(1 - ζ²)). — [Damping ratio, Wikipedia](https://en.wikipedia.org/wiki/Damping_ratio)
- Underdamped (0 <= ζ < 1): x(t) = A·e^(-ζ·ω_n·t)·cos(ω_d·t - φ), ω_d = ω_n·sqrt(1 - ζ²). Critical (ζ = 1): x(t) = (C1 + C2·t)·e^(-ω_n·t). Overdamped: sum of two decaying exponentials. These closed forms were derived in the fetch summary from the article's roots. Decay rate α = ζ·ω_n. Overshoot PO = 100·exp(-ζπ / sqrt(1 - ζ²)). — [Damping ratio, Wikipedia](https://en.wikipedia.org/wiki/Damping_ratio)
- Compose's "rounding off" threshold is 0.01 (`DefaultDisplacementThreshold`). — [Spring](https://developer.android.com/reference/kotlin/androidx/compose/animation/core/Spring)

### Inferences (my derivations, not from a cited source)
Let x = value - target (displacement), v = velocity, ω = sqrt(k) (mass 1).
- Underdamped step from (x0, v0): ω_d = ω·sqrt(1-ζ²); x(t) = e^(-ζωt)·[x0·cos(ω_d·t) + ((v0 + ζω·x0)/ω_d)·sin(ω_d·t)].
- Critically damped: x(t) = e^(-ωt)·[x0 + (v0 + ω·x0)·t].
- Overdamped: r = ω·sqrt(ζ² - 1); x(t) = e^(-ζωt)·[x0·cosh(rt) + ((v0 + ζω·x0)/r)·sinh(rt)].
- Velocity: differentiate the same expressions, or use a one-frame finite difference.
- Per frame: take the real elapsed dt (clamp to about 50 ms), re-evaluate from the stored (x, v) over dt (treat each frame as a fresh start with new initial conditions). This is exact, frame-rate independent and stable for any stiffness. Semi-implicit Euler needs sub-stepping at stiffness 3800.
- Settling test: |x| < ε_pos and |v| < ε_vel, then snap to target. Choose ε_pos about 0.5 device pixel for spatial properties (stops sub-pixel shimmer) or 0.01 as in Compose's default for unit-range values (opacity). Choose ε_vel about ε_pos·ω.
- Approximate time to settle to ε (as a fraction of the initial displacement): underdamped t ≈ ln(1/ε) / (ζ·ω); for ε = 0.01 that is 4.6 / (ζ·ω). Critically damped: solve (1 + ωt)·e^(-ωt) = ε, giving ωt ≈ 6.64 for ε = 0.01.
- Estimated 1% settling times (my calculations, mass 1):

| Token | k | ζ | ω_n (rad/s) | ~t (1%) |
|---|---|---|---|---|
| Expr. spatial fast | 800 | 0.6 | 28.3 | 0.27 s |
| Expr. spatial default | 380 | 0.8 | 19.5 | 0.30 s |
| Expr. spatial slow | 200 | 0.8 | 14.1 | 0.41 s |
| Std spatial fast | 1400 | 0.9 | 37.4 | 0.14 s |
| Std spatial default | 700 | 0.9 | 26.5 | 0.19 s |
| Std spatial slow | 300 | 0.9 | 17.3 | 0.30 s |
| Effects fast | 3800 | 1.0 | 61.6 | 0.11 s |
| Effects default | 1600 | 1.0 | 40.0 | 0.17 s |
| Effects slow | 800 | 1.0 | 28.3 | 0.23 s |

- Overshoot for expressive spatial from PO formula: ζ = 0.6 gives about 9.5%; ζ = 0.8 gives about 1.5%; ζ = 0.9 gives about 0.2%. (Calculated from the cited overshoot formula.) So only "fast" expressive spatial visibly bounces.
- Retargeting: keep (x, v) and just change the target, shifting x by the target delta. Do not reset velocity. This matches Compose's interruption behaviour cited above.
- Direct2D does not clamp: clamp animated opacity/colour to [0, 1] anyway, or use effects springs, which do not overshoot.
- Redraw only while any spring is unsettled (request frames on demand) so idle UI costs nothing.

### Gaps
- Compose mass = 1 and its exact settle criterion (it also uses velocity threshold) were not verified in source.

## 5. Reduced-motion behaviour (Android, Windows, iOS) and WCAG

### Takeaway
All three platforms have a system switch, but only Windows and iOS expose it as a direct "is it on" API; Android requires reading the animator duration scale. WCAG 2.3.3 is Level AAA and asks that non-essential interaction-triggered motion can be turned off. Replace travel with a short cross-fade.

### Cited Findings
- WCAG 2.3.3 Animation from Interactions: Level AAA; users must be able to turn off motion animation triggered by an interaction unless essential. "Essential" means removing it "would fundamentally change the information or functionality". Recommended techniques: CSS `prefers-reduced-motion` (C39), the same query via JavaScript (SCR40), or a user-set preference. Self-starting animation falls under 2.2.2, not 2.3.3. — [Understanding SC 2.3.3](https://www.w3.org/WAI/WCAG21/Understanding/animation-from-interactions.html)
- Windows: `SPI_GETCLIENTAREAANIMATION` (0x1042) "determines whether animations are enabled or disabled", returning TRUE if enabled via a BOOL. Microsoft ties this to photosensitive-epilepsy risk from "flashing, blinking, flickering, and moving content" in the client area. Not supported before Windows XP/2000 era. — [SystemParametersInfo](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfoa)
- Android: `Settings.Global.ANIMATOR_DURATION_SCALE` is the "Scaling factor for Animator-based animations". `ValueAnimator.areAnimatorsEnabled()` (API 26) "returns whether animators are currently enabled, system-wide". — [Settings.Global](https://developer.android.com/reference/android/provider/Settings.Global); [ValueAnimator](https://developer.android.com/reference/android/animation/ValueAnimator)
- Android "Remove animations": the setting is not surfaced like screen-reader state; developers read the animator duration scale, which is 0f when reduced motion is on, with a fallback of 1f when the value is absent (older than Android 12 per one author). Compose animations built on Animator-based APIs honour it automatically per one author, but `basicMarquee` did not. These are community write-ups, not official docs. — [Eevis, Android animations and reduced motion](https://dev.to/eevajonnapanula/android-animations-and-reduced-motion-2em2); [basicMarquee accessibility](https://proandroiddev.com/making-basicmarquee-modifier-more-accessible-8b3737307dee)
- iOS: with Reduce Motion on, "screen transitions and effects use the dissolve effect instead of zoom or slide effects". The separate "Prefer Cross-Fade Transitions" toggle is available only after Reduce Motion is enabled. — [Apple Support, Reduce screen motion](https://support.apple.com/en-us/HT202655); [AbilityNet guide](https://mcmw.abilitynet.org.uk/how-to-reduce-motion-effects-in-ios-16-on-your-iphone-or-ipad) (note: the support search snippet came from the Apple page, the toggle detail from AbilityNet).
- Material guidance: include a "no motion" setting (WCAG), keep durations short and movement natural, avoid flashes and sudden motion, constrain motion to the focused element, and convey animated information by other means (ARIA live regions). — [Fluent 2 Motion](https://fluent2.microsoft.design/motion) (the Material accessibility page text was relayed by the fetch tool from the M3 accessibility/motion section and not directly verified).

### Inferences
- Recommended behaviour when reduced motion is on: replace every spatial spring with a cross-fade (opacity only) of about 100-150 ms. Use the Fluent "Faster"/"Fast" values (100/150 ms) or M3 Short2/Short3 (100/150 ms) with the decelerate or standard curve. Keep state feedback (colour change, focus ring) but drop travel, scale, parallax and looping animation. Run progress indicators as static or as slow fades. Put the switch in one place in the renderer (springs resolve instantly, crossfades keep their own timing), so no component needs its own branch.
- Re-read the preference on change, not only at launch: on Windows listen for WM_SETTINGCHANGE (SPI_SETCLIENTAREAANIMATION), and on Android re-check the duration scale when the app resumes. (Inference; the events themselves were not verified in these sources.)
- Windows UI naming ("Animation effects" under Accessibility > Visual effects) was not verified in a fetched source; treat it as inference from the SPI flag.

### Gaps
- No Apple developer doc confirmed `accessibilityReduceMotion` wording (page unreadable via fetch). No official Android doc stating "Remove animations" sets duration scale to 0 was found; only community articles.
- Which Compose APIs honour duration scale 0 (spring-based `animate*AsState` versus Animator-based) was not verified from official docs. Note that Compose's own animations are not Animator-based; treat the "automatic" claim as unconfirmed and check `ValueAnimator.areAnimatorsEnabled()` yourself.

## 6. When motion is meaningful versus decorative

### Takeaway
Both Fluent 2 and Material tie motion to a job: show what changed, where something came from, or acknowledge an action. Everything else should be dropped or limited.

### Cited Findings
- Fluent 2 "Functional" principle: "Motion is applied with purpose and intent to serve the functionality of an experience." Use motion to signal the next step, show UI changes, and acknowledge completed actions. Limit motion to the element in focus because motion elsewhere distracts. — [Fluent 2 Motion](https://fluent2.microsoft.design/motion)
- Fluent 2 on navigation: prefer a quick fade over sliding large elements for top-level navigation. — [Fluent 2 Motion](https://fluent2.microsoft.design/motion)
- Windows connected animation example: an item image "continues" into the next page header, which "helps maintain user context across the transition". Parallax is described as creating depth. — [Composition animations](https://learn.microsoft.com/en-us/windows/apps/develop/composition/composition-animation)
- Material: duration scales with area and travel; large elements get more time, small ones less. — [Motion.md](https://raw.githubusercontent.com/material-components/material-components-android/master/docs/theming/Motion.md)
- Compose enter/exit and shared-element style APIs describe their role as introducing or dismissing UI, with elements entering "from off-screen or from within the screen bounds". — [Compose AnimationSpec/animation docs fetch](https://developer.android.com/develop/ui/compose/animation/customize) (summarised through the fetch tool; not independently verified against the page).
- Apple: WWDC23 says to use bounce 0 if unsure and keep bounce modest for UI elements. — [WWDC23 10158 notes](https://wwdcnotes.com/documentation/wwdc23-10158-animate-with-springs/)
- WCAG distinguishes essential from non-essential motion: parallax and similar motion triggered by interaction is the classic non-essential case. — [Understanding SC 2.3.3](https://www.w3.org/WAI/WCAG21/Understanding/animation-from-interactions.html)

### Inferences
- A usable rule for the design system: animate only (1) spatial continuity (where did this come from or go), (2) state change feedback under 150 ms, (3) attention for a changed value. No looping ornament and no entrance animation on first paint. Anything that fails the "what does it mean" test is cut.
- Use bounce only on spatial properties of user-initiated, direct-manipulation gestures (sheets, drag release), never on status, error or text.

### Gaps
- I did not find an explicit "decorative vs meaningful" criterion in Material's own text; the M3 guidance pages were unreadable via the fetch tool.

## 7. Reference apps with good motion

### Takeaway
Only first-party reference implementations could be tied to a URL. I found no reliable published critique of third-party app motion, so I am not listing consumer apps I cannot cite.

### Cited Findings
- WinUI 3 Gallery (Microsoft): contains interactive examples of WinUI controls and features, including motion. Store: [Microsoft Store listing](https://apps.microsoft.com/detail/9P3JFPWWDZRC); source: [github.com/microsoft/WinUI-Gallery](https://github.com/microsoft/WinUI-Gallery). — cited on [Timing and easing](https://learn.microsoft.com/en-us/windows/apps/design/motion/timing-and-easing)
- Windows connected animations and parallax are shown in the Windows motion docs. — [Composition animations](https://learn.microsoft.com/en-us/windows/apps/develop/composition/composition-animation)
- Material 3 Expressive in Google's own apps (for example Clock) is covered by press: Google says it was built from 46 studies and over 18,000 participants. This is Google's claim relayed by press, not independently verified. — [Android Authority, Google Clock Expressive](https://www.androidauthority.com/google-clock-expressive-features-changes-availability-supported-devices-3556392/); [9to5Google, M3 Expressive](https://9to5google.com/2025/05/05/material-3-expressive-leak/)
- The Material 3 Expressive source of truth to read and copy from is the Compose `MotionScheme` and token files cited in section 1. — [androidx material3 source](https://raw.githubusercontent.com/androidx/androidx/androidx-main/compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/MotionScheme.kt)

### Inferences
- Candidates worth checking by hand on a device (not researched or cited): Google Clock/Material Catalog on Android, Settings and Start menu flyouts on Windows 11, and iOS sheet/Dynamic Island motion as the spring reference.

### Gaps
- Fewer than the three requested third-party references with URLs: the search returned generic inspiration galleries only (Dribbble-sourced concepts, not shipped apps). Needs a follow-up with targeted searches.
- The Material 3 Expressive Google blog post and m3.material.io motion pages could not be read.
