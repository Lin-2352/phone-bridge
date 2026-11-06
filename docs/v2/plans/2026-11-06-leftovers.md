# Leftovers before M4

Part A of the plan the owner approved on 2026-11-06 (the plan covers M4 to M11 as well). Small tasks that need no phone, each with a test seen failing first.

| Task | What | Where |
|---|---|---|
| L1 | Home says when a VPN may block the PC (only when a PC is paired and reaching it failed) | `ui/home/Attention.kt` (`vpnHint`, `Need.VpnOn`), `Net.kt` (`vpnActive`) |
| L2 | A phone-audio share with no frame for 1.5 s says "Reconnecting…" on the PC flyout; the phone's notice says it when the PC session is gone | `audio_state.rs` (`STALL_MS`, `reconnecting`), `audio.rs` (`stalled`, `StallClock`), `flyout_tiles.rs`, `AudioConsent.kt` (`sessionDetail`) |
| L3 | The PC hears "yes" after the speaker is silenced, or after 1.5 s, without waiting on the main thread; new installs log under `Relay\logs`; the volume session is `@Volatile` | `audio/AcceptOrder.kt`, `AudioShare.kt` (`completeLaunch`), `applog.rs` (`fallback_dir_for`) |
| L4 | Release `Relay.exe` in the portable folder in use (stop Relay, copy only the exe, restart) | no repo change |
| L5 | Docs and the owner-only list | `DECISIONS.md`, `PROGRESS.md` |

Done 2026-11-06: the AI-attributed web commits were rewritten (see PROGRESS notes). Left to the owner: the earphones test, the audio delay choice, the paused-YouTube retest, and "Reset and pair again" after a real screen-lock change.
