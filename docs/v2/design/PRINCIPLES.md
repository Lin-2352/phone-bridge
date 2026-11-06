# Relay design principles

How Relay looks and moves, and the checklist every screen is held to. Decisions and their sources are in [RESEARCH.md](RESEARCH.md); values live in `design/tokens.toml`. Two sources per principle: a guideline and a real app, or a rule from WCAG.

## The anti-AI-look checklist
A screen fails review if it breaks any line. The first six are the SPEC's; 7 to 12 come from the research (RESEARCH.md, "Anti-AI-look checklist").

1. No generic purple-blue gradients, no emoji icons, no row of identical rounded cards.
2. Real copy: say what happens, in the user's words. No marketing adjectives, placeholder names or repeated labels.
3. A 4/8-pt grid with optical alignment. Every spacing, radius and size comes from the token scale; section gaps are visibly larger than item gaps.
4. One accent colour (see "Colour" below).
5. Hierarchy from size and weight, not from boxes or colour.
6. Motion only where it means something (spatial continuity, state feedback, a changed value). No looping ornament.
7. No gradient text, no glow halos, no gradient surfaces.
8. One icon family per platform and one weight; no icon tile above a heading.
9. No card inside a card; prefer rows and lists to card grids. One elevation style per surface (a border or a shadow, not both).
10. Glass only where it solves layering (the tray flyout, sheets and menus), never as decoration. Body text sits on opaque layers.
11. Left-aligned content. Status is stated in words plus an icon, never by colour alone.
12. Every control has a Narrator or TalkBack label.

## Colour (rulings)
- **One accent: `primary` (cobalt).** It marks actions, selection and "connected". Neutrals carry everything else. Sources: Fluent 2 colour (brand colour for buttons, CTAs and selected states, avoid overuse); the Windows 11 Settings and Pixel Settings pattern of one tint on neutral surfaces.
- **Amber is a status role, never a second accent.** The existing `accent` token (amber) is retired as an accent. Its jobs are "attention" (pairing codes, degraded links, a share that needs action; a share that is merely live is the selected state in `primary` with a state word such as "Live on DESKTOP-4K2", never colour alone) and, in its darker form, `warning`. It always comes with an icon and words, and is never used for selection, links or primary buttons. Status words, badges and amber text only ever sit on opaque layers, never directly on glass: the glass tests cover `text`, `text_muted` and `primary` only, and with the project's own formula status colours fall below 4.5:1 on glass over a pure white or black backdrop (M4.R review I-1, for example badge text on Native dark glass over white about 3.5:1). The token is renamed `attention` in M5.1 (the token file, the generator, `Tokens.kt`, the PC `Palette.amber` field and the Android `Theme.kt` users); until then `accent` stays a deprecated name for the same value. On the PC, `Palette.accent` already reads `primary` (cobalt) at all 22 colour reads and 11 `on_accent` reads, and the amber token is not drawn anywhere yet, so M5.1 also renames `Palette.accent`/`on_accent` to `primary`/`on_primary`. Its light value is `#955700` (5.2:1 on the surface) since M4.9; the original `#C9820F` failed text contrast (3.1:1). Text on a status tint uses the `success_text`, `warning_text` and `danger_text` tokens. Sources: Fluent 2 (yellow means caution, status colours are not decoration); Android's permission guidance (an error icon and colour for a degraded feature).
- **Danger (red) is for failure and destructive actions only.**
- **WCAG 1.4.11 for component boundaries.** A boundary that is the cue that identifies a control (an input's border, an outlined button, an unlabeled toggle) is at least 3:1 against the surface next to it, in both themes. Hairline dividers and card edges may be lower, and the `outline` token stays a divider colour; controls that need a stronger boundary use a separate, darker token (`outline_strong`, added in M4.3). Focus indicators are always 3:1. Sources: WCAG 1.4.11; Material 3's `outline` and `outline-variant` split.
- **Contrast floors.** Text 4.5:1 and icons 3:1 over a worst-case pure white or pure black backdrop, in every mode. Text-bearing glass has alpha 0.70 or more and icon-only glass 0.50 or more (RESEARCH.md derives the minimums at about 0.58 and 0.45).

## Glass and modes
- Every glass material has three states: blurred, scrim-only (alpha 0.70 or more, no blur), and solid. The solid state is built and tested first.
- Reduced transparency, high contrast and lite mode resolve to solid. Grain is dropped first in any fallback.
- Reduced motion replaces travel and springs with a 100 to 150 ms cross-fade and keeps colour and focus feedback.

## Shape, space, type, motion
- **Radii:** PC 8 for windows, flyouts and dialogs, 4 for controls, 0 where edges touch. Android 16 for cards, 28 for dialogs and sheets, full for chips and buttons. Sources: Windows 11 geometry; Material 3 shape scale.
- **Spacing:** a 4 base; steps 4, 8, 12, 16, 24, 32. Android targets are at least 48 dp.
- **Type:** PC Segoe UI Variable, Regular and Semibold. The tokens are sizes 12, 14, 16, 20 and 28 (caption, body, subtitle, title, display); Microsoft's ramp is 12/16, 14/20, 18/24, 20/28, 28/36 (size/line height). Ruling: the subtitle stays 16, not Microsoft's 18 - the PC has no 18 px role and the Android users of `Tokens.Type.Subtitle` would grow with it - cost if wrong: PC subtitles are 2 px smaller than Windows' own. Android uses the Material 3 scale (decided in M6.0a).
- **Motion:** Android uses the Material 3 springs. The PC uses Microsoft's published easing curves for every transition (page change, flyout open, dialog in) and, per SPEC M5.2, a spring in `ease.rs` only where motion follows the user's hand (a released drag such as the nav track or a scrollbar thumb, a fling settling); no Windows spring is published for transitions, so none is invented there. The PC durations are Relay's own tokens, 120/200/300 ms (Ruling in M5.0a: Microsoft's published 83/167/250/333 ms are not adopted, because one set of durations on both platforms keeps the motion demos honest and the cost is a few tens of milliseconds slower than Windows' own apps; if wrong, add PC duration tokens through gen-tokens and swap them in `ease.rs` only). Effects (opacity, colour) never overshoot.

## How a screen is reviewed
Each Floot page (M4.9) and each native screen (M5, M6) is scored line by line against the checklist, the contrast floors and the modes, with screenshots in both looks, light and dark, and each reduced mode.
