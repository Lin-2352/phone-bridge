# Citation review: Relay design system references

Reviewed 2026-11-06. Every URL was fetched with WebFetch.

| # | URL | Verdict | Note |
|---|---|---|---|
| 1 | learn.microsoft.com/.../signature-experiences/geometry | CONFIRMED | 8px for windows, flyouts, dialogs (ContentDialog, Flyout, MenuFlyout, TeachingTip); 4px for in-page controls; ToolTip is a 4px exception; 0px on touching edges. |
| 2 | learn.microsoft.com/.../style/acrylic | PARTIAL | Five layers confirmed (background, blur, exclusion blend, tint, noise). Solid fallback confirmed for transparency off, Battery Saver and low-end hardware. Only background acrylic goes solid when the window deactivates, so in-app acrylic does not. The page does not say "only transient surfaces": it also recommends in-app acrylic for supporting UI, panes and NavigationView, and says "don't" for large backgrounds and multiple adjacent panes. No accent text or links on acrylic is confirmed. |
| 3 | learn.microsoft.com/.../style/mica | CONFIRMED | Mica is for the backdrop of long-lived windows. Solid fallback (SolidBackgroundFillColorBase) when transparency is off, Battery Saver, low-end hardware, window inactive, or Windows below 22000. |
| 4 | learn.microsoft.com/.../signature-experiences/typography | CONFIRMED | Caption 12/16, Body 14/20, Body Large 18/24, Subtitle 20/28, Title 28/36. Regular and Semibold weights; Bold is not part of the ramp. The ramp also has Title Large 40/52 and Display 68/92. |
| 5 | learn.microsoft.com/.../signature-experiences/motion | PARTIAL | Durations 83/167/250/333 ms confirmed. Enter (0,0,0,1) confirmed. (1,0,1,1) is "Gentle Exit" at 167 ms for position and scale only. "Direct Exit" is (0,0,0,1) at 167 ms and must be combined with a fade. The 83 ms linear curve is opacity-only. Point-to-point (0.55,0.55,0,1) confirmed. The "taskbar flyouts slide up in and down out" claim is confirmed. |
| 6 | learn.microsoft.com/.../accessibility/high-contrast-themes | CONFIRMED | "We recommend using 2px borders for transitory surfaces such as flyouts and dialogs." Palettes are typically at or above 7:1. Do not use GrayText for secondary text. SystemColor pairings are required. |
| 7 | w3.org/WAI/WCAG22/Understanding/non-text-contrast.html | CONFIRMED | 3:1 against adjacent colours for what identifies a component and its state. #767676 input border passes; #AAA with no label fails. Inactive (disabled) components are exempt. |
| 8 | w3.org/WAI/WCAG22/Understanding/contrast-minimum.html | CONFIRMED | At least 4.5:1 for text; large text 3:1. |
| 9 | w3.org/WAI/WCAG21/Understanding/animation-from-interactions.html | CONFIRMED | Titled "Animation from Interactions (Level AAA)". |
| 10 | fluent2.microsoft.design/color | CONFIRMED | Brand colour for buttons, CTAs and selected states. Overuse can "dilute a hierarchy". Semantic red/yellow/green are for important messages: "Don't use them for decoration." |
| 11 | fluent2.microsoft.design/motion | CONFIRMED | Motion "with purpose and intent to serve the functionality"; "keep the motion constrained to the element in focus"; quick fade instead of sliding for top-level navigation. |
| 12 | raw.githubusercontent.com/chrisbanes/haze/.../FluentMaterials.kt | CONFIRMED | Acrylic dark #2C2C2C, tint 0.15, luminosity 0.96; light #FCFCFC, tint 0, luminosity 0.85; blur 60dp; noise 0.02. Mica blur 240dp, noise 0. |
| 13 | raw.githubusercontent.com/chrisbanes/haze/.../HazeMaterials.kt | CONFIRMED | Blur 24dp for all presets. Light/dark tint alphas: ultraThin 0.35/0.55, thin 0.6/0.65, regular 0.73/0.8, thick 0.83/0.9, ultraThick 0.92/0.97. |
| 14 | raw.githubusercontent.com/androidx/androidx/.../ShapeTokens.kt | CONFIRMED | 0, 4, 8, 12, 16, 20, 28, 32, 48 dp, plus Full (CircleShape). |
| 15 | raw.githubusercontent.com/androidx/androidx/.../StandardMotionTokens.kt | CONFIRMED | Spatial damping 0.9 with stiffness 700/1400/300. Effects damping 1.0 with stiffness 1600/3800/800. |
| 16 | raw.githubusercontent.com/androidx/androidx/.../ExpressiveMotionTokens.kt | CONFIRMED | Spatial default 0.8/380, fast 0.6/800, slow 0.8/200. Effects 1.0 with 1600/3800/800. |
| 17 | nngroup.com/articles/liquid-glass/ | CONFIRMED | "light, airy — and often invisible"; text on images is "a bad idea"; "Apple is prioritizing spectacle over usability". |
| 18 | developer.apple.com/videos/play/wwdc2025/219/ | CONFIRMED | Small elements flip light/dark and glyphs flip. Clear "needs a dimming layer". Reduce Transparency makes it "frostier". Increased Contrast makes it black or white with a contrasting border. |
| 19 | macrumors.com/2025/10/20/ios-26-1-liquid-glass-toggle/ | CONFIRMED | "tinted increases the opacity of Liquid Glass and adds more contrast". |
| 20 | developer.android.com/training/permissions/requesting | CONFIRMED | Ask in context; explain why before `requestPermissions()`; "persistent nagging... is not respectful"; "a different button that contains an error icon and color". |
| 21 | developer.android.com/media/grow/media-projection | CONFIRMED | Consent before each session. Status-bar chip lets the user stop (Android 15 QPR1 or higher only). Projection auto-stops on screen lock. `onStop()` fires for chip, lock, another session or process death. |
| 22 | source.android.com/docs/core/display/window-blurs | CONFIRMED | Public APIs from Android 12 (API 31). "The system server might temporarily disable window blurs at runtime"; `isCrossWindowBlurEnabled` plus a listener. |
| 23 | learn.microsoft.com/.../app-settings/guidelines-for-app-settings | PARTIAL | Live apply confirmed. SettingsCard rows (header, description, icon, right-aligned control), SettingsExpander and the About expander are confirmed. A left nav is not prescribed for the settings page: Settings is the last NavigationView item, and the page is a single scrollable column of at most about 1000-1100 px. The page also advises four or five settings at most. |
| 24 | learn.microsoft.com/.../dialogs-and-flyouts/dialogs | CONFIRMED | Specific verbs (Allow/Block). A safe CloseButton is required and maps to Esc. Only one ContentDialog per window. The DefaultButton is optional and not set by default, so the "safe default" wording should be "safe close action". |
| 25a | github.com/microsoft/fluentui-system-icons | CONFIRMED | Lists the MIT licence. Regular and Filled styles exist. The 16/20/24 sizes are not stated on the page. |
| 25b | developers.google.com/fonts/docs/material_symbols | CONFIRMED | Apache 2.0. Weight 400 default; 24px preferred. The page is ambiguous on the default optical size (48 in the link, 24 in the static font). The 48dp target size is not on this page; it comes from the Compose API defaults page. |
| 26 | impeccable.style/anti-patterns | CONFIRMED | Gradient text, nested cards, icon tile above heading, side-stripe accent borders and glow are all listed. Emoji icons are not listed as an anti-pattern, so checklist item 2 ("No emoji in chrome") does not map to Impeccable. |

## Changes recommended for the report

1. Motion (#5): state that (1,0,1,1) is "Gentle Exit", 167 ms, position and scale only. "Direct Exit" is (0,0,0,1) at 167 ms and must be combined with a fade. Adjust the "Easing" row and the "PC motion" row ("out 167 ms with fade") accordingly.
2. Acrylic (#2): change "acrylic only for transient surfaces" to "background acrylic for transient surfaces; in-app acrylic allowed for supporting UI". Add that only background acrylic goes solid when the window is inactive, so the report's "window inactive" fallback applies to background acrylic only.
3. Settings (#23): drop "left nav" as a settings-page guideline, or cite it as the app's NavigationView with Settings pinned last. Add the single-column and 4-5 settings guidance.
4. Dialog (#24): replace "safe default" with "safe close action". The DefaultButton is optional.
5. Minor: the MediaProjection status-bar chip applies only to Android 15 QPR1 or higher. Checklist item 2 (emoji) is not from Impeccable.
