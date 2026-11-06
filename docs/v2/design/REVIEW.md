# Relay design prototypes: fresh review (M4)

Reviewer: a fresh agent with no stake in the work.

Inputs:
- First pass: the 16 captures now in `review/first-pass/`, `PRINCIPLES.md`, `RESEARCH.md`, SPEC.md M4 (lines 228-252), `design/floot/base.css`, and the Floot source of `pages/_index`, `pages/pc`, `pages/android`, `pages/motion`, `components/DesignKit`, plus the `Button`, `Badge`, `Switch`, `Progress` and `Input` stylesheets.
- Re-check: the 7 post-fix captures in `review/`, the regenerated `design/floot/base.css`, and the new tests in `tools/gen-tokens/src/lib.rs`. The Floot project couldn't be re-read: the account's daily build limit was reached and resets 2026-11-07 12:00 UTC. A fix claimed only in Floot source, with nothing in a capture to show it, is marked **Open (unverified)**.

Method: contrast ratios are WCAG 2.x relative-luminance ratios computed from hex values. Alpha composites are `tint × alpha + backdrop × (1 − alpha)`. Pixel colours come from sampling the JPEGs.

Each finding ends with a status line: **Fixed**, **Open** or **Deferred (minor)**.

## 1. Summary

The prototypes avoid the AI look: no gradients, no emoji, real copy, one cobalt accent and hierarchy from size and weight. The first pass found 3 Critical, 8 Important and 10 Minor findings.
The re-check confirms 6 fixes, including the two pixel-checkable Critical ones. Glass now renders at its token alpha (R-1), and off switches have a visible ring (R-2). R-5, R-6, R-9 and R-21 are also confirmed.
Still open: focus rings (R-3), the Android live tile (R-7), the PC Live badge (R-8) and the reduced-motion easing demo (R-10). All four are claimed fixed in Floot, but nothing could confirm them.
Also open: in light mode the Relay look is still barely different from Native (R-4), Android still uses Windows radii and type (R-11), and one new Minor finding (R-22).
Open now: 1 Critical, 5 Important, 1 Minor. Nine Minor findings are deferred.

## 2. Score table (first pass; the re-check rows are in section 6)

1 = fails, 5 = excellent. Criteria: (1) no purple-blue gradients, (2) no emoji icons, (3) not one repeated rounded card, (4) real copy, (5) 4/8-pt grid and optical alignment, (6) one accent, (7) hierarchy from size and weight, (8) meaningful motion, (9) AA contrast, (10) status not by colour alone, (11) glass readability and look and mode difference. n/a means the criterion has nothing in frame (see section 5). For static pages, (8) is judged from the component transitions in the source.

| Capture | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| pc-native-light | 5 | 5 | 4 | 4 | 4 | 3 | 4 | 4 | 2 | 3 | 2 |
| pc-native-dark | 5 | 5 | 4 | 4 | 4 | 3 | 4 | 4 | 3 | 3 | 2 |
| pc-relay-light | 5 | 5 | 4 | 4 | 4 | 3 | 4 | 4 | 2 | 3 | 2 |
| pc-relay-dark | 5 | 5 | 4 | 4 | 4 | 3 | 4 | 4 | 3 | 3 | 3 |
| kit-native-light | 5 | 5 | 4 | 5 | 4 | 4 | 4 | 4 | 2 | 3 | 2 |
| kit-native-dark | 5 | 5 | 4 | 5 | 4 | 4 | 4 | 4 | 3 | 3 | 2 |
| kit-relay-light | 5 | 5 | 4 | 5 | 4 | 4 | 4 | 4 | 2 | 3 | 1 |
| kit-relay-dark | 5 | 5 | 4 | 5 | 4 | 4 | 4 | 4 | 3 | 3 | 3 |
| kit-relay-light-rt | 5 | 5 | 4 | 5 | 4 | 4 | 4 | 4 | 2 | 3 | 1 |
| kit-relay-dark-rt | 5 | 5 | 4 | 5 | 4 | 4 | 4 | 4 | 3 | 3 | 1 |
| android-native-light | 5 | 5 | 3 | 4 | 4 | 3 | 4 | 4 | 2 | 2 | n/a |
| android-native-dark | 5 | 5 | 3 | 4 | 4 | 3 | 4 | 4 | 3 | 2 | n/a |
| android-relay-light | 5 | 5 | 3 | 4 | 4 | 3 | 4 | 4 | 2 | 2 | n/a |
| android-relay-dark | 5 | 5 | 3 | 4 | 4 | 3 | 4 | 4 | 3 | 2 | n/a |
| motion-native-light | 5 | 5 | 2 | 5 | 4 | 5 | 4 | 3 | 3 | n/a | n/a |
| motion-relay-dark | 5 | 5 | 2 | 5 | 4 | 5 | 4 | 3 | 3 | n/a | n/a |

Why the low scores: (9) light pages fail on the badge text (R-5), the off switches (R-2) and the focus rings (R-3); dark pages fail only on the switches and focus rings. (10) R-6, R-7 and R-8. (11) R-1 and R-4: the RT captures score 1 because they are pixel-identical to the normal captures. (3) The Android 2×2 quick tiles and the motion page's seven identical cards. (6) Cobalt marks degraded and live states (R-6, R-7).

## 3. Findings

