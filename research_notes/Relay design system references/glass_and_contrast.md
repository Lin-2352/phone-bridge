# Translucent "glass" materials and keeping text readable on them

Research date: 2026-11-06. Items marked (derived) were computed by me, not taken from a source. The WCAG contrast formula was used for all derived numbers.

## 1. Apple Liquid Glass (iOS 26 / macOS Tahoe): official guidance, legibility mechanisms, criticism, later changes

### Takeaway
Apple's published answer to legibility is behavioural, not numeric. Each glass layer adapts light/dark, tint and shadow to what is behind it, and the user can switch to Reduce Transparency or Increase Contrast. Apple publishes no opacity, blur or contrast numbers for Liquid Glass. Criticism (NN/g and others) centred on glass over busy content. Apple responded with more opaque defaults in betas and a Clear/Tinted toggle in 26.1.

### Cited Findings
- Legibility is a central design concern. Small elements (nav bars, tab bars) constantly adapt and can flip between light and dark. Larger elements (menus, sidebars) adapt but do not flip, because the transition would be distracting. — [WWDC25 219 "Meet Liquid Glass"](https://developer.apple.com/videos/play/wwdc2025/219/)
- Symbols and glyphs on Liquid Glass flip between light and dark to maximise contrast. The Regular variant does this automatically. — [WWDC25 219](https://developer.apple.com/videos/play/wwdc2025/219/)
- Each glass layer "continuously adapts based on what's behind it". Tint amount and dynamic range shift to keep buttons legible while letting as much content through as possible. — [WWDC25 219](https://developer.apple.com/videos/play/wwdc2025/219/)
- Shadow adapts: opacity increases over text and decreases over a solid light background. Larger glass gets deeper shadows. — [WWDC25 219](https://developer.apple.com/videos/play/wwdc2025/219/)
- Two variants exist. Regular is the default and "works at any size, over any content". Clear is permanently more transparent, needs a dimming layer to darken the content beneath, and its legibility drops noticeably without one. Clear is only for use when all three hold: the element sits over media-rich content, the content layer is not harmed by dimming, and the content above is bold and bright. The two variants should never be mixed. The transcript gives no dimming percentage. — [WWDC25 219](https://developer.apple.com/videos/play/wwdc2025/219/)
- Tinting is supported but should be selective: primary elements and actions only. — [WWDC25 219](https://developer.apple.com/videos/play/wwdc2025/219/)
- Accessibility modifiers apply automatically when the system setting is on.
  - Reduced Transparency makes glass frostier and obscures more of what is behind it.
  - Increased Contrast makes elements predominantly black or white with a contrasting border.
  - Reduced Motion reduces effect intensity and disables the elastic properties.
  — [WWDC25 219](https://developer.apple.com/videos/play/wwdc2025/219/)
- "Elements using Liquid Glass require clear separation from content to maintain legibility." Controls should sit on a system material, not directly on content. Scroll edge effects (soft, or hard on macOS) provide that boundary as a subtle blur and "don't block or darken like overlays". — [WWDC25 356 "Get to know the new design system"](https://developer.apple.com/videos/play/wwdc2025/356/)
- System colours were retuned across Light, Dark and Increased Contrast for Liquid Glass. — [WWDC25 356](https://developer.apple.com/videos/play/wwdc2025/356/)
- For modal tasks, pair glass with a dimming layer. Glass becomes more opaque when focus shifts, for example when dragging a sheet up. — [WWDC25 356](https://developer.apple.com/videos/play/wwdc2025/356/)
- Press coverage of WWDC25 confirms the same mechanisms (adaptive shadow, light/dark switching, larger elements look thicker and frostier) and recommends restraint, mainly for navigation-layer controls. — [MobileSyrup](https://mobilesyrup.com/2025/06/10/how-apple-is-making-sure-liquid-glass-as-legible-as-possible/)
- NN/g (Raluca Budiu, 10 Oct 2025), "Liquid Glass Is Cracked, and Usability Suffers in iOS 26":
  - Translucent controls are "often invisible".
  - "Text on top of images is a bad idea" because contrast is often too low (Messages with photo backgrounds).
  - Maps bottom icons blend into imagery.
  - Mail search bar over previews gives "text on top of text creates an illegible mess".
  - The Safari URL bar is squeezed and truncated.
  - The pale search bar fades into the background.
  — [NN/g](https://www.nngroup.com/articles/liquid-glass/)
- A secondary summary adds that NN/g also flagged touch targets under 1 cm, and that Control Center was "nearly unreadable" in beta 1, so Apple made it more opaque. The same secondary source says shadow and transparency were tuned over several updates, and 26.2 gave users more control. It is a blog and not authoritative. — [nativefirst blog](https://nativefirst.pages.dev/blog/liquid-glass-turns-one-still-squinting-ios-27-wwdc-2026)
- iOS/iPadOS/macOS 26.1 beta 4 (20 Oct 2025) added a Liquid Glass setting with two options, Clear and Tinted. Tinted "increases opacity and adds more contrast". It applies across apps and Lock Screen notifications. Location: Settings > Display & Brightness on iOS, System Settings > Appearance on Mac. It is binary, with no slider. Apple reportedly added it after beta-tester feedback asking for a more opaque option. Before it, the only route was Accessibility > Reduce Transparency. — [MacRumors](https://www.macrumors.com/2025/10/20/ios-26-1-liquid-glass-toggle/), [Tom's Guide](https://www.tomsguide.com/phones/iphones/ios-26-1-lets-you-adjust-liquid-glass-transparency-on-your-iphone-heres-how-to-do-it)

### Inferences
- The "Clear needs a dim layer" rule and the "Tinted" mode are Apple's own admission that unmodified translucent glass does not reliably give text contrast. The shipped fix was higher opacity, which matches the numeric analysis in section 4.
- Apple's design pattern is to avoid placing body text directly on glass. It puts glyphs and short labels on glass, and keeps text on content or system materials.

### Gaps
- The HIG Materials page and the "Adopting Liquid Glass" page returned no usable body text through the fetch tool, so there are no HIG-quoted values. I found no Apple-published opacity, blur-radius or contrast-ratio numbers for Liquid Glass.
- iOS 27 / WWDC 2026 changes: only the unverified blog above mentions a rumoured system-wide intensity slider. I found no primary source, so it is unconfirmed.
- I did not retrieve the exact text of the 26.1 release notes. The "increases opacity and adds more contrast" wording is relayed by the press.

## 2. Windows Acrylic and Mica

### Takeaway
Acrylic is a recipe of five layers: background, blur, exclusion blend, tint/luminosity overlay, noise. Microsoft restricts it to transient or supporting surfaces, and it falls back to a solid colour in many conditions. Microsoft's pages do not state default numbers. The numbers below come from Haze's Fluent port, which mirrors WinUI.

### Cited Findings
- Recipe, in order: background, blur, exclusion blend, colour/tint overlay, noise. Design intent: translucency, blur and noise first, then an exclusion blend layer "to ensure contrast and legibility of UI placed on an acrylic background", then tint for personalisation. — [Microsoft Learn: Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)
- Background acrylic shows the desktop and windows behind the app. In-app acrylic blurs content within the app frame. Background acrylic is for transient UI (context menus, flyouts, light-dismiss). In-app acrylic is for supporting UI that overlaps scrolling content. Vertical panes that section the app should use an opaque background. — [Microsoft Learn: Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)
- Do not put desktop acrylic on large background surfaces. Do not put acrylic panes edge-to-edge (visible seam). Do not layer acrylic. Do not place accent-coloured text or hyperlinks on acrylic, because these are likely to fail minimum contrast at the default 14px size. — [Microsoft Learn: Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)
- "We've optimized the acrylic resources such that text meets contrast ratios on top of acrylic." If you customise tint or opacity, you own the legibility impact. — [Microsoft Learn: Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)
- Custom AcrylicBrush properties:
  - TintColor: the tint overlay colour.
  - TintOpacity: opacity of the tint layer.
  - TintLuminosityOpacity: how much saturation/luminosity passes through from the background. If omitted, the system derives it from TintColor and TintOpacity.
  - FallbackColor: the solid colour used when acrylic cannot render.
  - Docs sample: TintOpacity 0.8, TintLuminosityOpacity 0.5. Dark fallback #FF7F0000, light fallback #FFFF7F7F.
  — [Microsoft Learn: In-app acrylic](https://learn.microsoft.com/en-us/windows/apps/develop/ui/in-app-acrylic)
- Mica is "an opaque material" that samples the wallpaper once, so it is cheap. It is meant as the base layer, especially in the title bar. Mica Alt is the BaseAlt variant. — [Microsoft Learn: System backdrops](https://learn.microsoft.com/en-us/windows/apps/develop/ui/system-backdrops)
- Acrylic is used only for transient, light-dismiss surfaces. Controller kinds are DesktopAcrylicKind.Base and Thin. Controller overrides are FallbackColor, LuminosityOpacity, TintColor, TintOpacity. After overriding any of these, the theme-based defaults stop applying. — [Microsoft Learn: System backdrops](https://learn.microsoft.com/en-us/windows/apps/develop/ui/system-backdrops)
- Numeric defaults (Haze's Fluent port, `acrylicDefault`; I take it to mirror WinUI but did not verify against WinUI source):
  - Dark: container #2C2C2C, tint opacity 0.15, luminosity opacity 0.96, fallback #2C2C2C.
  - Light: container #FCFCFC, tint opacity 0, luminosity opacity 0.85, fallback #F9F9F9.
  - Blur 60 dp, noise factor 0.02 (2%).
  — [Haze FluentMaterials.kt](https://raw.githubusercontent.com/chrisbanes/haze/main/haze-materials/src/commonMain/kotlin/dev/chrisbanes/haze/blur/materials/FluentMaterials.kt)
- Haze Mica: blur 240 dp, noise 0.
  - Mica dark: tint 0.8, luminosity 1.0, container #202020.
  - Mica light: tint 0.5, luminosity 1.0, container #F3F3F3.
  - Mica Alt dark: tint 0, container #0A0A0A.
  - Mica Alt light: tint 0.5, container #DADADA.
  — [Haze FluentMaterials.kt](https://raw.githubusercontent.com/chrisbanes/haze/main/haze-materials/src/commonMain/kotlin/dev/chrisbanes/haze/blur/materials/FluentMaterials.kt)
- Fallbacks to a solid colour:
  - Both acrylic types: High Contrast mode (the user's own background colour is shown), Transparency effects turned off (Settings > Personalization > Colors), Battery Saver, low-end hardware.
  - Background acrylic only: window deactivated, and Xbox/HoloLens/tablet mode.
  - For high contrast, Microsoft recommends a SolidColorBrush (`SystemColorWindowColor`) under the same key.
  — [Microsoft Learn: Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic), [In-app acrylic](https://learn.microsoft.com/en-us/windows/apps/develop/ui/in-app-acrylic)

### Inferences
- The luminosity layer is Microsoft's scrim: luminosity 0.85 to 0.96 means almost the whole surface is neutral colour and only 4 to 15% passes the backdrop's brightness. Acrylic stays AA-safe by being close to opaque, not by being genuinely clear.
- Acrylic is not a wide-use panel material even on Windows. Microsoft keeps it to flyouts and menus, and uses opaque panes and Mica elsewhere.

### Gaps
- Microsoft's pages do not state the noise opacity, the exclusion-blend parameters, or the default tint/luminosity numbers. The 0.02 noise and 0.15/0.96/0.85 values come only from Haze's port.
- Attempts to fetch the WinUI theme-resource XAML via search found no confirming numbers.

## 3. Android / Compose glass: Haze presets and platform blur

### Takeaway
Haze's Material-style presets use one blur radius (24 dp) and vary only the tint alpha from 0.35 to 0.97. Contrast comes from tint alpha, not from blur. Haze's own docs warn that larger blur may be needed for text. Android blur silently disappears in some conditions, so a solid fallback is mandatory.

### Cited Findings
- Haze `HazeMaterials`: all presets use `blurRadius(24.dp)`. The tint alpha is chosen by container luminance (`containerColor.luminance() >= 0.5` selects the light alpha). The source defines no noise, saturation or luminosity for these presets.

  | Preset | Light alpha | Dark alpha |
  |---|---|---|
  | ultraThin | 0.35 | 0.55 |
  | thin | 0.60 | 0.65 |
  | regular | 0.73 | 0.80 |
  | thick | 0.83 | 0.90 |
  | ultraThick | 0.92 | 0.97 |

  — [Haze HazeMaterials.kt](https://raw.githubusercontent.com/chrisbanes/haze/main/haze-materials/src/commonMain/kotlin/dev/chrisbanes/haze/blur/materials/HazeMaterials.kt)
- Haze also ships `CupertinoMaterials` (Apple-style) and `FluentMaterials`. Presets are styles that can be overridden with `then`, for example `HazeMaterials.thin().then { blurRadius(12.dp); noiseFactor(0f) }`. — [Haze materials docs](https://chrisbanes.github.io/haze/latest/blur/materials/)
- Haze usage-doc examples (documented as examples, not defaults): `blurRadius(20.dp)`, `noiseFactor(0.15f)`, white tint at alpha 0.12, and a black fallback tint at alpha 0.7. When blur is not available (Android 11 and below), Haze draws a translucent scrim instead, via the fallback effect. — [Haze blur usage](https://chrisbanes.github.io/haze/latest/blur/usage/)
- Haze blur and tint order: `HazeBlurDefaults.style`, then `LocalHazeBlurStyle`, then the explicit style. The last write wins. — [Haze blur usage](https://chrisbanes.github.io/haze/latest/blur/usage/)
- Platform window blur needs Android 12+ (API 31). It can be off at runtime (battery saver, multimedia tunnelling, minimal post-processing, or GPU limits). Apps should listen via `WindowManager#addCrossWindowBlurEnabledListener` and Compose exposes `WindowInfo.isCrossWindowBlurEnabled`. — [AOSP window blurs](https://source.android.com/docs/core/display/window-blurs), [Compose isCrossWindowBlurEnabled](https://composables.com/jetpack-compose/androidx.compose.ui/ui/properties/isCrossWindowBlurEnabled/api)
- Reference value for another platform: iOS-style preset alpha and the Haze "thin" alpha of 0.6 to 0.65 are comparable.

### Inferences
- Haze's thin to regular range (0.60 to 0.80) matches the AA threshold computed in section 4 only when the text colour matches the tint colour's opposite and the tint is neutral. ultraThin (0.35 / 0.55) cannot carry body text over a worst-case backdrop.

### Gaps
- I did not find documented border alpha or inner-highlight values in the Haze presets or from real shipping apps. I found no verified source for "typical" border alpha. Treat any border or highlight number as a design choice to be set and tested by us.
- Haze's `HazeBlurDefaults` and `CupertinoMaterials.kt` numbers were not retrievable (404 on the guessed path).

## 4. Contrast technique comparison and minimum opacity

### Takeaway
Systems converge on one technique: a neutral scrim or tint layer between blur and text, with high opacity (roughly 0.6 to 0.97) and, where needed, a switch to solid. Blur alone does not guarantee contrast. By my computation, a neutral tint needs about 0.58 alpha for 4.5:1 and about 0.45 for 3:1 against a worst-case opposite backdrop.

### Cited Findings
- WCAG 1.4.3: 4.5:1 for normal text, 3:1 for large text (18pt, or 14pt bold, roughly 24px / 18.5px). The ratio is not rounded: 4.499 fails. Luminance is L = 0.2126R + 0.7152G + 0.0722B on linearised sRGB, where c = c/12.92 if c <= 0.04045, otherwise ((c+0.055)/1.055)^2.4. Contrast = (L1+0.05)/(L2+0.05). The page does not say how to test text over varied backgrounds. Checking the lowest-contrast point is the conservative reading, and is my inference. — [WCAG 2.2 Understanding 1.4.3](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)
- WCAG 1.4.11: UI components and meaningful graphics need 3:1 against adjacent colours. Focus indicators and state indicators too. For gradients or multi-colour areas, test the least contrasting area. Inactive controls and purely decorative graphics are exempt. — [WCAG 2.2 Understanding 1.4.11](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html)
- Technique actually used by each system:
  - Microsoft: exclusion blend, then tint plus luminosity layer with 0.85 to 0.96 luminosity, and a solid fallback when needed. — [Microsoft Learn: Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic), [Haze FluentMaterials.kt](https://raw.githubusercontent.com/chrisbanes/haze/main/haze-materials/src/commonMain/kotlin/dev/chrisbanes/haze/blur/materials/FluentMaterials.kt)
  - Apple: adaptive light/dark glyph flipping, adaptive shadow, a dimming layer for Clear, a solid/frosty fallback via Reduce Transparency, and a Tinted mode with higher opacity. — [WWDC25 219](https://developer.apple.com/videos/play/wwdc2025/219/), [MacRumors](https://www.macrumors.com/2025/10/20/ios-26-1-liquid-glass-toggle/)
  - Haze: tint alpha scaled 0.35 to 0.97 on a fixed 24 dp blur. — [Haze HazeMaterials.kt](https://raw.githubusercontent.com/chrisbanes/haze/main/haze-materials/src/commonMain/kotlin/dev/chrisbanes/haze/blur/materials/HazeMaterials.kt)
  - Microsoft's guidance for text over acrylic: avoid accent-coloured text and hyperlinks. — [Microsoft Learn: Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)

### Derived calculations (derived by me with the WCAG formula above; not from any source)
Method: composite = a*surface + (1-a)*backdrop, assuming the blur has already reduced the backdrop to one flat colour. The worst case is a backdrop equal to the text colour (white behind white text, black behind black text). Surface alpha is the minimum for the contrast target.

| Surface / text / worst backdrop | Min alpha for 3:1 | Min alpha for 4.5:1 | Min alpha for 7:1 |
|---|---|---|---|
| #000 surface, white text, white backdrop | 0.417 | 0.535 | 0.651 |
| #121212 surface, white text, white backdrop | 0.448 | 0.576 | 0.701 |
| #FFF surface, #000 text, black backdrop | 0.350 | 0.456 | 0.584 |
| #FFF surface, #111 text, black backdrop | 0.377 | 0.485 | 0.618 |

Haze alpha values checked against the worst case (derived):
- Black tint over white backdrop, white text: alpha 0.35 gives 2.44:1 (fails), 0.60 gives 5.74:1, 0.73 gives 9.61:1, 0.83 gives 14.09:1, 0.92 gives 18.36:1.
- White tint over black backdrop, black text: alpha 0.35 gives 3.01:1, 0.60 gives 7.37:1, 0.73 gives 10.84:1.
- Haze ultraThin dark (0.55) over white backdrop is just above the 4.5:1 line for pure black tint (needs 0.535). With the #121212 dark container it needs 0.576, so 0.55 fails.

Recommendations (derived from the above; these are my numbers, add a safety margin for tint colour, blur not fully flattening high-frequency backdrops, and text anti-aliasing):
- Body text (4.5:1): minimum surface alpha about 0.60, which is Haze "thin". Use 0.70 or more (Haze "regular", 0.73 / 0.80) for margin.
- Icons and essential glyphs (3:1): minimum surface alpha about 0.45. Use 0.50 to 0.55 for margin.
- Anything below about 0.45 alpha may hold only decoration or non-essential chrome. Do not place text or meaningful icons on it.
- Glyph and text colour: use pure or near-pure contrasting colour (white on a dark tint, near-black on a light tint). Lower-contrast secondary text colours cost headroom, so recompute with the same formula.
- Add a hairline border or edge treatment where the surface is the only cue for a control, because 1.4.11 requires 3:1 against adjacent colours.

### Inferences
- A flat-colour assumption overestimates what blur does. A radius of 24 dp flattens fine detail but not large bright or dark regions, so the worst-case numbers above are right for large-area backdrops and stay a minimum for any backdrop.
- Adaptive designs (Apple) that pick light vs dark based on the backdrop reduce the worst case, but a static design system must assume the worst case, which is why the alpha values above apply.

### Gaps
- I found no published "recommended minimum opacity" from Apple, Microsoft or Google. The thresholds above are my own derivation, and no source states them.
- WCAG gives no explicit procedure for translucent surfaces over arbitrary backdrops.

## 5. Reduced transparency / increased contrast / high contrast fallbacks

### Takeaway
Apple and Windows both swap glass for more opaque or solid surfaces, driven by OS settings. Windows also falls back on battery saver and window deactivation. Stock Android has no documented system "reduce transparency" setting, so the app must detect blur support and supply its own fallback.

### Cited Findings
- Apple: Reduce Transparency makes glass frostier and obscures more of the content. Increase Contrast makes elements predominantly black or white with a contrasting border. Reduce Motion cuts effect intensity and disables elastic behaviour. — [WWDC25 219](https://developer.apple.com/videos/play/wwdc2025/219/)
- Apple 26.1+: user-facing Clear / Tinted toggle (Tinted = more opacity and contrast). — [MacRumors](https://www.macrumors.com/2025/10/20/ios-26-1-liquid-glass-toggle/)
- Windows: solid colour for both acrylic types in High Contrast (user's chosen window colour), when Transparency effects is off, in Battery Saver, and on low-end hardware. Background acrylic alone also goes solid when the window deactivates and on Xbox/HoloLens/tablet mode. — [Microsoft Learn: Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)
- Windows: define separate Default, Light and HighContrast brush entries; HighContrast should be a SolidColorBrush using `SystemColorWindowColor`. — [Microsoft Learn: In-app acrylic](https://learn.microsoft.com/en-us/windows/apps/develop/ui/in-app-acrylic)
- Android: window blur needs API 31+, may be disabled at runtime (battery saver, tunnelling, minimal post-processing, GPU limits), and apps should observe `isCrossWindowBlurEnabled` and adapt. — [AOSP window blurs](https://source.android.com/docs/core/display/window-blurs), [Compose isCrossWindowBlurEnabled](https://composables.com/jetpack-compose/androidx.compose.ui/ui/properties/isCrossWindowBlurEnabled/api)
- Android accessibility: Google's "high contrast text" setting switches text colour to black or white (Display size and text), and Android 16 adds outline text. Samsung's One UI has a "Reduce transparency and blur" option under Visibility enhancements. I found no equivalent entry in Google's stock docs. — [Google Accessibility Help](https://support.google.com/accessibility/android/answer/11183305?hl=en), [Samsung accessibility](https://www.samsung.com/au/business/sustainability/accessibility/vision/)
- Haze: on Android 11 and below, with blur not enabled, Haze draws a translucent scrim (fallback tint, example alpha 0.7) instead of blur. — [Haze blur usage](https://chrisbanes.github.io/haze/latest/blur/usage/)

### Inferences
- A design system should define three states per glass token: blurred glass, scrim-only fallback (no blur, alpha at or above the 4.5:1 minimum), and solid opaque (Reduce Transparency, High Contrast, battery saver). Haze's example 0.7 fallback scrim is above our derived minimum.

### Gaps
- I did not verify whether Android's `Settings.Global`/accessibility API exposes a reduce-transparency flag on stock devices. The search found none. Samsung's setting has no confirmed public app-readable API.
- Apple's UIKit/SwiftUI API names (`isReduceTransparencyEnabled` etc.) appeared only in a tool-generated answer, not on a fetched Apple page, so I leave them out.

## 6. Grain / noise

### Takeaway
Noise is small, about 2% on Windows acrylic, and exists to hide banding and make the surface look like a physical material. Too much looks sandy and hurts text.

### Cited Findings
- Windows acrylic recipe includes a noise layer. Microsoft's own page does not give its opacity. — [Microsoft Learn: Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)
- Haze's acrylic port uses `noiseFactor = 0.02f` (2%). Mica uses 0. Haze's usage docs show 0.15 as an example only. — [Haze FluentMaterials.kt](https://raw.githubusercontent.com/chrisbanes/haze/main/haze-materials/src/commonMain/kotlin/dev/chrisbanes/haze/blur/materials/FluentMaterials.kt), [Haze blur usage](https://chrisbanes.github.io/haze/latest/blur/usage/)
- Why noise (community sources, lower authority): it breaks up banding in smooth blurred gradients (dithering), and a KDE developer argues real brushed glass has grain. Warnings: too much looks like sand, and should be reduced behind interface text. A KDE reviewer questioned its value. — [KDE Phabricator D10281](https://phabricator.kde.org/D10281?id=26677), [Instant Gradient: grainy gradients](https://instantgradient.com/blog/grainy-gradients), [Fragmented commit](https://remotebranch.eu/ChaoticByte/Fragmented/commit/233c7e1)

### Inferences
- Use about 0.02 (2%) noise on glass surfaces that carry text. Anything near 0.15 is decorative and should not sit behind text.
- Noise does not help contrast. It is cosmetic and should be the first thing dropped in reduced-transparency or fallback modes.

### Gaps
- No primary source from Microsoft states the 2% noise figure or explains the reason. The reasoning rests on community and developer commentary.
