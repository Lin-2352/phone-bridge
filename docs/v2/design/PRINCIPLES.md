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
- **Amber is a status role, never a second accent.** The existing `accent` token (amber) is retired as an accent. Its jobs are "attention" (pairing codes, "sharing live" notices, degraded links) and, in its darker form, `warning`. It always comes with an icon and words, and is never used for selection, links or primary buttons. The token is renamed `attention` when the M5 and M6 screens that use it (31 PC call sites, a few Android ones) are rewritten; until then `accent` stays a deprecated name for the same value. Sources: Fluent 2 (yellow means caution, status colours are not decoration); Android's permission guidance (an error icon and colour for a degraded feature).
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
- **Type:** PC Segoe UI Variable, Regular and Semibold, ramp 12/16, 14/20, 18/24, 20/28, 28/36. Android uses the Material 3 scale.
- **Motion:** Android uses the Material 3 springs; the PC uses Microsoft's published durations and easing curves (no Windows spring is published, so none is invented). Effects (opacity, colour) never overshoot.

## How a screen is reviewed
Each Floot page (M4.9) and each native screen (M5, M6) is scored line by line against the checklist, the contrast floors and the modes, with screenshots in both looks, light and dark, and each reduced mode.
