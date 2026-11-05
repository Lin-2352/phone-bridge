# Relay v2: progress list

The single place that says what is done, what is in progress and what is left in the Relay v2 program. It is updated and pushed after every task, so a new session can continue from GitHub alone.

## Resume here
- **Branch:** `cloud/v2-upgrade` (cut from `main` at `456e480`). Never push to another branch; no pull request.
- **Last finished task:** M1.12
- **Next task:** M1.7 (security items first: M1.7, M1.6, M1.5, M1.8, then M1.3, M1.4, M1.9, M1.10, M1.11, M1.13; plan: [plans/2026-11-05-m1-harden.md](plans/2026-11-05-m1-harden.md))
- **Read first, in this order:**
  1. this file;
  2. the spec: [SPEC.md](SPEC.md);
  3. the current milestone's plan in [plans/](plans/), when there is one.
- **Check the state:**
  - `git fetch origin cloud/v2-upgrade && git log --oneline -15 origin/cloud/v2-upgrade`;
  - `cargo test --workspace` on Linux;
  - the latest GitHub Actions run for the branch.
- **To continue in a new session**, tell the agent: *"Continue the Relay v2 program on branch `cloud/v2-upgrade`; start from `docs/v2/PROGRESS.md`."*

## How work is done (so every session works the same way)
- **Order:** follow the list below. Each milestone gets its own plan (superpowers:writing-plans), written when the milestone starts, against the code as it is then.
- **Tests first:** every task is TDD; see the test fail, then pass. Before each commit, run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`, plus the Android checks when the Android SDK is available here.
- **One commit per task:**
  - Its subject starts with the task ID (for example `M0.9: ...`), so `git log --grep "M0.9"` finds it.
  - The same commit ticks the task here and moves "Resume here".
  - Push after every task.
- **Budget mode:**
  - Tasks run inline (superpowers:executing-plans), with self-review.
  - A reviewer agent runs once per milestone (the `.R` task).
  - CI runs on every push.
- **Never:**
  - copy code from GPL or AGPL projects;
  - weaken "the PC can only ask";
  - skip or disable a test to get a green build;
  - put model names, e-mail addresses or session links in repository files.
- **This container:**
  - **Android checks here** (M0.3, about 5 minutes per fresh container):
    1. Install the Linux command-line tools from `dl.google.com/android/repository/commandlinetools-linux-13114758_latest.zip` into `~/android-sdk/cmdline-tools/latest`.
    2. Accept the licences, then run `sdkmanager "platforms;android-36" "build-tools;36.0.0" "platform-tools"`.
    3. Maven Central answers this proxy with HTTP 429, so add a Gradle init script `~/.gradle/init.d/central-mirror.init.gradle.kts` that swaps `repo.maven.apache.org` for `https://maven-central.storage-download.googleapis.com/maven2/`. It's container-only and never committed.
    4. `cargo build -p pb-ffi`, then generate the Kotlin bindings from `target/debug/libpb_ffi.so`.
    5. In `android/`: `ANDROID_HOME=~/android-sdk bash gradlew :app:testDebugUnitTest :app:lintRelease --no-daemon`.
  - No KVM, so no emulator; the NDK isn't installed here, so APKs come from CI.
  - CI has windows-latest and ubuntu-latest.
  - Device-only checks go into `docs/DEVICE-TEST-CHECKLIST.md` (M11.5), which Claude on the owner's desktop runs later.

Legend: `[x]` done · `[~]` in progress · `[ ]` to do · `[-]` dropped (with the reason)