### R-1 [Critical] The glass renders opaque in every capture, so no capture tests glass and reduced transparency changes nothing
- **Page / file:** Kit and PC; `components/DesignKit.module.css` `.glass`; `base.css` glass tokens and the `prefers-reduced-transparency` block.
- **Evidence:** I sampled the glass sample of the Kit page at (170,333), (190,383), (250,333), (300,383) and (230,360). Two of those points sit over the white half and two over the black half.
  - kit-native-light: every point is `#F3F3F3`. The tokens (`--glass-tint #F3F3F3`, `--glass-opacity 0.92`) predict `#F4F4F4` over the white half and `#E0E0E0` over the black half (0.92 × 243 = 224).
  - kit-relay-light and kit-relay-light-rt: every point is `#F8F9FE` in both captures. The tokens predict `#E4E5EB` over the black half.
  - kit-native-dark: every point is `#1F1F1F`. The tokens predict `#484848` over the white half (0.82 × 32 + 0.18 × 255).
  - kit-relay-dark and kit-relay-dark-rt: both are `#14172A` ± 2. The tokens predict `#303344` over the white half.
  - The PC tray flyout looks the same in pc-*.jpg: an even fill across the white/black split.
- **Defect or intended?** Partly each. The near-opaque glass is intended: RESEARCH chooses 0.70 or more, and the tokens use 0.82 to 0.92. Even so, at 0.92 the black half should be visibly darker than the white half, and reduced transparency should lighten it to the solid tint (luminance 0.785 → 0.949 for Relay light). The captures show no difference at all, so the glass is not rendering at the token alpha. That is a defect.
- **Likely cause (unconfirmed):** the base.css block `@media (prefers-reduced-transparency: reduce), (forced-colors: active)` sets `--glass-opacity: 1 !important`. If the capturing Chrome inherits Windows "Transparency effects: off" (or a battery-saver state), that block solids every capture, whatever the page's switch shows. The other possibility is that `color-mix(... calc(var(--glass-opacity) * 100%), transparent)` resolves to an opaque colour in the capture browser.
- **Principle:** PRINCIPLES "Glass and modes" (three states, each tested); SPEC M4 "Playwright screenshots, scored". RESEARCH's conclusion names glass as the main design risk.
- **Fix:**
  - In the screenshot script, force `prefers-reduced-transparency: no-preference` through CDP `Emulation.setEmulatedMedia`. Turn reduced transparency on only through `data-reduce-transparency`.
  - On the Kit page, print the computed `--glass-opacity` and `--glass-blur` under the glass sample (`pages/_index.tsx`), so each capture records the mode it shows.
  - Recapture and re-score criterion 11.
- **Status: Fixed.**
  - The cause is confirmed: the capture browser reported `prefers-reduced-transparency: reduce`.
  - `base.css` now scopes the media block with `:not([data-force-glass="true"])`, and the explicit `data-reduce-transparency` rule is unchanged, so the in-page switch still wins. The test `forced_glass_opts_out_of_the_os_preference` asserts both.
  - Pixels in the new captures match the token composites within JPEG error (sampled at y = 335 or 385 and across the flyout footer):

    | Capture | Over white | Over black | Predicted |
    |---|---|---|---|
    | kit-native-light | `#F3F3F3` | `#DEDEDE`–`#E2E2E2` | `#F4F4F4` / `#E0E0E0` |
    | kit-relay-light | `#F8F9FE` | `#E1E4ED`–`#E3E3EB` | `#F9F9FF` / `#E4E5EB` |
    | kit-native-dark | `#484848` | `#1A1A1A` | `#484848` / `#1A1A1A` |
    | pc-relay-dark flyout | `#313445` | `#14172A` | `#303344` / `#121425` |

  - A blurred gradient now crosses the split. For example, kit-native-light at y = 335 runs `#F2F2F2` → `#EAEAEA` → `#E2E2E2` between x = 150 and 240.
  - Highlight is visible only in Relay dark: a 1 px top edge `#292B3A` over `#151628` (pc-relay-dark, x = 300, y = 257). Native dark has none.
  - Grain is detectable only in dark. The flat-glass pixel standard deviation is 0.78 in pc-relay-dark against 0.00 in pc-native-dark. In light it is at JPEG noise level (relay 0.45–0.53, native 0.00–0.61).
  - Not re-captured: reduced transparency, now that glass is forced on (`kit-relay-*-rt`). The source path is correct, but no capture shows it.

