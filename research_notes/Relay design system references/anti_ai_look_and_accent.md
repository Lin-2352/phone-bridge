# Anti-"AI look" UI principles, accent colour practice, contrast, icons, reference apps (Relay design checklist inputs)

Research date 2026-11-06. About 17 tool calls. Several primary sources (m3.material.io, Apple HIG, Fluent 2 sub-pages, NN/g, refactoringui.com) are JS-rendered or returned stubs, so some points rest on secondary sources. Those are flagged "secondary". Items from my own background knowledge, not fetched this session, are in "Gaps" or marked "UNFETCHED".

## 1. What makes UI look AI-generated or templated (checkable anti-patterns)

### Takeaway
No peer-reviewed or NN/g source defines an "AI look". The evidence is practitioner consensus: output regresses to the statistical average (Inter/Roboto type, purple-blue gradient, centered hero, identical icon-heading-text cards, emoji icons, glass/glow). The most checkable list is the Impeccable catalog (below), and the best-supported counter-view is that the root problem is incoherence, not any single style. Treat the checklist as heuristics, not measured findings.

### Cited Findings
- Recurring tells named across sources: Inter or Roboto type, purple-to-indigo gradient, centered hero, rows of rounded cards with small icons, emoji as icons, glassmorphism, gradient text, "John Doe" placeholder names — [search synthesis; see sources below](https://dev.to/alanwest/how-to-fix-the-ai-generated-look-in-your-frontend-1ahh), [Kosta Canatselis, Spot the Slop](https://world.hey.com/kostac/spot-the-slop-a-ui-designer-s-guide-to-fixing-ai-defaults-4c448c9c), [superdesign: why AI design looks generic](https://www.superdesign.dev/blog/why-ai-design-looks-generic), [10 signs of AI-generated design (member-only)](https://pub.towardsai.net/10-signs-of-ai-generated-design-and-how-to-make-yours-not-look-like-it-482ff7da81fc). All are blogs; I saw only search snippets, not full pages.
- Impeccable anti-pattern catalog (fetched). Each item is tagged as an automatable "source/browser" check or a "design review" judgement ([impeccable.style/anti-patterns](https://impeccable.style/anti-patterns), [slop page](https://impeccable.style/slop)):
  - Gradient text: use one solid colour.
  - Purple-to-blue default palette: flagged.
  - Radial-gradient glow halo on dark pages: flagged when it competes with content.
  - Repeating-gradient stripes as decoration in empty space: flagged.
  - Side-tab accent border: a thick coloured stripe on one side of a rounded element.
  - Nested cards: any card inside a card.
  - Hairline border plus wide shadow on the same surface: flagged.
  - Identical card grids: equal-weight icon, heading, text repeated.
  - Icon tile (icon in a rounded square) above a heading: flagged.
  - Extreme border radius: flagged when it squeezes content.
  - Glassmorphism (blur, glow): allowed only to solve a layering problem, not as decoration.
  - Monotonous spacing: equal gaps everywhere make groups indistinguishable.
  - More space above a heading than below it.
  - No text touching its container edge or the viewport edge.
  - Contrast: 4.5:1 for text, 3:1 for large text. Line length about 65-75 characters. Line height about 1.5. Body text about 16px.
  - Copy: vague superlatives ("supercharge", "world-class"), repeated "Not X. Y." constructions, em-dash overuse, the same label repeated in one container.
  - The page does not cover centered hero or emoji, so those come from the other blogs above.
- Anthropic frontend-design skill: public, but I could not fetch the canonical copy (404 on the GitHub path guessed). Mirrors say: avoid generic "AI slop" aesthetics, avoid overused fonts (Inter/Roboto/Arial), avoid purple gradients on white, avoid predictable layouts and cookie-cutter components, and pair a distinctive display font with a refined body font. "Emoji as icons" and "Space Grotesk" appear only in some copies and look like community edits — [mcpservers.org mirror](https://mcpservers.org/agent-skills/anthropic/frontend-design), [Medium commentary](https://medium.com/@porter.nicholas/anthropic-skills-marketplace-the-anti-ai-slop-ui-design-skill-a572d0cfef4f), [skills.sh](https://skills.sh/anthropics/claude-code/frontend-design). The wording is secondary.
- Cause: models drift toward the average of the training data, so sameness is statistical. "The output is not bad. It is generic." Counterpoints: a three-column layout can be fine, and "the real culprit ... is Incoherence" — [DEV: why AI UIs look off](https://dev.to/kiwibreaksme/why-ai-generated-uis-look-off-and-the-one-principle-that-fixes-it-4j20), [Vandelay Design](https://www.vandelaydesign.com/?p=61673), [GeekyAnts](https://geekyants.com/blog/why-everything-your-ai-builds-looks-the-same).
- Fixes proposed: lint arbitrary colours and spacing values, add visual regression tests for key components, and treat AI output as a rough draft that needs editorial direction — [pixso](https://pixso.net/articles/how-to-prevent-ai-from-making-all-uis-look-the-same/). An arXiv paper (2607.22928, "Design Theater") warns that clustered generated UIs make conventions look uniquely designed. I only saw the search summary — [arXiv](https://arxiv.org/pdf/2607.22928).

### Inferences
- For a utility app, the cheapest checkable rules are:
  1. No gradients on text or surfaces.
  2. No emoji in chrome.
  3. No card inside a card.
  4. No icon-tile-above-heading.
  5. At most one elevation style per surface (border or shadow, not both).
  6. Spacing values come only from the scale.
  7. Section gaps are visibly larger than item gaps.
  8. No marketing adjectives in copy.
  9. No placeholder names.
  10. Left-aligned content, no centered hero.
- A utility app (tray flyout, settings) has no hero and little marketing surface, so many "tells" are avoided just by using native-feeling lists, rows and toggles instead of cards.

### Gaps
- No NN/g article on AI-generated look found. NN/g searches returned nothing relevant.
- Canonical Anthropic frontend-design text not retrieved.
- No quantified or empirical study of how users perceive the "AI look". All sources are opinion.

## 2. Positive principles: official guideline plus shipping app

### Takeaway
Each principle has an official anchor. The shipping-app examples are my selection, unverified in this session (UNFETCHED), and should be eyeballed by the writer or the user.

### Cited Findings
- 4/8 grid. Material: all components align to an 8dp baseline grid. Icons, type and some in-component elements align to 4dp. Default touch target 48x48dp with at least 8dp between targets. iOS recommends 44x44 — [material.io spacing methods](https://material.io/design/layout/spacing-methods.html), [Material baseline grids](https://m1.material.io:443/layout/metrics-keylines.html), [Google touch target size](https://support.google.com/accessibility/android/answer/7101858). Refactoring UI (secondary): use a fixed scale such as 4/8/16/24/32, not arbitrary values like 13px — [skill summary](https://github.com/s0xDk/refactoring-ui-skill/wiki), [book review](https://dev.to/updivision/book-review-refactoring-ui-by-adam-wathan-steve-schoger-2f7m). The Windows layout-metrics page returned 404, so the 4 epx claim for WinUI is not verified here.
- Hierarchy from size and weight, not boxes. Refactoring UI (secondary): don't rely on size alone. Use weight and colour for emphasis and de-emphasise with a lighter colour. Use two weights (400/500 and 600/700). Build in grayscale first. Use a small fixed set of shadow levels and defer shadows until layout works — [wondelai summary](https://playbooks.com/skills/wondelai/skills/refactoring-ui), [s0xDk wiki](https://github.com/s0xDk/refactoring-ui-skill/wiki). The primary book text was not fetched.
- Accent used sparingly. Fluent 2: "Avoid overusing brand colors or using them on large surfaces as they can dilute a hierarchy." Brand colour is for buttons, CTAs and selected states — [Fluent 2 color](https://fluent2.microsoft.design/color).
- Motion only with meaning, real copy and UX writing guidelines: not fetched. See Gaps.
- Shipping apps for these principles, UNFETCHED, from background knowledge:
  - Windows 11 Settings and Quick Settings: one system accent plus neutral rows.
  - Linear (hierarchy by weight and neutral tones).
  - Android Settings / Pixel Settings (list rows, 8dp grid).
  - Tailscale client (utility tray app).

### Inferences
- A rule "all spacing and size tokens are multiples of 4, layout rhythm in 8" is directly supported by Material and by Refactoring UI-style scales.

### Gaps
- Official Microsoft, Google and Apple UX writing guideline pages were not fetched (budget). Known entry points, unverified: learn.microsoft.com/style-guide, m3.material.io/foundations/content-design, developer.apple.com/design/human-interface-guidelines/writing.
- Official motion guidance (Fluent motion, M3 motion, HIG motion) not fetched.
- The refactoringui.com primary text was not fetched. All Refactoring UI statements are secondary.
- Shipping-app examples above are not verified.

## 3. One accent plus semantic status colours; where amber belongs

### Takeaway
All three systems separate a brand/accent role (interactive and selected) from semantic status roles (danger, caution, success), and tell you to pair status colour with text or icon. Fluent 2 and Apple say status colours are for status, not decoration. Recommendation: if amber means "attention needed / caution", make it a warning role, not a second accent. Use one accent only.

### Cited Findings
- Fluent 2: brand colour gives identity and emphasis, and is used on buttons, CTAs and selected states. Select colours in the shared palette "are specifically used to communicate feedback, status, or urgency". Red is danger, yellow is caution, green is positive. "Don't use them for decoration." Pair with text or graphics so meaning does not rely on colour alone — [Fluent 2 color](https://fluent2.microsoft.design/color). The page does not name success/warning/danger tokens explicitly.
- Apple HIG (the fetch returned only a generic summary, not page text, so treat as secondary): a single accent tint for interactive elements, users may change it system-wide, so UI must work with any accent. Red marks destructive actions and errors. Don't convey information by colour alone — [HIG Color](https://developer.apple.com/design/human-interface-guidelines/color). Unverified quotes.
- Material 3: the colour-roles page (m3.material.io/styles/color/roles) was not readable. Per search, M3 defines error roles with container variants, and has no official success/warning roles. Apps add "extended colors", which Theme Builder can harmonize toward the primary hue. Some M3-based design systems define success/warning/error roles each with on-colour and container variants, and require WCAG AA — [M3 Theme Builder](https://www.Github.com/material-foundation/material-theme-builder), [Android harmonization](https://developer.android.com/reference/com/google/android/material/color/HarmonizedColorsOptions), [Material Color Utilities issue on harmonizing a success colour](https://github.yxlr123.workers.dev/material-foundation/material-color-utilities/issues/71). Secondary.

### Inferences
- A second amber as an "accent" would break the one-accent rule and also collide with the universal caution association (Fluent: yellow = caution). Make amber a warning role with its own on-colour, container and icon, and never use it for selection, links or primary buttons.
- If Relay needs an amber brand moment, keep it in the app icon or logo only.
- Define roles: accent (one), danger, warning, success, neutral ramp. Each status role needs a text or icon companion.

### Gaps
- No primary M3 text confirming absence of success/warning roles; "no official role" is inferred from search results.
- Apple HIG exact quotes unavailable.

## 4. WCAG 2.2 SC 1.4.11 Non-text Contrast and divider vs control boundary

### Takeaway
1.4.11 requires 3:1 for visual information needed to identify a UI component or understand a graphic, so an input border needs 3:1 only when it is the thing that identifies the field. Decorative dividers are not covered. Material encodes this split as `outline` (control boundaries) vs `outline-variant` (dividers).

### Cited Findings
- W3C Understanding 1.4.11 (fetched): visual info required to identify a control must reach 3:1 against adjacent colours. A text input whose only cue is a border needs that border at 3:1 (passing examples use #767676). A light grey #AAA border on an unlabeled input fails. A control with other identifying content (text, a sufficiently contrasting icon) does not need a boundary. A background-only input must meet 3:1 against its surroundings. A focus indicator is still required. Outlining all controls is encouraged, not required — [Understanding 1.4.11](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html).
- Exceptions (same page): inactive/disabled components; user-agent-determined appearance without author modification (does not cover author-styled focus indicators); essential presentation of graphical objects; logotypes. Low contrast as an author choice is not exempt.
- Material 3: `outline` for borders needing 3:1 (for example a text field border). `outline-variant` for dividers and decorative boundaries where 3:1 is not required. An Angular Components PR moved non-text elements from outline-variant to outline to pass 3:1 — [Android ColorTokens](https://developer.android.com/reference/androidx/wear/protolayout/material3/ColorTokens), [Compose ColorScheme](https://kotlinlang.org/api/compose-multiplatform/material3/androidx.compose.material3/-color-scheme/), [SAP Fiori tonal](https://www.sap.com/design-system/fiori-design-android/v25-11/foundations/colors/tonal-option), [Angular PR 28995](https://fr.github.com/angular/components/pull/28995), [Firefox Android colours](https://acorn.firefox.com/latest/mobile/styles/color/android-ZJOEoOpd). The official M3 page itself was not read, so this is secondary.
- Fluent 2: the colour page gives no stroke or divider guidance beyond the focus container stroke thickening — [Fluent 2 color](https://fluent2.microsoft.design/color).

### Inferences
- Rule for Relay: any input, checkbox or outlined-button boundary is at least 3:1 against its adjacent surface (in both themes). Hairline dividers and card edges may be lower, as long as nothing relies on them to identify a control. If a filled input differs from the page only by a faint fill, it fails; add a 3:1 border or a contrasting bottom line.
- Disabled controls are exempt, but keep them legible enough to read.

### Gaps
- Fluent 2 control-stroke token values (for example its neutral stroke accessible token) not retrieved.
- The WCAG normative text was not read, only the Understanding page.

## 5. Icon sets: licence and sizes

### Takeaway
Both are safe to bundle: Fluent System Icons is MIT, Material Symbols is Apache-2.0. Use Fluent at 16/20/24 (Regular, Filled for selected) on the PC side, and Material Symbols at 24 on Android.

### Cited Findings
- Fluent System Icons repo is MIT-licensed, has regular, filled, light and color styles, ships plain SVG plus Android/iOS/Flutter packages, and per-icon `metadata.json` lists the available sizes. Directional icons use `_ltr`/`_rtl` — [microsoft/fluentui-system-icons](https://github.com/microsoft/fluentui-system-icons). The page did not list sizes, and the LICENSE text was not shown (only the badge).
- Material Symbols are Apache License 2.0. Variable axes: FILL 0-1, wght 100-700, GRAD about -25 to 200 (the page's example range was -50 to 200), opsz 20-48 dp. 24px is the "preferred icon size". The page is inconsistent on the default opsz (48 vs 24) — [Google Fonts Material Symbols guide](https://developers.google.com/fonts/docs/material_symbols).
- Windows system tray, context menu and title bar icons: 16 px at 100% scale, then 20/24/32 at 125/150/200 percent. App icons should ship at least 16, 24, 32, 48 and 256, with transparent backgrounds and light/dark variants — [Microsoft: app icon construction](https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-construction).

### Inferences
- Recommended defaults (my recommendation): one icon family per platform, one weight. Material Symbols Outlined, wght 400, opsz 24, FILL 0 (FILL 1 for selected nav items), size 24 with 48dp touch targets. Fluent Regular at 16 in dense rows, 20 in buttons and nav, 24 in headers, with Filled variants for selected state.
- Using two families (Fluent on PC, Material on Android) is consistent with each platform's native feel, but it needs a mapping table of concepts to avoid mismatched metaphors.

### Gaps
- Fluent icon size values are in `metadata.json`; I did not read it. Known sizes (UNFETCHED) include 16/20/24/28/32/48.
- Fluent LICENSE file not opened to confirm the text (the repo badge says MIT).

## 6. Reference apps per screen family

### Takeaway
I could not fetch and verify app pages in the budget. These are UNFETCHED picks from background knowledge, with official URLs where I'm confident. Treat each as a candidate to screenshot-check before citing in the final report.

### Cited Findings
None verified this session. Candidates, UNFETCHED:
- PC tray flyout
  - Windows 11 Quick Settings flyout (https://learn.microsoft.com/en-us/windows/apps/design/): a compact grid of toggles plus sliders in a single surface. Emulate the density.
  - Tailscale Windows tray menu (https://tailscale.com/): status line first, then actions. Emulate the status-first layout.
  - Dropbox / OneDrive tray flyout (https://www.microsoft.com/microsoft-365/onedrive/online-cloud-storage): a state summary with one primary action. Emulate it for transfer state.
- PC main window with navigation
  - Windows 11 Settings (WinUI NavigationView; https://learn.microsoft.com/en-us/windows/apps/design/controls/navigationview): left nav, page title, grouped setting rows. Emulate the row pattern.
  - Microsoft PowerToys settings (https://github.com/microsoft/PowerToys): a WinUI nav app with a utility focus.
  - Phone Link (https://www.microsoft.com/windows/sync-across-your-devices): the direct competitor, so study what to differ from.
- Android home
  - Google Files (https://play.google.com/store/apps/details?id=com.google.android.apps.nbu.files): a plain list-first home with a storage summary.
  - Pixel Quick Settings / Google Home (https://home.google.com/): status tiles.
  - KDE Connect Android (https://kdeconnect.kde.org/): the closest functional analogue, with a device list.
- Android settings
  - Pixel Settings (AOSP Settings, https://source.android.com/): grouped rows, switch on the right, summary text under title.
  - Material 3 Settings pattern (https://m3.material.io/components/lists/overview): list items with supporting text.
  - Signal settings (https://signal.org/android/apps/): plain, restrained.
- Consent dialogs and sheets
  - Android runtime permission dialog (https://developer.android.com/training/permissions/requesting): states what, offers allow and deny, with no decoration.
  - Windows UAC prompt (https://learn.microsoft.com/en-us/windows/security/application-security/application-control/user-account-control/): names the requester and keeps two buttons.
  - Apple permission sheets (https://developer.apple.com/design/human-interface-guidelines/privacy): purpose string wording as a model.

### Inferences
- The shared pattern across these is a left-aligned list of rows with title plus supporting text, a single control on the right, and status stated in words. Combined with section 1, this avoids card grids.

### Gaps
- None of the URLs in section 6 were opened. The URLs are best-effort; verify each before it goes in the final report.
- No dedicated search for tray-flyout design guidance was done.
- Anything marked UNFETCHED across this file is unverified.
