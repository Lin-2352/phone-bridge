# Relay v2: design spec, one branch `cloud/v2-upgrade`

**Status:** approved by the owner on 2026-11-05. Progress: [PROGRESS.md](PROGRESS.md).

This follows the superpowers:brainstorming architectural path:
1. The spec is committed as `docs/v2/SPEC.md` (the repo ignores `docs/superpowers/`).
2. Each milestone gets its own superpowers:writing-plans plan (saved in `docs/v2/plans/`), written when that milestone starts, against the code as it is then.
3. superpowers:subagent-driven-development runs each plan with TDD, a review after every task, a review after every milestone, and a final review of the whole branch.

## Context
The 2026-11-04 review (`docs/REVIEW-2026-11-04.md` on `cloud/v1-review-findings`) found 12 priority issues and about 50 more. You asked to:
- fix the Top 12;
- bring Relay up to a professional standard in security, features, UI and UX;
- redesign both apps with an acrylic, glassy, smooth look that doesn't read as AI-made, researched against real apps and prototyped in Floot;
- keep the Android app light enough for 10–15-year-old phones;
- test everything here, then hand over a README checklist that Claude on your desktop runs on the real devices.

**Not promised:** "bug-free" can't be proven. The measurable bar is in Verification.

## Your decisions (from our Q&A)
| Topic | Decision |
|---|---|
| Branching | One branch, `cloud/v2-upgrade`, from `main`. The first commit cherry-picks the review doc (`88a3e58`) |
| Floot | Prototype every PC and Android screen in your "Phone Bridge design" project, then rebuild the native apps to match through `design/tokens.toml` |
| Themes | Two, chosen by the user on both apps: **Native glass** (the OS's own materials) and **Relay glass** (one custom frosted look) |
| Old devices | Android 10+ stays (`minSdk 29`), plus `armeabi-v7a`. Lite mode turns on automatically. No blur below Android 12 |
| Level up | All four groups: Security shield (with an unlock on/off setting), Several devices, Transfers, Stay current and trusted |
| Testing | Built and tested here and in CI. `docs/DEVICE-TEST-CHECKLIST.md` for Claude on your desktop at the end |

## What the cross-check changed
An architect reviewed the first draft. I then checked each of its claims against the code and the platform docs (sources at the end).

| Change | Why |
|---|---|
| **No breaking protocol bump.** The version stays 0 and new features are gated by capability. `Hello` gains `min_compatible` and `app_version`. Relay 0.2.x and 0.3 still talk to each other, with the new features off | `Hello` already carries capabilities, and audio is already gated that way (`pb-protocol/src/lib.rs:23-58`) |
| **The PC unlock gate needs Windows 11.** On Windows 10, over Remote Desktop or without Hello, it's unavailable and says why | `IUserConsentVerifierInterop` needs build 22000. The README promises Windows 10 2004 |
| **The Android gate depends on the API level.** API 29 uses `BIOMETRIC_WEAK \| DEVICE_CREDENTIAL`. The activities that host the gate become `FragmentActivity` | `BIOMETRIC_STRONG \| DEVICE_CREDENTIAL` is unsupported on API 29 and lower. BiometricPrompt needs a FragmentActivity, and Relay's Compose activities are `ComponentActivity` |
| **The portable-folder fix is broader.** Elevation is refused from a folder other users can write. The camera DLL is embedded in `Relay.exe`. DLL planting gets a fix that CI tests | Anyone who can swap the DLL can also swap `Relay.exe` (`camera_register.rs:356-368`, `:465-481`). `/DEPENDENTLOADFLAG` alone is disputed (Full Disclosure, 2020) |
| **Heuristics instead of hidden APIs** for OTP redaction and restricted settings | `Ranking.hasSensitiveContent()` is `@SystemApi` |
| **Material 3 Expressive only if it's stable** by the time of the Android redesign; otherwise our own token springs | material3 1.5.0 is at RC, and 1.4.0 stable reportedly dropped the Expressive APIs |
| **Relay's own remote input pauses over every consent prompt.** Moved into M1 | A PC that already has Control could otherwise tap Allow on the phone |
| **"Never fail" became "resume".** After an app restart, a transfer resumes only where the sender can still read the file | Android share-sheet URI grants are temporary |
| **`qa` builds keep the release defaults** (gate on); debug builds have it off | `qa` exists to test what ships (`build.gradle.kts:47-51`) |
| **Every review finding has a milestone** or an explicit deferral | See the milestones and "Deferred on purpose" |

## Defaults I chose (change any in review)
- **Theme:** Native glass on first run.
- **Unlock gate:**
  - On in release and `qa` builds; off in debug builds.
  - Turning it off takes one unlock. After a successful unlock, nothing asks again for 60 s.
  - The device checklist starts by turning the gate off, and has its own section that turns it back on.
- **Update check and clipboard history:** opt-in (THREAT_MODEL rule 4: "no third-party services by default").
- **"Disconnect everything":** Relay stays paused until you resume it.
- **SDK levels:** `targetSdk` stays 36. Target 37 is prepared but not switched on, because it brings A8's while-in-use audio rule, which needs a device. `compileSdk` rises only if a stable library requires it.

## Global constraints
- **Licence:** MIT OR Apache-2.0. No code from GPL or AGPL projects; studying their UIs is fine. Icons come from Fluent System Icons (MIT) and Material Symbols (Apache-2.0).
- **Security model unchanged:** "the PC can only ask". Sensitive actions need the phone's own consent, and every live share stays visible with a Stop.
- **Budgets:**
  - Android: background PSS ≤60 MB (`DECISIONS.md:95`).
  - PC (`DECISIONS.md:393`): the idle tray uses 0 CPU, with ≤40 MB working set and ≤15 MB private while no window is open; ≤70 MB private with the main window open.
  - New, recorded in DECISIONS in M2: at most one Android wakeup per 15 min when idle with no PC.
  - Effects run only while visible.
- **Accessibility:**
  - WCAG AA (4.5:1 for text, 3:1 for icons) over the worst-case backdrop.
  - Honour reduced motion, reduced transparency (the OS setting on Windows, an in-app setting on Android), high contrast and font scale.
  - Narrator and TalkBack labels on every control.
- **Wire changes:** every new message or field is capability-gated and documented in WIRE_PROTOCOL.md. Every parser keeps its size limits and gets a fuzz target.
- **Docs:** each milestone updates THREAT_MODEL, WIRE_PROTOCOL and DECISIONS for what it changes (`CONTRIBUTING.md:16`).
- **UI text:** new strings go through Android string resources or the PC's `strings.rs`. English only for now.

## Key technical choices
1. **Themes:** materials and motion are tokens in `design/tokens.toml`. They are generated into `tokens.rs`, `Tokens.kt` and Floot's `base.css`, with contrast tests, so there is one source of truth.
2. **Relay glass on Windows:**
   - DWM Acrylic as the base, with tint, edge highlight and grain drawn in Direct2D, so the OS does the blur.
   - A solid fallback whenever DWM refuses, transparency effects are off, or high contrast is on.
3. **Unlock gate:** BiometricPrompt or Windows Hello before the action. Binding consent to a Keystore key is a later step.
4. **Keyed discovery:**
   - Each pair's key comes from X25519 static-static DH over the paired Ed25519 identities, derived inside `pb-identity`.
   - No re-pairing is needed, and strangers can't compute the key.
5. **Resume:** offset resume. The receiver proves its partial file by hash before the sender seeks.

## Progress tracking and handoff (added at your request)
- **The progress file**, `docs/v2/PROGRESS.md` on the branch:
  - Lists every milestone and task in this spec, in order, each marked done, in progress or to do, with the commit that finished it.
  - Is updated and pushed after every task, so a new session can resume from GitHub alone.
- **"Resume here" at the top:**
  - the branch;
  - the last commit;
  - the next task;
  - what a new agent reads first (this spec, the current milestone plan, the progress file);
  - the commands that check the state.
- **Budget mode** (limited session credit):
  - Tasks run inline (superpowers:executing-plans), with TDD and self-review.
  - A reviewer agent runs once per milestone instead of after every task.
  - CI runs on every push.
  - Anything unfinished stays on the list as to do.

## Milestones
Each milestone is one plan. The ledger records each milestone's last commit, so `main` could be fast-forwarded to any of them.

**One change from the order you approved** (Harden → Redesign → Level up): the protocol, peer-ID and migration foundations (M3) now come before the redesign. The new Devices and Transfers screens depend on them, and doing them first avoids building those screens twice.

### M0 Baseline (no change in behaviour)
- **Branch:** create it, cherry-pick `88a3e58`, commit this spec.
- **Toolchain:**
  - Rust 1.99.1 here, matching CI.
  - Try installing the Android SDK here; if that's blocked, the Android checks run only in CI.
  - `scripts/android-build.sh`, a bash version of `android-build.ps1`.
- **CI:**
  - Every action pinned by commit SHA, including `dtolnay/rust-toolchain` (currently `@master`) and `setup-oras`.
  - New jobs: Android unit tests, lint, the `qa` (R8) build, APKs for arm64-v8a, armeabi-v7a and x86_64, and the portable zip as an artifact.
- **armeabi-v7a:**
  - Add the ABI, and the target in `deny.toml`.
  - Fix `files.rs:462`, with a test. On 32-bit, `(size - done) as usize` truncates, so a remaining size that's a multiple of 4 GiB reads 0 bytes and the send fails.

### M1 Harden: the Top 12
Each item gives the fix, then its tests.

1. **Clipboard and file injection (S1, B4, S10)**
   - **Android:**
     - Every share opens a compact confirm sheet (target device, preview, Send), as Quick Share and LocalSend do. In M8 this sheet becomes the device picker.
     - Text shared together with files is kept. Links are found anywhere in the text.
     - The own-files guard compares parsed URIs.
   - **PC:**
     - Shared text shows a toast, "Text from ‹device›", with Copy or Open. The toast never shows the text itself.
     - Auto-copy is a setting, off by default. When it copies, it honours `keep_in_history`.
   - **Tests:** JVM for the share decision; Rust for the tray's decision.
2. **Lock-stop taken as a user Stop (A1, B2)**
   - A projection the system stops ends as `SystemStopped`, so no user stop is recorded and the PC isn't declined.
   - The PC shows "Phone locked: sharing stopped" with a Resume button, which sends a fresh request. Audio reports the same.
   - **Tests:** Rust (no `screen_user_stop` is recorded); JVM (stop-reason mapping).
3. **OTP redaction on Android 15+ (A2)**
   - Redaction is detected when the text equals Android's own placeholder, looked up from system resources at runtime *(verify on Android 15+)*.
   - The PC then shows "Code hidden by Android: open on phone".
   - Settings explains why, and the optional `appops` grant *(verify)*.
   - `find_otp` learns more languages.
   - **Tests:** JVM mapping; Rust display.
4. **Restricted settings and Advanced Protection (A3, A10)**
   - A guided card with deep links appears when `packageSource` (API 33) shows a sideloaded APK and the permission is still off. The app-op, read by its string name, is a best-effort hint *(verify)*.
   - It covers Control, Notification access and, on Android 15+, the overlay permission.
   - Android 17 Advanced Protection is detected and explained; Relay never fakes `isAccessibilityTool`.
   - **Tests:** JVM decisions.
5. **Tapjacking and self-injection (S3, S8, critique I6)**
   - `setHideOverlayWindows(true)` (API 31+) on every consent window, the SAS dialog, the quick panel and the gate.
   - API 29–30: `filterTouchesWhenObscured`, and obscured touches on confirm buttons are refused.
   - `setAccessibilityDataSensitive` (API 34+; *verify* under Compose).
   - Relay's own remote input pauses, with keys released, while any prompt Relay opened is on screen, including Android's capture prompt.
   - The quick panel shares the clipboard only on an explicit tap.
   - **Tests:** JVM policy; the input-pause state machine.
6. **Locked phone (S4)**
   - Nothing is auto-accepted while the phone is locked: no control, screen (Instant screen included), camera or audio. A "Unlock to allow" notification appears instead.
   - Control input pauses while locked.
   - On API 29–30, the notification's Allow opens the activity.
   - **Tests:** JVM `shouldAutoAccept`; the input pause.
7. **Mark-of-the-Web (S2)**
   - Every received file gets `Zone.Identifier` with ZoneId=3. Where that fails (FAT, exFAT), it's logged and the save goes ahead.
   - A file is risky if `AssocIsDangerous` says so, or its extension is on our list: `.exe .msi .msix .appx .com .bat .cmd .ps1 .psm1 .vbs .vbe .js .jse .wsf .hta .jar .reg .cpl .scr .pif .msc .chm .lnk .url .scf .library-ms .searchConnector-ms .appref-ms .settingcontent-ms .iso .img .vhd .vhdx .xll`.
   - Risky files always prompt, even with auto-accept.
   - Shortcut types (`.lnk .url .scf .library-ms .searchConnector-ms`) are refused unless "Allow shortcut files" is on.
   - **Tests:** Rust units; a Windows CI test that writes the stream and reads it back.
8. **Portable folder trust (S7, revised)**
   - **Data folder:** `data\` gets a protected DACL (this user, SYSTEM, Administrators) at every start.
   - **App folder:**
     - If anyone else can write to it, Relay warns and offers "Fix folder permissions" (only when this user owns the folder).
     - The camera install refuses to elevate from such a folder, and the elevated process checks again before acting.
   - **Camera DLL:** embedded with `include_bytes!`, through `build.rs` from the freshly built DLL. The elevated installer never reads a file next to the exe.
   - **DLL planting:**
     - `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)` runs first in `main`.
     - Static imports are limited to KnownDLLs and API sets; everything else is delay-loaded.
   - **Tests:** ACL decisions as pure functions. A CI planting test runs `Relay.exe` from a folder seeded with garbage DLLs named after its imports; it must start, and every DLL must load from System32.
9. **One event thread for multi-GB copies (B1)**
   - Received files are saved on their own ordered IO executor.
   - **Tests:** a JVM test with a slow fake saver answers a control request mid-copy.
10. **Android 17 audio hardening (A8)**
    - Volume is restored only while Relay holds a foreground service. The Stop paths restore it before `stopSelf`.
    - With no service running, the restore waits for the next start or app open.
    - **Tests:** JVM "restore now or defer"; a checklist step with `cmd audio set-enable-hardening enable`.
11. **Local network permission (A9)**
    - `ACCESS_LOCAL_NETWORK` is declared. A helper requests it on Android 17+ once Relay targets 37, and the onboarding copy is ready.
    - **Tests:** JVM.
12. **Release hygiene (S6)**
    - The test hooks move behind a `test-hooks` cargo feature, and CI scans the release exe for their flag strings.
    - `relay-cli.exe` leaves the user zip.
    - Without the release key, the release APK task fails when it runs, so a fresh checkout still configures.
    - `qa` stays debug-signed, with the release defaults.

**Fuzzing:**
- A `fuzz/` crate outside the workspace, on a pinned nightly.
- Targets: control frames, pairing, input records, audio datagrams, discovery, notify messages, video records, the camera frames-file header, `ipc::decode`, clipboard DIB, and the `peers.txt` and settings parsers.
- A smoke run on every push, and longer runs nightly.

### M2 Review sweep (every remaining finding)
- **Security:**
  - **S5:** the audio helper moves to the debug source set behind a no-op release stub. libadb, Conscrypt and BouncyCastle become `debugImplementation`, so JitPack leaves the release graph (−7 MB). A one-time cleanup removes it from old installs (`AdbBridge.forget`).
  - **S9:** the clip fingerprint becomes an HMAC under a Keystore key.
  - **S11:** the pipe name uses the SID, through `ipc.rs` `user_sid()`.
  - **S12:** toasts and balloons hide the text by default. No package names in `tray.log` or logcat. Android logs move to internal storage.
- **Android:**
  - **A4:** full-display capture config (API 34+), so clicks map correctly.
  - **A5:** opt in to background activity starts when sending PendingIntents, and give the PC an honest result ("Tap the notification on your phone").
  - **A6:** declare `READ_MEDIA_VISUAL_USER_SELECTED`, with guidance for partial access.
- **Bugs:** B3, B6, B7, B8, B10 and B13, plus:
  - **B9:** keep only what Relay uses from each notification; no bitmaps.
  - **B11:** rank pairing addresses: the real LAN first, virtual adapters last.
  - **B12:** fall back to an ephemeral QUIC port, and show in the status when the discovery port is taken.
  - **B14:** lift the speaker silencer while an alarm, ringtone or emergency sound plays *(verify)*.
- **QoL:** "Keep Control on between sessions", opt-in, offered only now that M1 #6 exists.
- **Engineering:**
  - Fewer idle wakeups: logs are sampled only while a session or the UI is active, and redraws happen only on change.
  - The screen encoder reuses its buffer.
  - The wakeup budget is recorded in DECISIONS.

### M3 Foundations
- **Protocol:**
  - `Hello` gains `min_compatible` and `app_version`. An old peer decodes them as 0 or empty, so it stays compatible.
  - Two peers are compatible when each side's `min_compatible` is at most the other's version.
  - "Incompatible" names the device to update.
  - Each M7–M9 feature gets a capability.
- **Peer IDs:** in every event and allow-list. This removes the name→ID mapping (`uniquePeerId`).
- **Settings:** versioned, with tested migrations.
  - PC: `settings` plus a new per-device key=value file.
  - Android: a prefs version with stepwise migrations.
  - New installs use "Relay" folder names; existing paths are kept.
- **B5:** when the Keystore has been reset, the identity can't be unsealed. Relay stops retrying every 15 min and offers "Reset and pair again".
- **Done when:** a transport test proves that a 0.2.0-style peer and the new build still talk.

### M4 Design system and Floot prototypes
- **Research:**
  - At least 3 real references per screen family:
    - Windows: Settings, Phone Link, Quick Share for Windows, PowerToys, Fluent 2.
    - Android: Pixel Android 16 Settings and Quick Settings, Files by Google, Quick Share, LocalSend, Nothing OS.
    - Glass: iOS 26 and macOS Tahoe, including Liquid Glass's readability lessons.
  - Each decision cites at least 2 sources: a guideline and a real app. A fresh reviewer checks the citations.
- **Anti-AI-look checklist:**
  - no generic purple-blue gradients, emoji icons, or same-size rounded cards everywhere;
  - real copy;
  - a 4/8-pt grid with optical alignment;
  - one accent colour;
  - hierarchy from size and weight;
  - motion only where it means something.
- **Tokens:**
  - Materials per theme and mode: tint, opacity, blur, highlight, border alpha, grain.
  - Elevation, radii, spacing and type scale.
  - Springs (stiffness and damping), plus Windows easing curves and durations.
  - Modes: light, dark or system; reduced transparency; reduced motion; lite.
  - `gen-tokens` emits Rust, Kotlin and CSS, plus WCAG tests over white and over black.
- **Floot:**
  - Rewrite the design principles.
  - Prototype every PC and Android screen, including those M7–M10 add, in light and dark and in both themes, with motion demos.
  - Playwright screenshots, scored by a fresh design reviewer.
  - You get the preview link when this starts; nothing waits for you.

### M5 PC native redesign
- **Materials:** Native glass uses Mica on the main window and Acrylic on the flyout, menus and dialogs. Relay glass is key choice 2.
- **Motion:** springs in `ease.rs`, frame-driven only while something animates, and off when Windows animations are off.
- **Screens:**
  - tray flyout;
  - main window: Home, Devices, Transfers, Timeline, Settings;
  - pairing and QR;
  - consent dialogs;
  - screen-window chrome;
  - toasts.
- **Narrator:** UIA for the quick panel, extending `uia.rs`.
- **CI snapshot job:**
  - A `test-hooks` build renders every surface offscreen with Direct2D over a fixed wallpaper.
  - Both themes, light and dark.
  - Fails fast on missing fonts.
- **Budgets:** re-measured and recorded.

### M6 Android native redesign
- **Themes:** Native glass is Material 3 with dynamic colour. Relay glass is a `GlassSurface`.
- **Settings → Appearance:** theme; light, dark or system; reduce transparency; lite mode.
- **Blur:**
  - Backdrop blur via Haze (Apache-2.0, latest stable, pinned, in `verification-metadata.xml`, with a DECISIONS entry) on API 31+ outside lite mode. Below API 31, a tinted scrim.
  - If blurred frames run over budget, Relay drops to lite on its own.
- **Motion:** Expressive motion only from a stable material3; otherwise the token springs.
- **Lite mode:**
  - Turns on automatically for any of: `isLowRamDevice`, under 3 GB RAM, Android below 12, battery saver, or animations off. There's also a manual toggle.
  - It means no blur, short fades only, and nothing animating while idle.
- **Screens:**
  - Home, device picker, Transfers, Timeline;
  - Remote and touchpad, Settings;
  - consent sheets, pairing and SAS;
  - widget, quick panel, tile;
  - per-maker background guidance (Xiaomi, Samsung, Huawei, Oppo/realme).
- **Tests:**
  - Roborazzi JVM screenshots of every screen, in both themes and every mode, which I inspect.
  - A CI emulator smoke job on API 29 and 36: start-up, every screen, PSS recorded. Informational at first.

### M7 Security shield
- **Unlock gate:**
  - **Android:** API 30+ uses STRONG or the device credential; API 29 uses WEAK or the credential. It can't be turned on without a screen lock.
  - **PC:** Hello through the interop on Windows 11. Elsewhere it's unavailable, and the reason is shown.
  - **It guards:**
    - allow decisions;
    - "always allow" and allow-list changes;
    - auto-accept;
    - confirming a pairing;
    - unpairing;
    - turning the gate off.
- **"Disconnect everything"** (tray, flyout, tile, notification, Home):
  - closes every session;
  - stops every share;
  - turns every "always allow" off;
  - pauses Relay until you resume it.
- **Keyed discovery (PBD2):**
  - Key: `k = HKDF-SHA256(X25519(own, peer), salt "pb-discovery/2", info = sorted ids)`. An all-zero DH result is rejected.
  - Probes, one per peer and padded to at least 4: a 16-byte random nonce plus `HMAC(k, "probe"‖window‖nonce‖port)[..16]`.
  - Answer: `HMAC(k, "answer"‖window‖probe nonce‖own nonce‖port‖addresses)`. Only MACed addresses are dialled.
  - Once a peer has announced `discovery2`, PBD1 is no longer sent to it or answered from it. PBD1 is removed in the next minor release.
  - A fuzz target.
- **Pairing:**
  - Bad tokens are limited per source, with a global cap, instead of 3 attempts from anyone (`pairing.rs:24`).
  - At most 2 handshakes at once, each time-boxed.
  - Per-peer quotas for sessions and offers (THREAT_MODEL row 14).

### M8 Several devices
- **Picker:** wherever a send starts (the M1 share sheet, Send to, sending the clipboard).
- **Devices:** a default device, renaming, and status per device. The tile and the widget act on the default; long-press opens the picker.
- **Per-device permissions:** control, screen, camera, audio, clipboard sync, notifications, auto-accept.
  - A DECISIONS entry supersedes `DECISIONS.md:428` ("No master switch"). With several PCs, "never ask from this PC" is a different decision from "no".
- **Migration:** today's settings become the first PC's settings.

### M9 Transfers that resume
- **Resume:**
  - `Offer.resume_token`; `OfferReply.resume_from` with a prefix hash (capability `resume`).
  - Partial files are kept for 24 h at most. They have caps per peer and on total size against free space, and a persisted token map.
  - Resume works across reconnects, in both directions. After a restart, it works only where the sender can still read the source.
- **Queue:** per device, with pause, resume and cancel.
- **History:**
  - Searchable on both apps.
  - Transfers are listed whenever `keep_history` allows.
  - Clipboard history is opt-in: encrypted, kept for 7 days, and sensitive clips are never stored.
- **Drag-and-drop:** onto the flyout, the main window and the screen window; on Android, on large screens.

### M10 Stay current and trusted
- **Opt-in update check:**
  - It reads a manifest from GitHub Releases (version, date, SHA-256 of each artifact), signed with an offline Ed25519 key you hold.
  - It never offers a version at or below the installed one.
  - The manifest can announce the next key, signed by the current one.
  - The PC swaps itself on restart; Android opens the installer or the release page.
- **Windows code signing:** works with any signtool-compatible provider, from secrets. Skipped cleanly when the secrets are absent.
- **Installer:** Inno Setup, with a firewall rule for `Relay.exe` on Private networks. The portable zip stays. Both detect a Public network and guide you.
- **Also:**
  - A Windows ARM64 build (the camera DLL is untested there).
  - A CycloneDX SBOM.
  - Third-party notices: a `NOTICE` file and an in-app screen.
  - A least-privilege publish job.

### M11 Docs, checklist, final review
- **THREAT_MODEL:** new rows for the gate, "Disconnect everything", PBD2, update signing and clipboard history. Drift fixed: flavors, Instant screen, Opus.
- **Other docs:** WIRE_PROTOCOL, DECISIONS, CHANGELOG 0.3.0, README.
- **`docs/DEVICE-TEST-CHECKLIST.md`** for Claude on your desktop:
  - setup, including turning the gate off with one unlock;
  - checks per milestone, including Android 15/16/17 steps and a gate section;
  - measurements;
  - expected results, and how to report.
- **Final:** a whole-branch review, then push. No PR.

## Deferred on purpose
| Item | Why not now |
|---|---|
| A7: Wi-Fi lock behaviour on Android 14+ | Measure on a device first (checklist), then decide |
| A11: developer verification for sideloading | Needs your Google developer registration. The README explains the install flow |
| A12: on Android 10–12, typing rewrites the whole field | Platform limit: the accessibility input method needs API 33 |
| Self-hosted relay for use outside the LAN | A new subsystem and a threat-model change |
| mDNS/NSD discovery | Comes after PBD2, if device tests show broadcasts being filtered |
| Single-app screen share | A feature of its own. M2 forces full-display capture so clicks map correctly |
| Binding consent to a Keystore key | The next security step after the gate |
| Translations; reproducible builds | Strings move to resources now; the SBOM ships in M10 |

## Verification (the quality bar)
- **Per task:**
  - TDD: the test fails first, then passes.
  - Rust 1.99.1: `fmt`, `clippy -D warnings`, `test`.
  - Android: `testDebugUnitTest`, `lintRelease`, `assembleQa` and Roborazzi, here if the SDK installs, otherwise in CI.
- **Per milestone:**
  - CI green on windows-latest and ubuntu-latest:
    - Rust;
    - Android unit tests, lint, `qa` build, 3 APKs;
    - the zip;
    - `cargo-deny`;
    - fuzz smoke;
    - the planting test (from M1);
    - snapshots (from M5);
    - the emulator smoke job (from M6).
  - Plus a milestone review.
- **At the end:** a review of the whole branch. Device behaviour goes through the checklist.

## Risks
- **Size:** this is weeks of human work. The ledger keeps it resumable, and you hear about progress after every milestone.
- **No device here:** OEM biometrics, DWM quirks and blur performance only show up in the checklist run.
- **Signing:** code signing and release signing need your certificate and key. The pipeline is ready but stays idle until then.
- **Network:** the Android SDK download may be blocked here. CI covers that case.
- **Library timing:** material3 1.5 and Haze 2 may not be stable in time. Nothing in this spec depends on them.

## Critical files and reuse
- **PC:** `crates/pb-tray/src/`:
  - `main.rs`, `core.rs`, `ui.rs`, `settings.rs`;
  - `ipc.rs`: reuse `user_sid()` and the SDDL in `owner_only()`;
  - `camera_register.rs`: reuse `ensure_trusted_dir`, `set_dacl`, `is_trusted_owner`;
  - `portable.rs`, `fluent.rs`, `ease.rs`, `strings.rs`, `uia.rs`, `tokens.rs`.
- **Core:**
  - `pb-protocol/src/lib.rs`: reuse `Hello::supports` and the `CAP_AUDIO` pattern.
  - `pb-transport/src/`:
    - `lib.rs`;
    - `discovery.rs`: keep the rule that an address stays a candidate until a peer is authenticated through it;
    - `files.rs`;
    - `pairing.rs`: reuse `ct_eq` and `Ticket`;
    - `notify.rs`.
  - `pb-identity/src/{lib,vault}.rs`, `pb-ffi/src/lib.rs`.
- **Android:**
  - `PbService.kt`, `ScreenShare.kt`, `EntryPoints.kt`, `Quick.kt`, `NotifyListener.kt`, `Recovery.kt`, `AudioPlayer.kt`, `SpeakerSilencer.kt`;
  - the `*ConsentActivity.kt` files, `AdbBridge.kt` (reuse `forget`), `ui/`;
  - `build.gradle.kts`, `AndroidManifest.xml`, `gradle/verification-metadata.xml`.
- **Design and CI:** `design/tokens.toml`, `tools/gen-tokens`, `.github/workflows/*.yml`, `scripts/*.ps1`, `deny.toml`.

## Sources for the cross-check
- [IUserConsentVerifierInterop::RequestVerificationForWindowAsync (Microsoft Learn)](https://learn.microsoft.com/en-us/windows/win32/api/userconsentverifierinterop/nf-userconsentverifierinterop-iuserconsentverifierinterop-requestverificationforwindowasync)
- [Show a biometric authentication dialog (Android Developers)](https://developer.android.com/identity/sign-in/biometric-auth)
- [/DEPENDENTLOADFLAG (Microsoft Learn)](https://learn.microsoft.com/en-us/cpp/build/reference/dependentloadflag) and [the 2020 Full Disclosure report](https://seclists.org/fulldisclosure/2020/Jan/48)
- [NotificationListenerService.Ranking](https://developer.android.com/reference/android/service/notification/NotificationListenerService.Ranking) and [the AOSP change marking hasSensitiveContent as SystemApi](https://gitlab.e.foundation/e/os/android_frameworks_base/-/commit/2a5a56b7111bfa55e523218a2097eeeb3b7d6398)
- [Compose Material 3 releases](https://developer.android.com/jetpack/androidx/releases/compose-material3) and [a project tracking the Expressive APIs' removal from 1.4.0](https://github.com/seijikohara/femto-car-launcher/issues/427)
- [Haze (Apache-2.0)](https://chrisbanes.github.io/haze/latest/)