## M0 Baseline (no change in behaviour)
- [x] M0.1 Branch `cloud/v2-upgrade` from `main`; cherry-pick the review doc; commit the spec and this list (in `docs/v2/`, because `.gitignore` excludes `docs/superpowers/`)
- [x] M0.2 Rust 1.99.1 here, matching CI
- [x] M0.3 Try the Android SDK here (Android checks fall back to CI if it can't be installed)
- [x] M0.4 `scripts/android-build.sh`, a bash twin of `android-build.ps1`
- [x] M0.5 CI: pin every action by commit SHA (`checkout`, `rust-toolchain`, `setup-java`, `cargo-deny-action`, `setup-oras`)
- [x] M0.6 CI: Android unit tests, lint and the `qa` (R8) build
- [x] M0.7 CI: APK artifacts (arm64-v8a, armeabi-v7a, x86_64) and the portable zip artifact
- [x] M0.8 `armeabi-v7a` ABI in Gradle and the build script; `armv7-linux-androideabi` in `deny.toml`
- [x] M0.9 Fix the `files.rs:462` 32-bit truncation (a remaining size that's a multiple of 4 GiB fails the send), with a test
- [x] M0.R Milestone review and CI green

## M1 Harden: the Top 12
- [x] M1.1 Clipboard and file injection (S1, B4, S10): Android confirm sheet; PC toast with Copy/Open; auto-copy off by default; `keep_in_history` honoured
- [x] M1.2 Lock-stop taken as a user Stop (A1, B2): `SystemStopped`; the PC shows "Phone locked" with Resume
- [ ] M1.3 OTP redaction on Android 15+ (A2): detect the placeholder; PC "Code hidden by Android"; Settings explains; more `find_otp` languages
- [ ] M1.4 Restricted settings and Advanced Protection (A3, A10): guided card from `packageSource`; explain Advanced Protection
- [ ] M1.5 Tapjacking and self-injection (S3, S8, I6): hide overlays; obscured-touch refusal; data-sensitive views; pause Relay's own remote input over every prompt; the quick panel shares only on tap
- [ ] M1.6 Locked phone (S4): no auto-accept while locked; input paused; on API 29–30, Allow opens the activity
- [ ] M1.7 Mark-of-the-Web (S2): `Zone.Identifier` ZoneId=3; risky types always prompt; shortcut types refused by default
- [ ] M1.8 Portable folder trust (S7): `data\` DACL; app-folder check and fix; no elevation from unsafe folders; embedded camera DLL; DLL-planting fix and CI planting test
- [ ] M1.9 Saving received files on its own executor (B1)
- [ ] M1.10 Android 17 audio hardening (A8): restore volume only under a foreground service, otherwise defer
- [ ] M1.11 Local network permission (A9): declare it; a helper for target 37
- [x] M1.12 Release hygiene (S6): `test-hooks` feature and an exe scan in CI; `relay-cli.exe` out of the zip; the release APK fails without its key
- [ ] M1.13 Fuzz crate (`fuzz/`, pinned nightly) with targets for every existing parser; smoke run in CI
- [ ] M1.R Milestone review and CI green

## M2 Review sweep (every remaining finding)
- [ ] M2.1 S5: audio helper to the debug source set with a no-op release stub; its libraries `debugImplementation`; one-time cleanup (`AdbBridge.forget`)
- [ ] M2.2 S9: clip fingerprint as HMAC under a Keystore key
- [ ] M2.3 S11: pipe name from the user SID
- [ ] M2.4 S12: hide toast and balloon text by default; no package names in `tray.log` or logcat; Android logs in internal storage
- [ ] M2.5 A4: full-display capture config (API 34+)
- [ ] M2.6 A5: background-activity-start opt-in for PendingIntents; honest result to the PC
- [ ] M2.7 A6: `READ_MEDIA_VISUAL_USER_SELECTED` and partial-access guidance
- [ ] M2.8 B3: link check matches what the phone opens
- [ ] M2.9 B6: `startArmed` encoder left running
- [ ] M2.10 B7: main-thread sleeps in `SpeakerSilencer` and `PhoneSpeaker`
- [ ] M2.11 B8: downsampled bitmap decode in `ClipImages.toPng`
- [ ] M2.12 B9: keep only what Relay uses from each notification
- [ ] M2.13 B10: audio focus and `AUDIO_BECOMING_NOISY` in `AudioPlayer`
- [ ] M2.14 B11: rank pairing addresses (real LAN first)
- [ ] M2.15 B12: ephemeral QUIC port fallback; status when the discovery port is taken
- [ ] M2.16 B13: `teardownCapture` lock/join race
- [ ] M2.17 B14: lift the speaker silencer while an alarm, ringtone or emergency sound plays
- [ ] M2.18 QoL: "Keep Control on between sessions" (opt-in)
- [ ] M2.19 Fewer idle wakeups; new wakeup budget recorded in DECISIONS
- [ ] M2.20 Screen encoder reuses its frame buffer
- [ ] M2.R Milestone review and CI green

## M3 Foundations
- [ ] M3.1 `Hello.min_compatible` and `app_version`; compatibility rule; "update ‹device›" message
- [ ] M3.2 Capabilities defined for every M7–M9 feature
- [ ] M3.3 Peer IDs in every event and allow-list (drop the name→ID mapping)
- [ ] M3.4 PC settings versioning and a per-device key=value file, with migrations
- [ ] M3.5 Android prefs versioning with stepwise migrations
- [ ] M3.6 "Relay" folder names for new installs (existing paths kept)
- [ ] M3.7 B5: Keystore-reset recovery ("Reset and pair again")
- [ ] M3.8 Transport test: a 0.2.0-style peer and the new build still talk
- [ ] M3.R Milestone review and CI green

## M4 Design system and Floot prototypes
- [ ] M4.1 Research pass: at least 3 real references per screen family, 2 sources per decision, citation review
- [ ] M4.2 Anti-AI-look checklist written into the design principles
- [ ] M4.3 `design/tokens.toml`: materials per theme and mode, elevation, radii, spacing, type, springs, Windows easing; modes
- [ ] M4.4 `gen-tokens` also emits Floot `base.css`, plus WCAG contrast tests
- [ ] M4.5 Floot "Phone Bridge design": rewrite the design principles; share the preview link
- [ ] M4.6 Floot PC screens (light/dark × both themes)
- [ ] M4.7 Floot Android screens (light/dark × both themes)
- [ ] M4.8 Floot motion demos
- [ ] M4.9 Playwright screenshots and a design review against the checklist and contrast rules
- [ ] M4.R Milestone review

## M5 PC native redesign
- [ ] M5.1 Materials: Mica, Acrylic, Relay glass, and fallbacks keyed on DWM failure, transparency off or high contrast
- [ ] M5.2 Springs in `ease.rs` (only while animating; off when Windows animations are off)
- [ ] M5.3 Tray flyout
- [ ] M5.4 Main window: Home, Devices, Transfers, Timeline, Settings
- [ ] M5.5 Pairing and QR
- [ ] M5.6 Consent dialogs
- [ ] M5.7 Screen-window chrome
- [ ] M5.8 Toasts
- [ ] M5.9 Narrator UIA for the quick panel
- [ ] M5.10 CI snapshot job (offscreen Direct2D, both themes, light and dark)
- [ ] M5.11 Memory and CPU budgets re-measured and recorded
- [ ] M5.R Milestone review and CI green

## M6 Android native redesign
- [ ] M6.1 Theme system (Native glass, Relay glass `GlassSurface`) and Settings → Appearance
- [ ] M6.2 Backdrop blur with Haze (API 31+), tinted scrim below that, frame-time fallback
- [ ] M6.3 Lite mode (automatic triggers and a manual toggle)
- [ ] M6.4 Motion: token springs (Expressive only from a stable material3)
- [ ] M6.5 Home
- [ ] M6.6 Device picker
- [ ] M6.7 Transfers
- [ ] M6.8 Timeline
- [ ] M6.9 Remote and touchpad
- [ ] M6.10 Settings
- [ ] M6.11 Consent sheets
- [ ] M6.12 Pairing and SAS
- [ ] M6.13 Widget, quick panel and tile
- [ ] M6.14 Per-OEM background guidance (Xiaomi, Samsung, Huawei, Oppo/realme)
- [ ] M6.15 Roborazzi screenshots of every screen (both themes, every mode)
- [ ] M6.16 CI emulator smoke job (API 29 and 36; PSS recorded)
- [ ] M6.R Milestone review and CI green

## M7 Security shield
- [ ] M7.1 Android unlock gate (BiometricPrompt per API level; `FragmentActivity` hosts)
- [ ] M7.2 PC unlock gate (Windows Hello interop on Windows 11; unavailable with a reason elsewhere)
- [ ] M7.3 Gate wired to every guarded action, with a 60 s grace
- [ ] M7.4 "Disconnect everything" in the tray, flyout, tile, notification and Home
- [ ] M7.5 PBD2 key derivation inside `pb-identity` (X25519 + HKDF; all-zero results rejected)
- [ ] M7.6 PBD2 packets: MACed probe and answer, only MACed addresses dialled, per-peer ratchet off PBD1, fuzz target
- [ ] M7.7 Pairing limits per source plus a global cap; 2 time-boxed handshakes at most
- [ ] M7.8 Per-peer session and offer quotas (THREAT_MODEL row 14)
- [ ] M7.R Milestone review and CI green

## M8 Several devices
- [ ] M8.1 Device picker on every send (share sheet, Send to, clipboard)
- [ ] M8.2 Default device, renaming, status per device
- [ ] M8.3 Tile and widget act on the default device; long-press opens the picker
- [ ] M8.4 Per-device permissions; DECISIONS entry superseding `DECISIONS.md:428`
- [ ] M8.5 Migration: today's settings become the first PC's
- [ ] M8.R Milestone review and CI green

## M9 Transfers that resume
- [ ] M9.1 Resume protocol: `resume_token`, `resume_from`, prefix hash, `resume` capability
- [ ] M9.2 Partial-file store: 24 h limit, caps, persisted token map
- [ ] M9.3 Send queue per device (pause, resume, cancel)
- [ ] M9.4 History: transfers under `keep_history`; clipboard history opt-in (encrypted, 7 days, no sensitive clips)
- [ ] M9.5 Drag-and-drop on the PC (flyout, main window, screen window)
- [ ] M9.6 Drag-and-drop on Android large screens
- [ ] M9.R Milestone review and CI green

## M10 Stay current and trusted
- [ ] M10.1 Signed release manifest (offline Ed25519 key, anti-rollback, key rotation) and the PC update check
- [ ] M10.2 Android update check
- [ ] M10.3 Provider-agnostic Windows code-signing step (skipped without secrets)
- [ ] M10.4 Inno Setup installer with a Private-network firewall rule for `Relay.exe`
- [ ] M10.5 Public-network detection and guidance (installer and portable)
- [ ] M10.6 Windows ARM64 build
- [ ] M10.7 CycloneDX SBOM (Rust and Gradle)
- [ ] M10.8 Third-party notices (`NOTICE` file and in-app screen)
- [ ] M10.9 Least-privilege publish job
- [ ] M10.R Milestone review and CI green

## M11 Docs, checklist, final review
- [ ] M11.1 THREAT_MODEL: new rows and drift fixed
- [ ] M11.2 WIRE_PROTOCOL
- [ ] M11.3 DECISIONS
- [ ] M11.4 CHANGELOG 0.3.0 and README
- [ ] M11.5 `docs/DEVICE-TEST-CHECKLIST.md` for Claude on the owner's desktop
- [ ] M11.6 Final whole-branch review
- [ ] M11.7 Final push (no pull request)

## Notes per session
- 2026-11-05: spec approved. M0.1 done.
- 2026-11-05: M0 done (CI run 41945588677 green on every job: Rust on Linux and Windows, Android tests and lint, qa APK for 3 ABIs, portable zip, deny). Windows code can be type-checked here: `apt-get install gcc-mingw-w64-x86-64`, `rustup target add x86_64-pc-windows-gnu`, `cargo check -p pb-tray --target x86_64-pc-windows-gnu`.
