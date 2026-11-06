# Relay v2: progress list

The single place that says what is done, what is in progress and what is left in the Relay v2 program. It is updated and pushed after every task, so a new session can continue from GitHub alone.

## Resume here
- **Branch:** `cloud/v2-upgrade` (cut from `main` at `456e480`). Never push to another branch; no pull request.
- **Last finished task:** M4.3 (materials, elevation, springs, easing, modes as tokens). Before it, L5 (Part A, leftovers, done; the portable `Relay.exe` in use was swapped for a `--release` build on 2026-11-06). Before it, M3.R: milestone M3 is done (the audio branch `feature/audio-full` was merged before M3.7 part 2, see the note "Audio merge" at the bottom). Plan: [plans/2026-11-05-m3-foundations.md](plans/2026-11-05-m3-foundations.md)
- **Next task:** M4.4 (CSS output and WCAG tests in the generator; the AA tests decide the material opacities, per RESEARCH.md: text surfaces 0.70 or more, icon-only 0.50 or more). Plan: `plans/2026-11-06-m4-design-system.md`. `audio_is_current_again_right_after_a_blackout` can fail under a loaded full-workspace run on this PC (passes alone and in CI); it is timing-based.
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
- [x] M1.3 OTP redaction on Android 15+ (A2): detect the placeholder; PC "Code hidden by Android"; Settings explains; more `find_otp` languages
- [x] M1.4 Restricted settings and Advanced Protection (A3, A10): guided card from `packageSource`; explain Advanced Protection
- [x] M1.5 Tapjacking and self-injection (S3, S8, I6): hide overlays; obscured-touch refusal; data-sensitive views; pause Relay's own remote input over every prompt; the quick panel shares only on tap
- [x] M1.6 Locked phone (S4): no auto-accept while locked; input paused; on API 29–30, Allow opens the activity
- [x] M1.7 Mark-of-the-Web (S2): `Zone.Identifier` ZoneId=3; risky types always prompt; shortcut types refused by default
- [x] M1.8 Portable folder trust (S7): `data\` DACL; app-folder check and fix; no elevation from unsafe folders; embedded camera DLL; DLL-planting fix and CI planting test
- [x] M1.9 Saving received files on its own executor (B1)
- [x] M1.10 Android 17 audio hardening (A8): restore volume only under a foreground service, otherwise defer
- [x] M1.11 Local network permission (A9): declare it; a helper for target 37
- [x] M1.12 Release hygiene (S6): `test-hooks` feature and an exe scan in CI; `relay-cli.exe` out of the zip; the release APK fails without its key
- [x] M1.13 Fuzzing: `fuzz/` (nightly-2026-11-01, cargo-fuzz 0.13.2) with 10 targets (control frames, input datagrams, pairing, discovery packets, file names, notification text, video config, audio packets, camera frames file, peers.txt) and the fuzz-smoke CI job; the PC binary's settings and pipe parsers get randomized in-crate tests instead (cargo-fuzz can't link a binary); no DIB target (Windows decodes clipboard images, not our code)
- [x] M1.R Milestone review and CI green — reviewer found 2 critical (screen-window taps skipped the lock/prompt pause; the screen and audio consent windows still auto-accepted while locked) and 1 important (`--test-audio-synth` in release): all fixed. Deferred minors: the folder check reads neither the folder's owner nor its parent folder's ACL; the DLL planting check covers start-up imports only

## M2 Review sweep (every remaining finding)
- [x] M2.1 S5: audio helper to the debug source set with a no-op release stub; its libraries `debugImplementation`; one-time cleanup (`AdbBridge.forget`)
- [x] M2.2 S9: clip fingerprint as HMAC under a Keystore key
- [x] M2.3 S11: pipe name from the user SID
- [x] M2.4 S12: hide toast and balloon text by default; no package names in `tray.log` or logcat; Android logs in internal storage
- [x] M2.5 A4: full-display capture config (API 34+)
- [x] M2.6 A5: background-activity-start opt-in for PendingIntents; honest result to the PC
- [x] M2.7 A6: `READ_MEDIA_VISUAL_USER_SELECTED` and partial-access guidance
- [x] M2.8 B3: link check matches what the phone opens
- [x] M2.9 B6: `startArmed` encoder left running
- [x] M2.10 B7: main-thread sleeps in `SpeakerSilencer` and `PhoneSpeaker`
- [x] M2.11 B8: downsampled bitmap decode in `ClipImages.toPng`
- [x] M2.12 B9: keep only what Relay uses from each notification
- [x] M2.13 B10: audio focus and `AUDIO_BECOMING_NOISY` in `AudioPlayer`
- [x] M2.14 B11: rank pairing addresses (real LAN first)
- [x] M2.15 B12: ephemeral QUIC port fallback; status when the discovery port is taken
- [x] M2.16 B13: `teardownCapture` lock/join race
- [x] M2.17 B14: lift the speaker silencer while an alarm, ringtone or emergency sound plays
- [x] M2.18 QoL: "Keep Control on between sessions" (opt-in)
- [x] M2.19 Fewer idle wakeups; new wakeup budget recorded in DECISIONS
- [-] M2.20 Screen encoder reuses its frame buffer — dropped: uniffi copies the whole `ByteArray` into Rust on every `sendFrame` and takes only exact-size arrays, so a reused larger array would copy more, not less. The device checklist measures memory churn while mirroring instead
- [x] M2.R Milestone review and CI green — reviewer found 2 important (a queued speaker-silence job could run after the service's final restore, leaving the phone silenced with Relay off; a Keystore key that loads but fails in use crashed the app on focus) and 4 cheap minors (an alarm already ringing at share start stayed silenced; link toasts hid the site too; a bound-but-idle Control service let the PC open apps without a tap; a visible Relay screen didn't count): all fixed. Deferred minor: the fallback QUIC port changes on each start (discovery and the PC dialling the phone still recover); device check whether `activePlaybackConfigurations` lists idle alarm players

## M3 Foundations
- [x] M3.1 `Hello.min_compatible` and `app_version`; compatibility rule; "update ‹device›" message
- [x] M3.2 Capabilities defined for every M7–M9 feature
- [x] M3.3 Peer IDs in every event and allow-list (drop the name→ID mapping)
- [x] M3.4 PC settings versioning and a per-device key=value file, with migrations
- [x] M3.5 Android prefs versioning with stepwise migrations
- [x] M3.6 "Relay" folder names for new installs (existing paths kept)
- [x] M3.7 B5: Keystore-reset recovery ("Reset and pair again"): `pb-identity` reset and `PbError::IdentityUnreadable` (part 1); Android `startFailure`/`shouldRecover`, `Need.IdentityLost` first, status line, one notification, `IdentityReset.kt`, Home card with a confirmation (part 2)
- [x] M3.8 Transport test: a 0.2.0-style peer and the new build still talk (`a_0_2_0_peer_still_talks`; `HelloV020` round trips in `pb-protocol`)
- [x] M3.R Milestone review and CI green (1 Important and 2 upgraded Minors fixed test-first; see the note "M3 review")

## Part A: leftovers before M4 (plan: plans/2026-11-06-leftovers.md)
- [x] L1 VPN hint on Home (`vpnHint`, `Need.VpnOn`, `vpnActive`)
- [x] L2 "Reconnecting" while audio is stalled (PC flyout and phone notification)
- [x] L3 Deferred M3 minors: accept off the main thread, `Relay\logs` fallback, volatile volume session
- [x] L4 Release `Relay.exe` into the owner's `dist\Relay` (stop Relay, copy only the exe)
- [x] L5 Part A wrap-up and docs

## M4 Design system and Floot prototypes
- [x] M4.1 Research pass: at least 3 real references per screen family, 2 sources per decision, citation review
- [x] M4.2 Anti-AI-look checklist written into the design principles
- [x] M4.3 `design/tokens.toml`: materials per theme and mode, elevation, radii, spacing, type, springs, Windows easing; modes
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
- 2026-11-05 (same session, later): M1 security items done and pushed: M1.1, M1.2, M1.3, M1.4, M1.5, M1.6, M1.7, M1.9, M1.10, M1.11, M1.12, and part 1 of M1.8 (data folder ACL). CI was green on every job at b0b8528; later commits were checked here (Linux tests, Windows type-check and clippy through x86_64-pc-windows-gnu, Android unit tests and lint). Some PC behaviour is Windows-only and is covered by CI's windows job; check the latest run before going on.
- Rulings and deviations are listed per task in the plan's ledger style in the commit messages; the most important: the screen lock reason travels in the existing screen notice (no wire change), the redacted-code text is replaced on the phone (no wire change), allow_shortcut_files has no settings row yet (M5 adds it).
- 2026-11-05 (end of session): M1.8 is down to part c-2. Before going on, check the latest CI run of cloud/v2-upgrade (the Windows jobs run tests that can't run in a Linux container). The ledger for M1 lives in the git-ignored .superpowers/ folder of the old container; everything that matters is in this file and in the commit messages.
- 2026-11-05 (wrap-up): CI run 41956488616 green on every job, fuzz-smoke included. The DLL planting check passed with /DEPENDENTLOADFLAG:0x800 and SetDefaultDllDirectories alone: Relay.exe started next to 34 garbage DLLs named after the libraries it uses, so no delay-loading was needed (ruling). The check is now required. If Relay ever imports a new non-system DLL, add its name to the check's list in ci.yml. All six fuzz targets also ran here without a crash (0.3–4.9 million runs each).
- 2026-11-05 (M1 closed): M1.13 has 10 fuzz targets plus randomized tests for the PC's settings and pipe parsers. The M1 review's critical findings are fixed (see M1.R). docs/THREAT_MODEL.md has a short list of what M1 added. Check the latest CI run first: the Windows-only tests (ipc random text, settings pages) run only there.
- 2026-11-05 (M2 start): CI run 41969323722 on e4f5f12 green on every job. M2 plan written. No wire change in M2. Rulings in the plan: PC toasts hide text by default (`toast_show_text`); the PC-away status shows an absolute time so it needs no timed redraw; M2.20 dropped (reason above).
- Device checklist items collected for M11.5: (M2.1) on a phone that once ran the audio helper's setup, the device-idle allow-list entry stays until removed over adb (`adb shell cmd deviceidle whitelist -dev.phonebridge.app`); (M2.20) measure memory churn while mirroring; (M2.17) set an alarm during a phone-audio share with the speaker silenced: the alarm must be heard, and silence must return after it; (M2.19) an hour idle with no PC and the screen off, then `adb shell dumpsys batterystats`: Relay's wakeups stay within one per 15 min.
- 2026-11-05 (M2, mid-milestone): a new PC settings toggle must also go into `settings_groups.rs` (OPTIONS and `page_of`; at most 9 rows a page) and the KEYS count in `pages/settings.rs`. `settings_groups` now builds and tests on Linux; `pages/settings.rs` tests still run only on CI's Windows job.
- 2026-11-05 (M2 closed): CI run 41983048190 on 6096cb4 green on every job (Rust on Linux and Windows, Android, APKs, fuzz smoke, zip, deny). Rulings in M2 commit messages: M2.10 keeps onDestroy's restore on its own thread; M2.16 keeps the join under the lock but the capture thread never takes it; M2.19 keeps the relative "5 min ago" wording (Home and the PC use it) and gates redraws on the screen being on.
- 2026-11-05 (M3 start): M3 plan written. Rulings in the plan: the protocol version stays 0 and `Hello` gains optional fields 4 and 5; reserved capabilities (`discovery2`, `resume`) are advertised only by the build that implements them; the PC gets peer ids in its request events with M8's per-device permissions, not now; an old global "allow" switch no longer covers PCs paired later (M3.5).
- 2026-11-05 (M3 progress): M3.1-M3.6 done and pushed, M3.7 part 1 done. CI was green on every earlier M3 push that finished when last checked; check the latest run first. Rulings so far (also in commit messages): `Hello` fields 4 and 5 are optional and the protocol version stays 0; a 426 close carries `update:you` or `update:me`; capabilities `discovery2` and `resume` are reserved, advertised only by the build that implements them; every phone event with `peer_name` now also has `peer_id` (the PC keeps name-keyed events until M8); the Android global `allow_control`/`allow_screen` switches are still folded into the paired PCs at every start by `Prefs.migrateAllowances` (Settings writes them on purpose while the node is off), not by the version step; `settings.txt` starts with `version=1`, keeps unknown keys, and an existing install gets `Downloads\Phone Bridge` pinned once; new installs use `%LOCALAPPDATA%\Relay\tray` and `Downloads\Relay`; the Run value name, AUMID, pipe, camera folder and notification channel ids are never renamed.
- **M3.7 part 2 (what is left of B5, Android only).** Failing tests first, one commit `M3.7: ...`. (1) New pure functions with JUnit tests: `enum class StartFailure { RETRY, KEYSTORE_RESET }` and `fun startFailure(e: Throwable): StartFailure`: `KEYSTORE_RESET` only when `e` is `PbException.IdentityUnreadable` AND its reason contains `AEADBadTagException`, `sealed blob too short` or `32-byte seed` (a transient keystore error such as `KeyStoreException` must stay `RETRY`, or Relay would stop retrying wrongly); `fun shouldRecover(enabled: Boolean, running: Boolean, identityLost: Boolean)` (never recover while the identity is lost); `Need.IdentityLost` placed FIRST in `needs(...)` in `ui/home/Attention.kt` (add a parameter `identityLost: Boolean` and update `AttentionTest` and the call in `HomeTab.kt`); `relayStatus(..., identityLost)` in `StatusLine.kt` saying "Relay can't open its keys" (update `StatusLineTest`). (2) `Prefs.identityLost: Boolean` (key `identity_lost`). In `PbService.kt` in the `catch (e: Throwable)` of the node start (around line 487): if `startFailure(e) == KEYSTORE_RESET`, set `identityLost`, call `cancelRecovery(this)`, post ONE notification (modelled on `notifyStopped` in `Recovery.kt`) whose tap opens `MainActivity`, then `stopSelf()`; otherwise keep today's behaviour. Log the `StartFailure` value instead of the class name. In `Recovery.kt` `ensureRunning`, use `shouldRecover`. (3) `IdentityReset.kt` with `fun resetIdentity(ctx: Context)`: make sure Relay is off (`setBridgeEnabled(ctx, false)`), call the generated Kotlin function `uniffi.pb_ffi.resetIdentity(ctx.filesDir.absolutePath)`, delete the Keystore alias `phone_bridge_identity_v1` (add `KeystoreWrapper.forget()` like `ClipFingerprint` does with its alias), clear the allow lists with `Prefs.dropAllowancesExcept(emptyList())`, clear `lastPcName`, `lastSeenMs` and `identityLost`, then `setBridgeEnabled(ctx, true)` and open `ScanActivity`. (4) UI: the Home "Needs attention" card gets the `IdentityLost` row with a "Reset and pair again" action behind a confirmation dialog (model it on `forgetPrompt` in `ui/home/ForgetPc.kt`; its text must say the PCs will see this phone as a new device, so remove it on each PC, then pair again). Never delete the whole data folder: only what `resetIdentity` removes. Then run the Android checks, commit, push and tick M3.7 in this file.
- **After M3.7:** M3.8 is the transport test that a Relay 0.2.0-style peer still talks (see the plan; hand-encode a `Hello` with only fields 1-3, as the existing test around `a_version_mismatch_is_reported_as_one_not_as_a_refused_pairing` does). Then M3.R: one reviewer agent for the whole M3 diff against the plan's Review Focus, fix the serious findings test-first, update `docs/THREAT_MODEL.md` (keystore reset; `devices.txt` limits) and `docs/DECISIONS.md` (settings versions, folder names, keystore reset), make sure CI is green, then tick M3.R and write the M4 plan.
- **Audio merge (2026-11-06).** The owner's local branch `feature/audio-full` (phone-to-PC audio: silence for all sound, the volume keys setting the PC level in every app, Wi-Fi stall handling, PC playout queue) was merged into this branch before M3.7 part 2, on the same base `456e480`. Rulings are in the merge commit `a6a395a`. Points that later tasks must keep: `PhoneSpeaker.begin` runs on `VolumeJobs` but the share waits for it (`runAndWait`) so the phone is silent before the PC hears yes; an alarm or ringtone counts as a route player while the silencer is lifted (B14); `shouldSwitchOff` takes both `keepOn` and `holdingVolumeKeys`; the node start keeps the audio branch's retry loop, and M3.7 stops it only for `StartFailure.KEYSTORE_RESET`. The merged build passed a device check on a realme RMX3561 (silence, volume keys in YouTube and Clash Royale, exact volume restore). Device items for `DEVICE-TEST-CHECKLIST.md`: "Reset and pair again" after a real screen-lock change (no way to force a Keystore loss here).
- **M3 review (2026-11-06).** One reviewer read the whole M3 diff and the audio merge against the plan's Review Focus. Fixed, each test-first: an old PC install that had never written `settings.txt` (0.2.0 wrote it only on a change) moved to `Downloads\Relay`: a missing file is now written at once, with today's defaults on a new install and migrated from version 0 on an old one (`old_install_that_never_saved_keeps_phone_bridge`, `new_install_keeps_relay_on_the_second_start`; the legacy `identity.key` also marks an existing install); the lost-identity flag is cleared by any start that opens the identity (`identityLostAfter`); opening the app or sharing to it no longer restarts a service whose identity is lost (`startsOnOpen`; the share sheet says to open Relay). Deferred minors: a phone-audio share waits up to 1.5 s on the main thread for the speaker to go silent (`AudioShare.launch`, from the consent screen); the log fallback folder is still `%LOCALAPPDATA%\PhoneBridge\logs` on new installs; `PcVolumeKeys.session` is read without `@Volatile`; `startNodeOnce` throwing after `startNode` returned could leave a node holding the port (unconfirmed). The eight web-session M3 commits (`3dbe031`..`5591b9e`) carry an AI author and trailers; they were already pushed and are left for the owner to decide.