### R-2 [Critical] Off switches have no visible boundary: 1.22:1 in light, 1.31:1 in dark
- **Page / file:** every page; `components/Switch.module.css` (track `--muted` = `--color-outline`; thumb `--card`).
- **Evidence:**
  - Light: the track `#E5E5E5` is 1.22:1 on the card `#FBFBFB` and 1.14:1 on the surface `#F3F3F3`. The thumb `#FBFBFB` on that track is also 1.22:1.
  - Dark: the track `#3F3F3F` is 1.31:1 on `#2D2D2D`. The thumb `#2D2D2D` on the track is 1.31:1.
  - Visible in the four header toggles of every capture (pc-native-light: "Relay glass", "Dark" and the others look like faint pills).
  - Below the fold (source only): `pages/pc.tsx` lines 116 and 120 ("Ask for Windows Hello first", "Show shared text in notifications" are off), and `pages/android.tsx` lines 105-106 (Lite mode, Reduce transparency).
  - Arithmetic (light): L(#E5E5E5) = 0.7835 and L(#FBFBFB) = 0.9647, so (0.9647 + 0.05) / (0.7835 + 0.05) = 1.22.
- **Principle:** WCAG 1.4.11 and PRINCIPLES "Colour": an unlabeled toggle's boundary must reach 3:1. A settings page made of off switches shows rows with no visible control.
- **Fix:** follow the Windows 11 pattern for the off state: a 1 px `--color-outline-strong` ring on the track and the thumb in `--color-text-muted`. `#767676` gives 4.39:1 on `#FBFBFB` and 3.61:1 on `#E5E5E5`. In dark, `#8A8A8A` gives 3.99:1 on `#2D2D2D`. Make the change in `components/Switch.module.css`.
- **Status: Fixed.**
  - In kit-relay-light, the off header toggles now show a dark ring and a grey thumb. Sampled ring and thumb pixels are `#585858`–`#676767` against `#F2F2F2`, where the first pass showed `#E5E5E5`.
  - Arithmetic: the token ring `#767676` is 4.09:1 on `#F3F3F3`, and a `#5F5F5F` thumb is 5.75:1. In dark, `#8A8A8A` is 4.72:1 on `#202020` and the `#C5C5C5` thumb is 9.44:1.
  - The off switches below the fold (pc.tsx Settings, android Settings) use the same component, but no capture shows them.

### R-3 [Critical] Keyboard focus indicators are about 1.2:1
- **Page / file:** all pages; `components/Button.module.css` `.button:focus-visible`, `components/Switch.module.css` `:focus-visible`, `components/Input.module.css` `:focus`.
- **Evidence:**
  - Button and Switch draw a 4 px halo of `color-mix(foreground 9%, transparent)`. In light that gives `#E7E7E7` on `#FBFBFB`, 1.20:1. In dark it gives `#404040` on `#2D2D2D`, 1.33:1.
  - Input focus is a border at 55% primary. In light that is `#8F9EDD` on `#FBFBFB`, 2.50:1.
  - `base.css` already defines `--shadow-focus: 0 0 0 2px var(--color-primary)` (6.46:1 light, 8.07:1 dark), and no component uses it.
- **Principle:** PRINCIPLES "Focus indicators are always 3:1"; WCAG 2.4.7. A tray app is used from the keyboard (Win+B, arrows), so this is the main way some people get around.
- **Fix:** use `box-shadow: var(--shadow-focus)` with a 2 px offset ring in all three components. Set the Input focus border to `--color-primary` at 100%.
- **Status: Open (unverified).**
  - The claimed fix is a 2 px `--color-primary` ring with a 2 px gap, which would give 6.46:1 in light and 8.07:1 in dark.
  - No capture shows a focused element, and the Floot source couldn't be read (build limit), so the claim can't be confirmed.
  - To close: one capture with keyboard focus on a Button, a Switch and the Input, in both themes.

### R-4 [Important] In light mode the "Relay glass" look can't be told apart from Native
- **Page / file:** all; `base.css` `[data-look="relay"]`.
- **Evidence:**
  - kit-native-light and kit-relay-light differ only in the glass sample (`#F3F3F3` vs `#F8F9FE`, ΔE barely visible) and the active header toggle.
  - The highlight `rgba(255,255,255,0.1)` on a near-white tint and grain 0.02 add no visible detail.
  - android-native-light and android-relay-light are effectively identical.
  - In dark, the navy tint `#14172A` (kit-relay-dark, pc-relay-dark flyout) is visibly different from `#202020`.
  - `pages/android.tsx` line 104 offers "Look: Relay glass" as a user setting.
- **Principle:** a user-facing switch that changes nothing visible in light mode is a "UI shows X, reality is Y" problem. SPEC M4 asks for "both themes" (looks) to be prototyped.
- **Fix:** either give the Relay light look a perceptible identity on glass surfaces only (for example a cooler tint such as `#EEF1FF`, which keeps text above 13:1, or a 1 px `--color-primary` 12% top edge), or drop the light variant and describe the look as dark-only. Decide this in `design/tokens.toml` and regenerate `base.css`.
- **Status: Open.**
  - Nothing in the Relay light look changed: `base.css` still has tint `#F8F9FF`, alpha 0.92, highlight 0.1 and grain 0.02.
  - Now that glass renders, Relay light is distinguishable from Native only side by side, and only on the glass. Over the black half the flyout is `#E1E4EB` against `#DEDEDE` (a slight blue cast); over the white half it is `#F7F8FD` against `#F3F3F3`.
  - Highlight and grain are not visible in light (see R-1).
  - Every opaque surface is identical between pc-native-light and pc-relay-light, and between kit-native-light and kit-relay-light.
  - In dark the looks are clearly different (navy tint plus a visible highlight).

### R-5 [Important] Light-theme status badges are below 4.5:1
- **Page / file:** Kit Status, PC flyout and device picker, Android device picker; `components/Badge.module.css` (`.success`, `.warning` and `.destructive` use a 15% tint on `--surface`).
- **Evidence:** the badge text is 14 px at weight 500, so it is normal text and needs 4.5:1.
  - `#0F7B0F` on `#D8E8D8` = 4.26:1
  - `#9D5D00` on `#EDE3D5` = 4.14:1
  - `#C42B1C` on `#F3DCDA` = 4.33:1
  - Seen in kit-*-light ("Connected", "Reconnecting…", "Couldn't reach the phone"), the pc-*-light flyout and the android-*-light picker.
  - Dark passes: 5.03, 7.03 and 5.00.
- **Principle:** PRINCIPLES contrast floor; WCAG 1.4.3.
- **Fix:** add `on-*-container` text tokens: success `#0B6A0B` (5.22:1 on the 15% tint), warning `#8A5100` (5.02), danger `#A82316` (5.42). Alternatively, drop the tint to 8% (4.71, 4.55, 4.83), which leaves almost no margin. Set the values in `design/tokens.toml`, then update `Badge.module.css`.
- **Status: Fixed.**
  - `base.css` now has `--color-success-text #0B6A0B`, `--color-warning-text #8A5100` and `--color-danger-text #A82316` (dark keeps the old values, which already passed).
  - On the 15% tint the light ratios are 5.34, 5.08 and 5.51 over `#FBFBFB`, and 5.00, 4.76 and 5.15 over `#F3F3F3`. All pass.
  - The test `status_text_is_aa_on_its_tint` asserts this for both surfaces and both themes.
  - The capture agrees: the darkest text pixel in kit-relay-light is darker than in the first pass for all three badges (`#285828` vs `#366636`, `#6C502B` vs `#785A34`, `#70302E` vs `#803A30`).

### R-6 [Important] The PC tray audio tile loses its name and shows a degraded state in the accent
- **Page / file:** PC tray flyout; `pages/pc.tsx` line 56, `pages/pc.module.css` `.tileOn`.
- **Evidence:**
  - In all four pc-*.jpg captures the third tile reads only "Reconnecting…" next to a speaker icon, on a solid cobalt fill.
  - `<Tile icon={<Volume2/>} label="Reconnecting…" on />` with `aria-pressed`, so Narrator announces "Reconnecting…, toggle button, pressed" and never says "audio".
  - The caption promises "Tiles … with a state word", but the other three tiles have no state word.
  - Cobalt marks "connected and selected". RESEARCH and PRINCIPLES give degraded links to amber attention.
- **Principle:** checklist 4 and the Colour ruling (one accent; amber for degraded); checklist 2 (say what happens); checklist 12 (Narrator label).
- **Fix:** each tile gets its name plus a second-line state ("Phone audio / Reconnecting…"). The on state uses cobalt. A degraded state keeps the neutral tile and adds an amber icon and words, using `--warning` per R-9.
- **Status: Fixed.**
  - In all four new pc-*.jpg captures, every tile has a name plus a state line: "Phone screen / Off", "Camera / Off", "Phone audio / Reconnecting…". The audio tile is neutral, with an amber speaker icon and amber words.
  - Cobalt no longer marks the degraded state.
  - "Send a file" has no state line. That is correct, because it is an action, not a feature with a state.
  - Narrator labels were not checked (source unreadable).

### R-7 [Important] The Android "Phone audio" quick action shows "live" by colour alone
- **Page / file:** Android Home; `pages/android.tsx` line 56, `pages/android.module.css` `.quickOn`.
- **Evidence:**
  - In all four android-*.jpg captures, "Phone audio" is a filled cobalt tile with no state text.
  - The frame caption says "a live share shows in the accent with its state word", and there is no state word.
  - PRINCIPLES lists "sharing live" notices under amber attention, not cobalt.
- **Principle:** checklist 11 (status in words plus an icon, never colour alone); the Colour ruling.
- **Fix:** add a second line "Live on DESKTOP-4K2" with an icon, and announce it in the TalkBack state description. Settle in PRINCIPLES whether "live" is accent (selected) or attention. The two pages currently disagree.
- **Status: Open (unverified).**
  - The claimed fix is a "Live on DESKTOP-4K2" line.
  - The android-* captures were not retaken (build limit), and the source couldn't be read.
  - The accent-versus-attention question for "live" is still unanswered in PRINCIPLES.

### R-8 [Important] Amber "Live" badge without an icon, and status badges without the icons the kit promises
- **Page / file:** PC "Phone screen window" (below the fold); `pages/pc.tsx` line 153; `components/Badge`. Also Kit Status.
- **Evidence:**
  - `<Badge variant="warning">Live</Badge>` has no icon, which breaks the amber rule "always with an icon and words".
  - The Kit Status caption (kit-*.jpg) says "each status has an icon and words", but the four chips "Connected", "Reconnecting…", "Couldn't reach the phone" and "Paused" carry words only.
  - The words keep this from being colour alone. The kit, though, contradicts its own caption.
- **Principle:** PRINCIPLES Colour (amber always with an icon and words); checklist 11.
- **Fix:** add an optional leading `icon` prop to `Badge` and require it for `warning` and `destructive`. Use it in `_index.tsx`, `pc.tsx` and `android.tsx`.
- **Status: Open (partly fixed).**
  - Kit status badges now carry icons, confirmed in kit-*-light and kit-native-dark: a check, a refresh, wifi-off and pause. The PC flyout badge "Connected" has a wifi icon.
  - The amber "Live" badge in the Phone screen window is below the fold, and the source couldn't be read. Its icon is therefore unverified, and that was the rule break in this finding.

### R-9 [Important] The amber token chosen as "attention" fails the floors, and the prototypes use a different colour
- **Page / file:** `base.css` `--color-accent`; `components/DesignKit.module.css` `.attention`.
- **Evidence:**
  - PRINCIPLES plans to rename `accent` (`#C9820F`) to `attention`.
  - `#C9820F` gives 3.14:1 on white, 3.04:1 on `#FBFBFB` and 2.83:1 on `#F3F3F3`. That fails 4.5:1 for text everywhere, and fails 3:1 for icons on the surface.
  - The prototypes draw attention with `--warning` instead: light `#9D5D00` (brown, 5.07:1 on the card), dark `#FCE100` (yellow, not amber).
  - In kit-native-dark and pc-*-dark the attention banners are yellow, not amber.
- **Principle:** PRINCIPLES contrast floors and Colour ruling (one meaning, one colour).
- **Fix:** before the M5 rename, give `attention` the values the prototypes already prove (`#9D5D00` light, plus a dark amber that passes, such as `#E8A93A` at 6.67:1 on `#2D2D2D`). Retire `#C9820F` from any text or icon use, and have the gen-tokens WCAG tests cover the attention role.
- **Status: Fixed.**
  - `--color-accent` (attention) is now `#955700` in light: 5.20:1 on `#F3F3F3`, 5.57:1 on `#FBFBFB` and 5.77:1 on white.
  - Dark `#E8A93A` is 7.89:1 and 6.67:1.
  - The test `attention_is_readable_as_text_and_icon` asserts at least 4.5:1 on both surfaces in both themes.
  - Remaining colour inconsistency: R-22.

### R-10 [Important] Reduced motion still moves the easing demos
- **Page / file:** Motion; `pages/motion.tsx` lines 104-112.
- **Evidence:**
  - `EasingDemo` always animates `transform: translateX(220px)` with `transition: transform var(--motion-emphasised-ms)`.
  - Reduced motion only shortens that duration to 120 ms (`base.css` `:root[data-reduce-motion="true"]`), so the dot still travels.
  - The page copy (motion-*.jpg lede) says "every Replay becomes a short cross-fade, with no travel".
  - The spring demos do this correctly (lines 59-64).
- **Principle:** PRINCIPLES "Reduced motion replaces travel and springs with a 100 to 150 ms cross-fade".
- **Fix:** in `EasingDemo`, branch on `reduced()` the way `SpringDemo` does. Also check component transitions: the `Switch` thumb slides with a hard-coded `cubic-bezier(0.16,1,0.3,1)` in all modes.
- **Status: Open (unverified).**
  - Two claims: `EasingDemo` now honours reduced motion, and the Switch thumb transition is off under reduced motion.
  - The motion-* captures were not retaken, and a still capture couldn't show either change anyway. The source couldn't be read.

### R-11 [Important] Android prototypes use Windows radii and type, and the token scale lacks the Android values
- **Page / file:** Android; `pages/android.module.css`; `base.css` radius scale.
- **Evidence:**
  - PRINCIPLES says Android uses 16 for cards, 28 for sheets and full for buttons. The token scale stops at 4, 8, 12 and 20.
  - Quick tiles use `--radius-3` (12 px), the consent sheet uses `--radius-4` (20 px), and buttons use `--radius` (8 px). Compare "Resume" and "Pause" in android-*.jpg Transfers with Material's full-round buttons.
  - The type is Segoe UI Variable (`--font-family-base`) at Windows sizes (title 20, not M3 Title L 22/28).
- **Principle:** PRINCIPLES "Shape, space, type"; RESEARCH "shape … straight from each platform".
- **Fix:** add per-platform radius tokens (`radius-card-android: 16`, `radius-sheet-android: 28`) and an Android type ramp to `design/tokens.toml`. Scope them under a `[data-platform="android"]` wrapper in the `Phone` component (`pages/android.tsx`), with `Button shape="pill"` inside it.
- **Status: Open.** No fix was claimed. `base.css` still has no 16 or 28 radius and no Android type ramp.

### R-12 [Minor] Transfers miss "to whom", and a size wraps mid-figure
- **Page / file:** Android Transfers; `pages/android.tsx` lines 74 and 76. PC queue, `pages/pc.tsx` line 214.
- **Evidence:**
  - The caption says "What, to whom, how far". The rows read "holiday-photos.zip · 38 of 96 MB" and "lecture-notes.pdf · stopped at 62%", with no destination.
  - In android-*.jpg the first row breaks as "38 of / 96 MB".
  - On the PC, the second queue row also has no destination.
- **Principle:** checklist 2 (say what happens).
- **Fix:** give each transfer row a destination line ("to DESKTOP-4K2"). Wrap sizes in a non-breaking span, or move the size to the second line.
- **Status: Deferred (minor).**

### R-13 [Minor] Repeated labels, and badges that look like controls
- **Page / file:** PC flyout and Home, both device pickers; `pages/pc.tsx` lines 49-51, 92 and 198; `pages/android.tsx` line 65; `pages/_index.tsx` line 56.
- **Evidence:**
  - The flyout shows "Connected · Wi-Fi" next to a "Connected" badge (pc-*.jpg).
  - The pickers show "Default · connected" next to "Connected" (android-*.jpg).
  - "Clipboard: On · text and links" sits next to an "On" badge.
  - The "On" and "Ask" pills sit where a control belongs (kit-*.jpg Settings rows) but do nothing.
- **Principle:** checklist 2 (no repeated labels); the Kit caption "control on the right".
- **Fix:** keep the state in one place, preferably the badge, and let the subtitle carry new information (the network, "Default"). Replace "Ask" and "On" with real controls (a dropdown, a switch) or remove them.
- **Status: Deferred (minor).**
  - Still visible in the new pc-*.jpg: "Connected · Wi-Fi" sits next to a "Connected" badge that now also carries a wifi icon.
  - "Ask" is still a badge in the kit-*.jpg Settings rows.

### R-14 [Minor] Border and shadow on the same surface, and panels nested in glass
- **Page / file:** `components/DesignKit.module.css` (`.solid` border), `pages/pc.module.css` (`.dialog`, `.window` and `.toast` add a shadow), `pages/android.module.css` `.phone`, `.glass` (border plus shadow).
- **Evidence:**
  - Dialogs, windows and phone frames carry both `1px solid` and `--shadow-lg`.
  - The tray flyout puts a bordered SolidPanel and bordered tiles inside a bordered glass surface (pc-*.jpg).
- **Principle:** checklist 9 (one elevation style per surface; no card inside a card).
- **Fix:** transient surfaces (dialog, flyout, window) get shadow plus the 1 px glass contour only. Inner panels drop their border and use the `--card` fill alone.
- **Status: Deferred (minor).**

### R-15 [Minor] Duration tokens are not Windows' published durations
- **Page / file:** `base.css` `--motion-*`; motion-*.jpg Easings caption.
- **Evidence:**
  - The tokens are 120, 200 and 300 ms. RESEARCH cites Windows durations of 83, 167, 250 and 333 ms, with hover at 83 ms linear.
  - The Motion page says "hover is 120 ms".
- **Principle:** PRINCIPLES Motion ("the PC uses Microsoft's published durations").
- **Fix:** add PC duration tokens (83, 167, 250, 333) to `design/tokens.toml`. Keep the 120 ms value only as the reduced-motion cross-fade.
- **Status: Deferred (minor).** `base.css` still has 120, 200 and 300 ms.

### R-16 [Minor] The Motion page is a grid of identical cards with inconsistent rest positions
- **Page / file:** `pages/motion.tsx`, `pages/motion.module.css` `.grid`.
- **Evidence:**
  - Seven same-size rounded cards (motion-*.jpg).
  - The spring dots rest at 220 px, mid-track, so they look caught mid-animation. The easing dots rest at 0.
- **Principle:** checklist 1 and 9 (no row of identical cards; prefer rows).
- **Fix:** use one list with a row per token. Rest every demo at the start, or draw start and end ticks on the track.
- **Status: Deferred (minor).**

### R-17 [Minor] Capture overlays cover content
- **Page / file:** every capture.
- **Evidence:**
  - An "Agent is checking the 1280×940 layout" pill sits over the top edge, for example in pc-relay-light.
  - A dark pill near y = 920 overlaps the "Last seen" row in pc-native-light and the Android section headings.
- **Principle:** the review needs clean evidence.
- **Fix:** hide the agent overlay in the screenshot script, then recapture together with R-1.
- **Status: Deferred (minor).**
  - This is a capture-tool artefact, not part of the design, and it is still present in the 7 new captures.
  - It is not counted against any score.

### R-18 [Minor] Navigation and chrome in the prototypes are not real controls
- **Page / file:** `pages/pc.tsx` line 21 (`NavItem` is a `div`), line 31 (the close `X` is `aria-hidden` and not a button), line 130 (`aria-label="QR code placeholder"`); `pages/android.tsx` line 21 (tabs are `span`s).
- **Evidence:** the source only. These items can't take keyboard focus and are not announced as navigation.
- **Principle:** checklist 12 (every control has a Narrator or TalkBack label).
- **Fix:** use `button` or `a` elements with `aria-current`, a labelled close button, and a QR label that describes what the code does ("QR code to pair this PC").
- **Status: Deferred (minor).**

### R-19 [Minor] Off-grid values in shared components
- **Page / file:** `components/Button.module.css` (gap 6 px; padding 10, 14 and 18 px), `components/Switch.module.css` (thumb inset 2 px), `components/DesignKit.module.css` `.attention > span` (margin-top 2 px).
- **Evidence:** these values are not in the 4, 8, 12, 16, 24, 32 scale. The optical 2 px nudges are defensible; the button paddings are not.
- **Principle:** checklist 3.
- **Fix:** set button padding to `--space-3` (sm) and `--space-4` (md and lg), and the gap to `--space-2`.
- **Status: Deferred (minor).**

### R-20 [Minor] One icon family for both platforms, and not the chosen ones
- **Page / file:** all pages import `lucide-react`.
- **Evidence:** RESEARCH chooses Fluent System Icons for the PC and Material Symbols for Android. All screens use Lucide strokes.
- **Principle:** checklist 8 (one icon family per platform).
- **Fix:** acceptable for prototypes. Note in PRINCIPLES that M5 and M6 swap in the chosen families, and check optical sizes then.
- **Status: Deferred (minor).**

### R-21 [Minor] A disabled button is used to carry a status
- **Page / file:** Kit Buttons; `pages/_index.tsx` line 47.
- **Evidence:** "Waiting for the phone" is a disabled primary at opacity 0.45, about 1.45:1 in light (kit-native-light) and navy on grey-blue in dark (kit-native-dark). Disabled controls are exempt from contrast rules, but this text is the message.
- **Principle:** checklist 11; WCAG 1.4.3 in spirit.
- **Fix:** show waiting as status text with a progress ring next to an enabled Cancel button. Keep disabled buttons for actions that are unavailable.
- **Status: Fixed.** kit-native-light, kit-relay-light and kit-native-dark now show "Waiting for the phone…" as muted text with a refresh icon, next to an enabled ghost "Cancel" button.

### R-22 [Minor] (new in the re-check) Dark-theme attention still renders yellow, not amber
- **Page / file:** Kit and PC, dark; `components/DesignKit.module.css` `.attention` and `Badge` `warning`, which use `--warning`, not `--color-accent`.
- **Evidence:**
  - In kit-native-dark, pc-native-dark and pc-relay-dark, the attention banner border, the "Reconnecting…" badge and the flyout "Reconnecting…" text are yellow, consistent with `--color-warning #FCE100`.
  - The attention token named in PRINCIPLES is amber `#E8A93A` in dark.
  - Light `#9D5D00` (warning) and `#955700` (accent) are close enough to read as one colour.
  - Contrast is fine either way: `#FCE100` is 10.43:1 on `#2D2D2D`.
- **Principle:** PRINCIPLES Colour (amber is the attention role); one meaning, one colour.
- **Fix:** point `.attention`, the warning Badge and the tile state text at the attention token (`--color-accent`, later `attention`), or record in PRINCIPLES that dark attention is Fluent yellow.
- **Status: Open.**

## 4. What works, to keep

- **No AI tells.** No gradients, glows, gradient text or emoji. The only "gradient" is the hard white/black test split, which is a test fixture. The titles and captions use size and weight (28 semibold, 16 semibold, 12 muted), not colour boxes.
- **Real, specific copy.** Examples: "Phone notifications are not reaching your PC. Turn on notification access on the phone.", "A VPN is on. If your PC can't reach this phone, turn the VPN off or allow local network access.", "Screen, camera, audio and transfers stop now. Your phone stays paired.", "Calls can't be shared…". Dialog verbs are specific: Accept/Decline, Share sound / Don't share, It matches / Not the same.
- **One accent in practice.** Cobalt `#3651C5` / `#B9C3FF` is the only chromatic action colour, and the primary button and on-primary pairs pass well (6.69:1 light, 7.58:1 dark). Floot's own `--accent` is mapped to a neutral in `base.css`.
- **Text and token contrast is solid.** Body text is 16.82:1 (light) and 13.77:1 (dark) on the card. Muted text is 6.17:1 and 7.98:1. `outline_strong` is 4.39:1 and 3.99:1 on the card, and the Input boundary and PC tile borders use it. All of these fit the 3:1 ruling.
- **Attention banners** combine an icon, words, a border and opaque text (`Attention` with `role="status"`). Keep that pattern.
- **Structure follows the references.** Left navigation with Settings last, a hero, "needs attention" only when something is wrong, rows with the control on the right at 48 px minimum, and 2-per-row tray tiles.
- **Glass policy.** Opaque panels carry the controls, glass carries only the layer, and the values are near-opaque.
  - Measured in the post-fix captures, text on glass is at least 12.9:1 (pc-native-light, over black) and at least 9.1:1 (pc-native-dark, over white).
  - Muted 12 px text on light glass over the black half is the tightest case: `#5F5F5F` on a sampled `#DEDEDE` is 4.7:1 (Native) and 5.0:1 on `#E1E4EB` (Relay). That passes, but with little margin. Keep body text off glass, as PRINCIPLES says.
- **Spring maths is right.** The standard spring (700 / 0.9) settles in about 0.17 s; the page says about 0.15. The emphasised spring (380 / 0.8) overshoots exp(−0.8π/0.6) ≈ 1.5%, as stated. The effects spring is critically damped. The spring demos honour reduced motion.

## 5. Coverage limits

- **Method.** The `design:design-critique` and `design:accessibility-review` skills weren't available to this reviewer. Their criteria (a design critique plus a WCAG 2.1 AA audit) were applied by hand.
- **Capture set.** 23 captures in total:
  - 16 first-pass captures, in `review/first-pass/`. Every one was solidified by the OS reduced-transparency preference (R-1).
  - 7 post-fix captures, in `review/`, with glass forced on: pc-native-light, pc-native-dark, pc-relay-light, pc-relay-dark, kit-native-light, kit-native-dark and kit-relay-light.
  - Not recaptured, because the Floot build limit was reached: kit-relay-dark, kit-relay-light-rt, kit-relay-dark-rt, android-* and motion-*. Their scores are first-pass scores, and the fixes to them are unverified.
- **First screen only.** Every capture shows only about the first 920 px.
  - Below the fold, judged from the first-pass source only. PC: Main window Settings, Pairing and QR, incoming-files consent, the Phone screen window and its Live badge, the Windows toast, Hello unlock, Disconnect everything, the device picker, the transfer queue, update and Public network. Android: everything below the first row of phones, including the consent sheet, which is the only Android glass. Kit: the Input field and the Toast output.
  - R-2 (switches below the fold), R-8, R-12, R-13 and R-18 rest partly on these.
- **The Floot source wasn't re-read.** The build limit also blocks reads. Claims made only in Floot source are therefore unverified: the focus ring (R-3), "Live on DESKTOP-4K2" (R-7), the Live badge icon (R-8) and reduced motion in `EasingDemo` and the Switch (R-10). The repo files (`base.css`, `lib.rs`) were read and confirm the token and test claims.
- **Motion was not seen moving.** Still captures can't show animation. Criterion 8 is scored from the first-pass `motion.tsx`. There is no reduced-motion capture.
- **Glass.** Now observed in the 7 post-fix captures (section 6). Not observed: reduced transparency with glass forced on (no new RT capture), the Android consent-sheet glass, and blur on a photographic backdrop (only the hard white/black split was captured).
- **Modes not captured:** high contrast and forced colours, lite mode, keyboard focus states (needed to close R-3), and reduced transparency after the fix.
- **Rendering limits.** The captures are JPEG, so pixel samples are within about ±2 per channel, and grain at 0.02 is near the compression noise floor. Fonts are the capture machine's Segoe UI.
- **Capture-tool artefact.** The "Agent is checking the 1280×940 layout" pill and the dark bottom pill (R-17) come from the capture tool and are ignored in scoring.
- **Not checked:** the Narrator and TalkBack experience in real use, keyboard order, touch targets on a device, and Floot pages other than the five named.

## 6. Re-check (post-fix captures)

The scale and criteria are the same as in section 2. Only the 7 recaptured pages are re-scored. Every other row in section 2 still stands.

| Capture | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| pc-native-light | 5 | 5 | 4 | 4 | 4 | 5 | 4 | 4 | 4 | 5 | 4 |
| pc-native-dark | 5 | 5 | 4 | 4 | 4 | 4 | 4 | 4 | 4 | 5 | 4 |
| pc-relay-light | 5 | 5 | 4 | 4 | 4 | 5 | 4 | 4 | 4 | 5 | 3 |
| pc-relay-dark | 5 | 5 | 4 | 4 | 4 | 4 | 4 | 4 | 4 | 5 | 5 |
| kit-native-light | 5 | 5 | 4 | 5 | 4 | 5 | 4 | 4 | 4 | 5 | 4 |
| kit-native-dark | 5 | 5 | 4 | 5 | 4 | 4 | 4 | 4 | 4 | 5 | 4 |
| kit-relay-light | 5 | 5 | 4 | 5 | 4 | 5 | 4 | 4 | 4 | 5 | 3 |

Why the scores changed:
- **(6)** Cobalt now marks only actions, selection and "connected". Dark keeps a 4 because attention renders yellow, not amber (R-22).
- **(9)** Badges and off switches now pass (R-2, R-5). The scores stay at 4, not 5, because focus rings are unverified (R-3).
- **(10)** Tiles show name plus state, and the kit badges have icons (R-6, R-8 in part).
- **(11)** Glass now composites at the token alpha with a visible blurred split (R-1).
  - Native: 4. Readable everywhere, but muted text over black is only 4.7:1.
  - Relay dark: 5. Distinct navy tint, a visible 1 px highlight and detectable grain.
  - Relay light: 3. The glass renders, but the look differs from Native only by a faint blue cast (R-4).
- **(4)** Repeated labels remain (R-13).

Status of the claimed fixes:

| Claim | Verdict | Evidence |
|---|---|---|
| Attention light `#C9820F` → `#955700` | Confirmed | `base.css` line 12. 5.20:1 / 5.57:1. Test `attention_is_readable_as_text_and_icon` |
| `*_text` badge colours | Confirmed | `base.css` lines 16-18 and 77-79. 4.76–5.51:1. Test `status_text_is_aa_on_its_tint`. Badge text is darker in the capture (R-5) |
| `data-force-glass` opt-out; R-1 cause | Confirmed | `base.css` line 107. Test `forced_glass_opts_out_of_the_os_preference`. Pixels match the token composites (R-1) |
| Switch off ring, muted thumb | Confirmed | kit-*.jpg header toggles (R-2) |
| Switch, Button and Input focus = primary | Unverified | No focus capture, and the source is unreadable (R-3) |
| Switch thumb transition off under reduced motion | Unverified | No capture, and the source is unreadable (R-10) |
| Badge icons | Confirmed (kit and PC flyout) | kit-*.jpg and pc-*.jpg; the Live badge is unverified (R-8) |
| "Waiting for the phone" as status text plus Cancel | Confirmed | kit-*.jpg Buttons (R-21) |
| PC tiles name plus state; degraded audio neutral with amber icon and words | Confirmed | pc-*.jpg flyout (R-6) |
| Android "Live on DESKTOP-4K2" | Unverified | Not recaptured, and the source is unreadable (R-7) |
| `EasingDemo` honours reduced motion | Unverified | Not recaptured, and the source is unreadable (R-10) |

Open now: 1 Critical (R-3), 5 Important (R-4, R-7, R-8, R-10, R-11) and 1 Minor (R-22).
- Fixed: R-1, R-2, R-5, R-6, R-9 and R-21.
- Deferred (minor): R-12 to R-20.
- R-3, R-7, R-8 and R-10 can likely be closed by one recapture after the Floot limit resets: keyboard focus states, the android and motion pages with reduced motion, and the PC page scrolled to the Phone screen window.
