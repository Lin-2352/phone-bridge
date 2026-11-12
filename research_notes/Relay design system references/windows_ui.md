# Windows 11 / Fluent 2 desktop UI references for Relay (tray flyout + main window)

Research date: 2026-11-06. Sources are mostly learn.microsoft.com pages (updated 2024-2026) and fluent2.microsoft.design. Where a number came from a non-Microsoft or unverified source, it is flagged. Relay is MIT/Apache-2.0: all real apps below are cited for study only; no code is to be copied (PowerToys is MIT, so it is safe to study, but the Quick Share and Phone Link apps are closed source).

## 1. Shape, spacing, type, elevation tokens (Windows 11 + Fluent 2)

### Takeaway
Windows 11 uses three radii: 8 for top-level/transient surfaces (windows, flyouts, dialogs), 4 for in-page controls and bars, 0 where edges touch or the window is maximized/snapped. Spacing is a 4 px base grid, with Windows-specific rhythm of 8/12/16 epx. The type ramp is Segoe UI Variable, Regular and Semibold only, 12/16 to 68/92. Elevation is a 2-layer system with numeric elevations (window/dialog 128, flyout 32, tooltip 16, card 8, control 2, layer 1) and 1 px strokes.

### Cited Findings
**Corner radius**
- 8 px: top-level containers such as app windows, flyouts and dialogs. 4 px: in-page elements such as buttons and list backplates. 0 px: straight edges that meet other straight edges, and windows that are snapped or maximized. — [Geometry in Windows 11](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry)
- Overlay/transient elements (ContentDialog, Flyout, MenuFlyout, TeachingTip) = 8 px; ToolTip is the exception at 4 px. Bar-shaped elements (ProgressBar, ScrollBar, Slider) = 4 px. Global resources are `ControlCornerRadius` (4) and `OverlayCornerRadius` (8). — [Geometry in Windows 11](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry)
- Do not round where two controls touch inside a container (SplitButton), or where a flyout attaches to its invoker on one side. — [Geometry in Windows 11](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry)
- Win32 desktop apps opt in to rounded window corners through the DWM API. — [Geometry](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry) (links to "Apply rounded corners in desktop apps")
- Fluent 2 (cross-platform) radius tokens differ slightly: None 0, Small 2, Medium 4 (default for rectangles), Large 8, X-Large 12 (button sheets, popovers), Circle 50%. Shapes under 32 px use 2 px corners. Strokes: Thin 1, Thick 2, Thicker 3, Thickest 4 (web). — [Fluent 2 Shapes](https://fluent2.microsoft.design/shapes). Note: Fluent 2 web says 12 px for popovers; the Windows guidance says 8 px for flyouts. For a Windows-native look, follow the Windows page.

**Spacing**
- Windows content spacing: 8 epx between buttons, between button and flyout, and between control and header; 12 epx between control and label and between content areas; 16 epx between a surface and its edge text. — [Content layout and spacing](https://learn.microsoft.com/en-us/windows/apps/design/basics/content-basics)
- Multi-line list rows use Body + Caption with 32 epx icons; section headers in lists use Body Strong; expander content is indented 48 epx; controls in expander rows are right-aligned with 16 epx to the expander button. — [Content layout and spacing](https://learn.microsoft.com/en-us/windows/apps/design/basics/content-basics)
- Fluent 2 spacing ramp (base 4 px): 0, 2, 4, 6, 8, 10, 12, 16, 20, 24, 28, 32, 36, 40, 48, 52, 56 (tokens size20 ... size560). The 2/6/10 values exist to align icons to the 4 px grid. Breakpoints: small 320-479, medium 480-639, large 640-1023, x-large 1024-1365, xx-large 1366-1919, xxx-large 1920+. — [Fluent 2 Layout](https://fluent2.microsoft.design/layout)
- Content margins next to a NavigationView: 12 px in Minimal mode, 24 px otherwise. NavigationView header is a fixed 52 px tall. — [NavigationView](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/navigationview)

**Type ramp (Segoe UI Variable, effective px, size/line-height)**
- Caption 12/16 (Small optical); Body 14/20 (Text); Body Strong 14/20 (Text Semibold); Body Large 18/24; Body Large Strong 18/24 Semibold; Subtitle 20/28 (Display Semibold); Title 28/36; Title Large 40/52; Display 68/92. — [Typography in Windows](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography)
- Weights used: Regular (400) for most text, Semibold (600) for titles. Bold and Italic are not part of the ramp. Minimum sizes: 14 px Semibold, 12 px Regular. Sentence case everywhere. Variable axes: wght 100-700, opsz 8-36 pt automatic. Use one font family; 50-60 characters per line. — [Typography in Windows](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography)
- Window title text in the title bar uses Caption style. In confined spaces use Body Strong for a title with no extra spacing; Caption for very small spaces such as command buttons. — [Title bar](https://learn.microsoft.com/en-us/windows/apps/design/basics/titlebar-design), [Content layout](https://learn.microsoft.com/en-us/windows/apps/design/basics/content-basics)

**Elevation and shadow**
- Windows 11 elevation values (shadow + 1 px contour stroke): Window 128, Dialog 128, Flyout 32, Tooltip 16, Card 8, Control 2 (rest and hover 2, pressed 1), Layer 1. Shadow intensity changes with theme at equal value. — [Layering and elevation](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/layering)
- Two-layer system: base layer (menus, commands, navigation) plus content layer (contiguous or cards). — [Layering and elevation](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/layering)
- Fluent 2 shadow tokens (Shadow1 / Shadow2, blur, x, y, opacity): shadow2 light 2,0,1,14% / 2,0,1,14%, dark 28% / 14%; shadow4 4,0,2; shadow8 8,0,4; shadow16 16,0,8 (same 14% light, 28%/14% dark); shadow28 light 28,0,14,24% + 8,0,0,20%, dark 28,0,14,28% + 2,0,0,20%; shadow64 light 64,0,32,24% + 8,0,0,20%, dark 64,0,32,28% + 2,0,0,20%. Rule: blur = n, Y = n/2. — [Fluent 2 Elevation](https://fluent2.microsoft.design/elevation). Caveat: the fetch tool said shadow4/8/16 rows were derived from the page's formula, not listed verbatim.

**Title bar (main window)**
- Standard title bar 32 px tall (48 px if it holds a search box or person picture). Icon 16x16, 16 px from the left edge; title 16 px after the icon; whole empty bar is draggable; double-click toggles maximize; right-click shows the system menu. Caption glyphs: E921 minimize, E922 maximize, E923 restore, E8BB close, full-bleed backplates, rest/hover/pressed/active/inactive states. Title bar elements become semi-transparent when inactive. — [Title bar](https://learn.microsoft.com/en-us/windows/apps/design/basics/titlebar-design)

### Inferences
- Relay tray flyout: 8 px outer radius, 4 px inner controls, 16 px outer padding, 8 px between buttons, 12 px between cards/groups. Concentric rule (inner = outer minus padding) is not stated by Microsoft on the pages fetched, so treat it as a design choice, not a cited rule.
- Relay body text 14/20 Regular, section headers Body Strong 14/20, device name Subtitle 20/28 at most; nothing below 12/16 Caption.
- Use shadow28 or shadow64 style for the tray flyout (Windows rates flyout 32, window 128) plus a 1 px stroke; shadows alone are not enough in dark mode, hence the stroke.

### Gaps
- Windows-specific numeric shadow parameters (the "elevation 32" blur/offset/opacity) are not given in the layering page; only the Fluent 2 token table above is numeric. No official mapping between the two scales was found.
- Segoe UI Variable is installed on Windows 11 only; the fallback for a web-view-based Relay UI on Windows 10 is not covered by the fetched pages (Selawik is listed as a metric-compatible open font: [Typography](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography)).

## 2. Materials: Mica vs Mica Alt vs Acrylic, and fallbacks

### Takeaway
Mica (or Mica Alt) is the base layer of a long-lived window; Acrylic is only for transient light-dismiss surfaces such as flyouts and context menus. All three degrade to solid colors when transparency is off, Battery Saver is on, hardware is low-end, or (for Mica and background acrylic) the window is inactive.

### Cited Findings
- Acrylic is used only for transient, light-dismiss surfaces such as flyouts and context menus. Mica is an opaque material tinted by the desktop wallpaper, with built-in active/inactive states. Smoke dims content under a modal dialog and is always translucent black in both themes. — [Materials](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/materials)
- Use background acrylic for context menus, flyouts, non-modal popups and light-dismiss panes, especially when they draw outside the app window. Do not put desktop acrylic on large background surfaces; do not put multiple acrylic panes edge to edge; do not put accent-colored text or hyperlinks on acrylic; avoid layering several acrylic surfaces. — [Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)
- Acrylic fallbacks: in High Contrast the user's chosen background color replaces acrylic. Both background and in-app acrylic become a solid color when Transparency effects is off (Settings > Personalization > Colors), in Battery Saver, and on low-end hardware. Background acrylic alone also goes solid when the desktop window deactivates, and on Xbox/HoloLens/tablet mode. — [Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)
- Mica: base layer of the app, title bar included (extend into the non-client area with a transparent custom title bar). Samples the wallpaper once, so it is cheap. Do not apply the backdrop more than once per app; set intermediate layers transparent so it shows through. — [Mica](https://learn.microsoft.com/en-us/windows/apps/design/style/mica)
- Mica fallback is the solid color `SolidBackgroundFillColorBase` (Mica) or `SolidBackgroundFillColorBaseAlt` (Mica Alt) when: transparency is off, Battery Saver is on, low-end hardware, window inactive, or Windows build is below 22000. In High Contrast the user's background color is used. — [Mica](https://learn.microsoft.com/en-us/windows/apps/design/style/mica)
- Mica Alt = stronger tint of the wallpaper color; use it when you need contrast between title bar/commanding areas and content, typically a tabbed title bar. It needs Windows App SDK 1.1+ and Windows 11 build 22000+. Layer model for Mica Alt: base Mica Alt, commanding layer `LayerOnMicaBaseAltFillColorDefaultBrush`, content layer `LayerFillColorDefaultBrush`. For plain Mica, the content layer on top uses `LayerFillColorDefaultBrush`. — [Mica](https://learn.microsoft.com/en-us/windows/apps/design/style/mica)
- For vertical nav panes that section off content, Microsoft recommends an opaque background rather than acrylic; in-app acrylic is only for panes that overlay content (Compact/Minimal NavigationView). — [Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)
- Contrast themes use a constrained palette with contrast ratios typically 7:1 or higher; four built-in themes (Aquatic, Desert, Dusk, Night sky); toggled by Left Alt + Left Shift + PrtScn. Pages, panes, popups, bars should use `SystemColorWindowColor` as background; add contrast-theme-only borders where panes share the page color; use 2 px borders for transient surfaces such as flyouts and dialogs. Don't hard-code colors in HighContrast; don't use GrayText for secondary text (disabled only); don't use Hotlight for anything but hyperlinks. — [Contrast themes](https://learn.microsoft.com/en-us/windows/apps/design/accessibility/high-contrast-themes)
- Text contrast: 4.5:1 minimum for visible text (aligned to WCAG 2.0 technique G18), with exceptions for logos and inactive UI text; do not rely on high contrast mode as the fix for low readability. Text size: honor Windows "Text size" (TextScaleFactor range 1 to 2.25). — [Accessible text requirements](https://learn.microsoft.com/en-us/windows/apps/design/accessibility/accessible-text-requirements)

### Inferences
- A Relay tray flyout built as its own top-level window should draw Acrylic only if the host framework provides the system backdrop; otherwise paint an opaque layer color (the same color the fallbacks resolve to). The main window should use Mica (Alt optional) with a content layer, and a solid fallback in light/dark.
- Fallback table for Relay: Transparency off, Battery Saver, inactive window -> solid base color; High Contrast -> system colors (Window / WindowText / ButtonFace / Highlight / GrayText / Hotlight) with 2 px borders on flyout and dialog.
- Detect transparency / contrast live; both can change while the tray app is running.

### Gaps
- Exact hex values of `SolidBackgroundFillColorBase` and `LayerFillColorDefault` (light/dark) were not on the pages fetched (the Color page only describes principles). They are defined in WinUI's theme resources; not verified here.
- The 3:1 non-text contrast ratio for UI components was not found on the Microsoft pages fetched; the pages cite 4.5:1 for text and about 7:1 for contrast themes only.
- The Settings > Accessibility > Visual effects "Animation effects" toggle: found the path via third-party guides ([pureinfotech](https://pureinfotech.com/turnoff-animation-effects-windows-11/)) but no official page on which API apps should read to honor it (SPI_GETCLIENTAREAANIMATION or UISettings.AnimationsEnabled are candidates, unverified).

## 3. Motion

### Takeaway
The Windows motion page gives the concrete values: durations 83/167/250/333 ms and four cubic-bezier curves plus an elastic 3-keyframe entrance. Fluent 2's own motion page gives only qualitative rules (no ms, no beziers, no springs).

### Cited Findings
- Windows animation table (curve, timing, use): Direct Entrance, Fast-In, cubic-bezier(0,0,0,1), 167/250/333 ms, position/scale/rotation. Existing Elements, Point to Point, cubic-bezier(0.55,0.55,0,1), 167/250/333 ms. Direct Exit, Fast-Out, cubic-bezier(0,0,0,1), 167 ms, always combine with fade out. Gentle Exit, Soft-Out, cubic-bezier(1,0,1,1), 167 ms. Bare Minimum fade in/out: Linear, 83 ms, opacity only. Strong Entrance "Elastic In" has 3 keyframes: (0.85,0,0,1) 167 ms, (0.85,0,0.75,1) 167 ms, (0.85,0,0,1) 333 ms. — [Motion in Windows](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/motion)
- Principles: Connected, Consistent (example: all taskbar flyouts slide up when invoked and slide down when dismissed), Responsive, Delightful, Resourceful (avoid custom animations, reuse OS behavior). Settings top-level pages slide up from the bottom; sub-page navigation slides left/right. — [Motion in Windows](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/motion)
- Fluent 2 motion page: principles Functional, Natural, Consistent, Appealing; names linear / ease-in / ease-out / ease-in-out; larger elements get more time; no ms, no cubic-bezier values, no springs published on the page. — [Fluent 2 Motion](https://fluent2.microsoft.design/motion)
- Quick Share for Windows reworked its animations "to reduce system load and improve battery life" in v1.0.1939.4. — [Google Quick Share release notes](https://support.google.com/product-documentation/answer/14185560)

### Inferences
- Relay flyout open: 250-333 ms entrance, translate up from the taskbar edge using (0,0,0,1) combined with fade; close: 167 ms with (0,0,0,1) plus fade out. Hover/press states: 83 ms linear opacity. All of this collapses to opacity-only or instant when animation effects are off.
- No spring values are published by Microsoft in the sources found; do not invent springs for Relay, use the elastic keyframe set only if a "strong entrance" is wanted (probably not for a utility).

### Gaps
- No official Fluent 2 numeric motion tokens (e.g. durationFast/Normal) on the fetched page; Fluent React tokens may exist but were not fetched.
- No official statement on which durations apply to flyouts vs dialogs; the table gives "167/250/333" as a set with no per-component mapping.

## 4. Real shipping references: tray flyouts

### Takeaway
Three patterns exist: a rich anchored flyout on single left-click (PowerToys Quick Access, Windows quick settings), a full app window raised from the tray (Quick Share, Phone Link), and a right-click context menu. Microsoft's older Win32 guidance says the left click should show what the user expects in the best-suited form (popup, dialog or window) placed near the icon.

### Cited Findings
- Official: right-click on a notification-area icon brings a normal shortcut menu; left click "should display what the user would expect to see in the form best suited to that content", e.g. status text or a volume slider, placed near the click using `CalculatePopupWindowPosition`. Icons need 16x16 and 32x32 versions; tooltips are recommended; the user should be asked permission before a permanent icon is installed and be able to remove it via the shortcut menu. Notification title max 48 chars, body max 200 chars (English, for localization). — [Notifications and the Notification Area (Win32)](https://learn.microsoft.com/en-us/windows/win32/shell/notification-area)
- That same page says the notification area is "not for critical information that must be acted on immediately" and not intended for quick program/command access (dated 2018, Windows 7-era; modern Windows 11 quick settings and PowerToys contradict the second half in practice). — [Notification Area (Win32)](https://learn.microsoft.com/en-us/windows/win32/shell/notification-area)
- All taskbar flyouts in Windows slide up on invoke and down on dismiss (the "Consistent" motion principle). — [Motion in Windows](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/motion)
- Flyouts dismiss on outside click, Esc, window resize; dialogs block. — [Dialogs and flyouts](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/dialogs-and-flyouts/)

**Reference A: PowerToys Quick Access flyout (MIT, study only)**
- Learn docs: the tray icon opens either the Settings app or the PowerToys flyout; the Quick Access flyout lets you toggle utilities on/off without opening Settings; it has an enable switch and a keyboard shortcut; there is a "theme-adaptive monochrome tray icon" option; app theme Dark/Light/Windows default applies to both the settings app and the flyout. — [PowerToys General settings](https://learn.microsoft.com/en-us/windows/powertoys/general)
- Single left-click shows the flyout, double-click opens Settings, right-click gives the context menu (this comes from a third-party mirror of PowerToys runner dev docs plus release-note coverage; verify against `doc/devdocs/core/runner.md` in the repo before relying on it). — [Windows Central on Quick Access](https://windowscentral.com/software-apps/windows-11/microsoft-powertoys-for-windows-gets-new-taskbar-quick-access-feature), [Mintlify mirror](https://mintlify.com/microsoft/powertoys/dev/runner)
- What to copy: single-click opens a toggle list (so the common action is one click), double-click opens full settings, a keyboard shortcut for the flyout, a monochrome theme-adaptive tray icon option, theme override independent of Windows. Repo: [github.com/microsoft/PowerToys](https://github.com/microsoft/PowerToys) (MIT).

**Reference B: Windows 11 Quick Settings flyout**
- Tile grid with brightness/volume sliders under it; in 24H2 users can reorder tiles but the add/remove option was reported removed (news sources, moderate reliability). — [allthings.how](https://allthings.how/customize-the-quick-settings-panel-in-windows-11-24h/), [Windows Central 24H2 taskbar](https://www.windowscentral.com/software-apps/windows-11/whats-new-with-taskbar-and-start-menu-on-windows-11-version-24H2-2024-update)
- What to copy: a tile grid of toggles (large hit target, state shown by accent fill) with sliders below, all in one flyout anchored to the taskbar corner, light dismiss. No official dimensions or material spec was found for it.

**Reference C: Google Quick Share for Windows**
- Lives in the tray; closing the window does not quit, Quit is via tray right-click; receives files while backgrounded; shows a notification for incoming files (PIN check if needed), user taps Accept; transfers between own devices on the same Google account auto-accept; notifications are suppressed under Focus Assist/Do Not Disturb. — [Google: Quick Share Android-Windows](https://support.google.com/android/answer/13801258?hl=en)
- Device visibility has four levels: Everyone, Contacts, Your devices, No one; v1.0.2034.4 added changing visibility straight from the tray icon; v1.0.1533.13 added "run at startup" setting; v1.0.1637.0 stopped showing the PIN when both devices share an account. — [Google Quick Share release notes](https://support.google.com/product-documentation/answer/14185560)
- What to copy: visibility/receive state reachable from the tray in one step; "closing the window keeps running, tray Quit exits"; auto-accept only for the user's own paired devices; no PIN friction for already-trusted devices.

**Reference D: Phone Link (Start menu panel)**
- A Start menu sidebar shows phone battery and connection status, buttons for Messages, Calls, Photos, and a Recent section; reviewers note clicking items just opens the full Phone Link app; the recent list can be hidden in Phone Link settings. — [Gizchina](https://www.gizchina.com/2025/05/19/windows-11-start-menu-now-has-a-phone-link-panel/), [Windows Central critique](https://windowscentral.com/software-apps/windows-11/new-start-menu-phone-link-only-adds-clutter), [Geek Rewind](https://geekrewind.com/?p=81706) (secondary sources)
- What to copy: battery + connection status at a glance. What to avoid: a panel whose items only open the full app (Windows Central's complaint).

### Inferences
- Relay tray flyout: left click = anchored flyout with phone name, connection state, battery, and 3-4 big toggles (audio share, mic, clipboard, files); double-click or "Open Relay" = main window; right-click = minimal menu (Open, Pause, Quit). That mirrors PowerToys and Quick Share.
- Provide a flyout shortcut and a monochrome theme-adaptive tray icon with state variants (connected, sharing, paused), per PowerToys and the Win32 icon guidance (16 and 32 px assets).
- Position the flyout by tray icon rect (Shell_NotifyIconGetRect / CalculatePopupWindowPosition), slide up from the taskbar edge.

### Gaps
- Phone Link's official Microsoft Support page was not retrievable (fetch returned 404 for the URL tried; searches returned only third-party sites). Its tray behavior is unverified.
- No official dimensions (width, padding) for the Windows 11 quick settings flyout, or confirmation of its material, were found.
- PowerToys flyout source was not read (study-only, not required); only docs and third-party descriptions were used.

## 5. Real shipping references: main window with left navigation

### Takeaway
The official pattern is WinUI NavigationView in Left mode: 320 px expanded pane, adaptive collapse to icon-only below 1008 px and to a hamburger at 640 px or less, Settings pinned at the bottom, 52 px header, 24 px content margins, Mica base with a content layer.

### Cited Findings
- Left nav is recommended for 5-10 equally important top-level categories; top nav for 5 or fewer. Default adaptive behavior (PaneDisplayMode Auto): expanded at 1008 px or wider, LeftCompact (icons only) at 641-1007 px, LeftMinimal (menu button only) at 640 px or less. Ideal hierarchy depth is two levels. Settings is the last item, pinned to the bottom (`IsSettingsVisible`). — [NavigationView](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/navigationview)
- `OpenPaneLength` default is 320 DIP. — [NavigationView.OpenPaneLength](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.navigationview.openpanelength)
- In Left NavigationView with Mica, the content layer sits on top of Mica; card layout is an alternative; Mica shows through the title bar. — [Mica](https://learn.microsoft.com/en-us/windows/apps/design/style/mica)
- App settings page guidance: open full window, single scrollable column, max content width about 1000-1100 px, section headers in BodyStrong, SettingsCard (header, description, icon, right-aligned action) and SettingsExpander (no nesting deeper than one level), toggle switches for binary settings, radio buttons for up to 5 exclusive options, changes apply immediately with no Save button, disabled settings explain why in the description, theme choice offered as Light / Dark / Use system setting, an About expander at the bottom (name, icon, version, repo link, bug link, legal). Keep total settings small (4-5 groups). — [Guidelines for app settings](https://learn.microsoft.com/en-us/windows/apps/design/app-settings/guidelines-for-app-settings)

**Reference A: Windows 11 Settings app**
- Cited indirectly: Microsoft's motion doc shows Settings pages sliding up for top-level pages and left/right for sub-pages. — [Motion in Windows](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/motion). No separate official page describing Settings layout was fetched; the SettingsCard layout above is the documented derivative of that design.
- What to copy: left nav + page header + stacked cards of "icon, title, description, control" rows with expanders.

**Reference B: PowerToys Settings (MIT, study only)**
- Left nav organized by utility, global search box at top of settings that filters live and jumps to the setting, shortcut-conflict detection window, version/updates status surface, Dark/Light/Windows-default app theme, backup/restore. — [PowerToys General settings](https://learn.microsoft.com/en-us/windows/powertoys/general), [PowerToys overview](https://learn.microsoft.com/en-us/windows/powertoys/)
- What to copy: settings search, an "update status" row, one master enable switch per module with its sub-settings beneath, and a diagnostics/bug-report package generator.

**Reference C: Google Quick Share for Windows**
- Single-window app: settings gear at top right; Device visibility inside; drag-and-drop zone plus "Select files" / "Select folders"; right-click "Send with Quick Share" shell integration; do not run as administrator. — [Google: Quick Share Android-Windows](https://support.google.com/android/answer/13801258?hl=en)
- What to copy: one obvious primary action area (drop zone), shell integration entry points, one visibility control.

**Reference D: Phone Link**
- Tabbed layout: Messages, Calls, Photos (plus Apps, notifications, etc., depending on the phone); gear at top right opens Settings with a Features tab of per-capability toggles (Notifications, Calls...), each tied to a granted permission; calls are placed through the phone. — [Windows Central](https://www.windowscentral.com/software-apps/windows-11/i-connected-my-phone-to-windows-11-and-now-i-cant-imagine-working-without-it), [Geek Rewind](https://geekrewind.com/how-to-enable-or-disable-phone-link-to-make-phone-calls-from-your-mobile-device-in-windows-11/) (secondary sources)
- What to copy: a Features page where each toggle maps 1:1 to a permission the user granted on the phone, showing it as off when the phone side has revoked it.

### Inferences
- Relay main window nav (about 5-6 items fits Left mode): Devices/Home, Audio, Clipboard (or Sharing), Files, Activity/Log, then Settings pinned at the bottom. Open pane 320 is wide for a small utility; Microsoft documents the default, and the property is overridable, so a narrower pane (for example 240-280) is a design choice not a guideline.
- Settings page: SettingsCard rows under BodyStrong headers (General, Audio, Privacy, Advanced, About), live apply, About expander with version and repo link.
- Min window width: honor the 640 / 1008 breakpoints or collapse nav manually.

### Gaps
- Official 48 px compact pane width and exact item heights (40 px) for NavigationView are not on the fetched pages; the numbers circulate widely but were not verified here.
- No official Microsoft page was fetched that documents Windows Settings' exact layout (nav width, header heights).
- PowerToys' Settings nav structure details (group names) were not read from source.

## 6. Consent and permission dialog patterns

### Takeaway
Use ContentDialog only when a decision truly blocks the user (security, permanent change, a question the app cannot decide); otherwise use a non-blocking toast or an in-flyout prompt. Buttons are specific verbs, with the safe action always present, rightmost in Windows' ordering, and Esc mapped to it.

### Cited Findings
- Use a dialog for: info the user must read before proceeding (security might be compromised, permanently altering or deleting a valuable asset), app-wide errors like connectivity, and blocking questions the app cannot answer for the user. Use a flyout when it is OK for the user to ignore it. Consider undo instead of repeated confirmations. If a dialog or notification would appear every few minutes, find room in the primary UI instead. — [Dialogs and flyouts](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/dialogs-and-flyouts/)
- Dialog rules: state the issue in the first line; title is optional and the main instruction (short); content required; at least one safe, non-destructive CloseButton ("Got it", "Close", "Cancel") that also handles Esc, back and gamepad B; use specific button text such as "Allow" / "Block" for "Do you want to allow AppName to access your location?"; "do it" buttons leftmost, safe action rightmost; up to three buttons (use three sparingly); optional DefaultButton gets accent style and responds to Enter and initial focus; no inline validation errors in dialogs; only one ContentDialog per window at a time; don't use the deprecated MessageDialog. Confirmation dialog: affirmation button on the left, cancel on the right, and buttons should name the specific response. — [Dialog controls](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/dialogs-and-flyouts/dialogs)
- Smoke (translucent black) dims content beneath a modal dialog; dialog elevation 128; dialog radius 8; 2 px borders under contrast themes. — [Materials](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/materials), [Layering](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/layering), [Geometry](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry), [Contrast themes](https://learn.microsoft.com/en-us/windows/apps/design/accessibility/high-contrast-themes)
- Toast / app notification guidance: notifications should be informative, not distract or create more work; buttons trigger an immediate action and let the user complete the task in the toast; images (logo override, inline, hero), quick reply boxes, context menu buttons and progress bars are available; don't be noisy: suppress to Notification Center without an alert, and respect Windows 11 Focus Sessions (detectable via `FocusSessionManager`); a click on the toast should launch the app in context, but background actions (like quick reply) should not launch the app; clear stale notifications from Notification Center once the user has handled the item elsewhere. — [Notifications design basics](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-ux-guidance)
- App notification limitation: elevated (administrator) apps cannot send or receive app notifications via the Windows App SDK. — [App notifications overview](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/)
- Win32 notification-area rules: notifications should be ignorable and never require immediate action; no notification is important enough to override quiet time; notifications sent during quiet time are discarded; check `SHQueryUserNotificationState` for presentation/away states. — [Notification Area (Win32)](https://learn.microsoft.com/en-us/windows/win32/shell/notification-area)
- Real-world: Quick Share uses a notification with Accept for incoming transfers (PIN confirmation if shown), auto-accepts for same-account devices, and its notifications are held back by Focus Assist/Do Not Disturb. — [Google: Quick Share](https://support.google.com/android/answer/13801258?hl=en)
- App theme and settings rule: setting changes apply immediately, no confirm button. — [Guidelines for app settings](https://learn.microsoft.com/en-us/windows/apps/design/app-settings/guidelines-for-app-settings)

### Inferences
- Relay first-time pairing / "phone wants to share audio/mic": show a toast (Allow / Block) when the main window is not focused, and an inline consent card in the flyout or a ContentDialog when the window is in front. Button labels: "Allow" / "Block" (or "Share audio" / "Not now"), never OK/Cancel. Default button = the safe choice for mic access (microphone should be opt-in; do not default-focus Allow).
- Because Focus Sessions/Quiet Time can suppress toasts, the pending request must also appear as a persistent state in the tray flyout (badge on the icon) so a missed toast does not strand the session; consent must time out into "denied".
- Revocation must be reflected immediately in the UI (state displayed matches real state), consistent with the "changes apply immediately" rule.

### Gaps
- Toast button count limits and expiration values were not captured from the fetched pages (the UX guidance page is qualitative); the first fetch of `toast-ux-guidance` returned 404.
- No official Microsoft guidance was found specifically about consent for microphone/audio sharing between devices; the "location access" example is the closest.

## Decision cheat sheet (one official guideline + one real app each)

| Decision | Official source | Real app |
|---|---|---|
| Outer radius 8, inner controls 4 | [Geometry](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry) | WinUI/Windows 11 controls (see [WinUI Gallery](https://apps.microsoft.com/detail/9P3JFPWWDZRC)) |
| 4 px grid; 8/12/16 rhythm | [Content layout](https://learn.microsoft.com/en-us/windows/apps/design/basics/content-basics), [Fluent 2 Layout](https://fluent2.microsoft.design/layout) | Windows Settings (cards) |
| Segoe UI Variable, Regular/Semibold, 14/20 body | [Typography](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography) | PowerToys Settings |
| Mica base + LayerFill content, Acrylic only for flyouts | [Mica](https://learn.microsoft.com/en-us/windows/apps/design/style/mica), [Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic) | Windows 11 Settings / context menus |
| Solid fallback when transparency off / inactive / battery saver | [Mica](https://learn.microsoft.com/en-us/windows/apps/design/style/mica) | Windows 11 itself |
| Contrast themes: system colors, 2 px borders on flyout | [Contrast themes](https://learn.microsoft.com/en-us/windows/apps/design/accessibility/high-contrast-themes) | Calculator example in same page |
| Flyout slide-up 167/250/333 ms with (0,0,0,1) | [Motion](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/motion) | Taskbar flyouts |
| Left-click anchored flyout, right-click menu, double-click opens app | [Notification area](https://learn.microsoft.com/en-us/windows/win32/shell/notification-area) | PowerToys Quick Access |
| Left nav, Settings pinned at bottom, 1008/640 breakpoints | [NavigationView](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/navigationview) | Windows Settings / PowerToys |
| Settings = SettingsCards, live apply, About expander | [App settings guidelines](https://learn.microsoft.com/en-us/windows/apps/design/app-settings/guidelines-for-app-settings) | PowerToys General settings |
| Consent = specific verbs, safe action present, Esc = safe | [Dialogs](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/dialogs-and-flyouts/dialogs) | Quick Share accept prompt |
| Respect Focus Sessions / quiet time | [Notifications design basics](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-ux-guidance) | Quick Share (held by Focus Assist) |
