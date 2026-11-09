# M9 Transfers that resume Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:executing-plans (inline, TDD, one reviewer at the end of the milestone). Steps use checkbox (`- [ ]`) syntax. Run `advisor` where a step says **ADVISOR** (M9.1 changes the file wire; M9.4 is encryption; also the `.R` task). Run several Monitors at once (cargo, gradle, CI by full SHA); never wait on one job alone.

**Goal:** a file transfer that breaks (Wi-Fi drop, app restart, sleep) continues from where it stopped, in both directions; each device has a send queue; transfers and, opt-in, clipboard clips can be searched in a history; files can be dropped onto the PC windows and onto large Android screens.

**Architecture:** the wire gains two optional offer fields and one optional reply field behind the already reserved `resume` capability. A new `PartialStore` in `pb-transport` owns kept partial files, their token map, caps and expiry; `files.rs` asks it instead of deleting partials. Queues, history and drag-and-drop live in the apps and only call pure functions that are tested on Linux.

**Tech Stack:** Rust 1.99.1, prost (pb-protocol), tokio (pb-transport), uniffi, Kotlin and Compose, Win32 OLE drop targets (`windows` 0.62), DPAPI, Android Keystore AES-GCM.

**Spec:** `docs/v2/SPEC.md` lines 325-335 and key line 81 ("The receiver proves its partial file by hash before the sender seeks"); rows M9.1-M9.R in `docs/v2/PROGRESS.md`; `docs/WIRE_PROTOCOL.md` lines 27 and 65-70; THREAT_MODEL rows 7, 14, 15.

## Global Constraints
- MIT OR Apache-2.0; no GPL/AGPL code.
- "The PC can only ask": resume never skips consent. A resumed offer is the same offer the phone already accepted only if its token matches a kept partial of that peer; otherwise it asks as a new file.
- `PROTOCOL_VERSION` stays 0; the new fields are optional, gated by `CAP_RESUME` (advertised only once implemented, DECISIONS.md:464). `a_0_2_0_peer_still_talks` stays green.
- Every parser keeps its size limits and gets a fuzz target.
- Budgets unchanged: Android PSS <= 80 MB, one idle wakeup per 15 min (history and queue add none); PC idle tray 0 CPU, <= 80 MB private.
- Never rename: package, Keystore alias, DPAPI entropy, pipe/mutex names, AUMID, notification channel ids, `filesDir` file names.
- New strings go in `strings.xml` / `strings.rs`, English. Every control has a Narrator/TalkBack name.
- One commit per task, subject starting with the task ID; the same commit ticks `docs/v2/PROGRESS.md` and moves "Resume here". Push after every task. No attribution lines.
- Gates: `cargo fmt --all`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace --no-fail-fast`; `(cd fuzz && cargo check --bins)`; Android: regenerate uniffi bindings, then `./gradlew :app:testDebugUnitTest :app:lintRelease --no-daemon --console=plain`. One cargo or gradle command at a time. CI by full SHA under a Monitor.

## Review Focus
1. **Partial file path or token abuse:** a paired peer cannot make Relay write outside the partial folder or continue another peer's partial. Tests: `partial_names_cannot_escape`, `token_of_another_peer_never_matches` (M9.2).
2. **A paired peer fills the disk:** caps hold against free space. Test: `caps_hold_against_free_space` (M9.2).
3. **The source file changed on the sender between attempts:** the prefix hash differs, the transfer restarts at 0, never splices. Test: `resume_with_wrong_prefix_restarts_at_zero` (M9.1).
4. **Files over 4 GiB on 32-bit Android:** offsets use u64 everywhere. Test: `offset_math_is_u64_on_32_bit` (M9.2).
5. **UI shows X, reality is Y:** queue state after process death, a "paused" row that is really sending, history after `keep_history` is turned off, an encrypted history whose key was lost. Tests: `queue_survives_restart_as_paused`, `history_with_lost_key_reads_as_empty_not_garbage` (M9.3, M9.4).

## Rulings already made
- Ruling: `resume_token` is 16 random bytes chosen by the sender per transfer; never derived from the offer id; token map keyed `(peer_id, token, name, size)`. Cost if wrong: one wire-field rename before release.
- Ruling: wire fields `Offer.resume_token = 7 (bytes, optional)`, `OfferReply.resume_from = 4 (u64)`, `OfferReply.prefix_sha256 = 5 (bytes)`. Receiver keeping a partial of `n` bytes replies `resume_from = n` and `prefix_sha256 = SHA-256(partial)`; the sender hashes its first `n` bytes: equal -> seek to `n`, send `FileHeader.offset = n` and the rest; different -> send `FileHeader.offset = 0` and the whole file, and the receiver truncates its partial. `FileHeader` gains optional `offset: u64 = 2` (absent = 0, so 0.2.0 behaviour is unchanged). `TransferDone.sha256` still covers the whole file: the receiver feeds the kept prefix into its hasher first.
- Ruling: partial caps 2 GiB per peer, 8 GiB total, never leaving less than `KEEP_FREE` (256 MiB) free; oldest partial evicted first; 24 h expiry; token map persisted as `resume.txt` (hex lines) next to the partials; orphan partials swept at start.
- Ruling: after an app restart a resume works only where the sender can still read the source: PC sends keep the path in the queue file; Android sends need a persistable URI permission, otherwise the row says "Needs the file again" and offers a re-pick.
- Ruling: queue pause/resume/cancel is local only (no new wire message); at most 64 queued items per device.
- Ruling: clipboard history is opt-in, default off; encrypted with DPAPI (PC) and an Android Keystore AES-GCM key; 7 days; clips marked sensitive or recognised as one-time codes are never stored; search decrypts in memory; turning `keep_history` off wipes it.
- Ruling: drag-and-drop goes through the M8 picker when `send_target = Ask`; Android "large screen" = window width >= 600 dp.
- Ruling: the open M8 follow-up "Explorer Send to, drag-drop and the screen-window file send go to the default phone without asking" is fixed by M9.5 (same call).

## Today in the code (checked 2026-11-08; M9.0a re-checks it)
- `crates/pb-protocol/src/control.rs:610-641`: `Offer` fields 1..6, `OfferReply` 1..3, `TransferDone`. `crates/pb-protocol/src/lib.rs:39-53`: `CAP_RESUME` in `RESERVED_CAPS`; `local_capabilities()` lists clipboard, audio, discovery2.
- `crates/pb-transport/src/files.rs`: `DEFAULT_MAX_FILE` :25, `BATCH` :36, offset fix :462, `KEEP_FREE` :587, partial path `.<name>.partial-<16 hex>` :704, `receive_bytes` :758; doc comment :59-60 says no partial is ever kept (must change).
- PC: `pb-tray/src/{core.rs,share.rs,settings.rs,timeline.rs,devices.rs,ui.rs}`, `pages/` (Transfers page exists from M5.4). Android: `EntryPoints.kt` (`ShareActivity` loops files one by one), `Timeline.kt`, `Prefs.kt` (`keep_history`), `ClipFingerprint.kt` (Keystore HMAC pattern), `KeystoreWrapper.kt`.
- `keep_history` exists on both apps (default true); PC `keep_in_history` (clipboard) default false.

## Shared interfaces (defined in M9.1, M9.2)
```rust
// pb-protocol: Offer { ..., resume_token: Vec<u8> /*7*/ }, OfferReply { ..., resume_from: u64 /*4*/, prefix_sha256: Vec<u8> /*5*/ }, FileHeader { id, offset: u64 /*2*/ }
// pb-transport/src/partials.rs
pub struct PartialStore { /* dir, entries, caps */ }
pub struct Claim { pub path: PathBuf, pub len: u64 }
impl PartialStore {
    pub fn open(dir: &Path, now: SystemTime) -> io::Result<Self>;
    pub fn claim(&mut self, peer: PeerId, token: [u8; 16], name: &str, size: u64, now: SystemTime) -> Option<Claim>;
    pub fn create(&mut self, peer: PeerId, token: [u8; 16], name: &str, size: u64, now: SystemTime) -> io::Result<PathBuf>;
    pub fn finish(&mut self, peer: PeerId, token: [u8; 16]);   // delete entry (success or cancel)
    pub fn sweep(&mut self, now: SystemTime, free_bytes: u64);   // expiry + caps
}
// pb-tray/src/queue.rs and Android devices/SendQueue.kt: pure queue
pub enum QueueState { Waiting, Sending, Paused, NeedsFile, Done, Failed }
pub struct SendQueue { /* per device */ }
```

---

### Task M9.0a: Refresh at start
- [ ] **Step 1:** Re-run Grep for every `file:line` in "Today in the code"; record drift as `Ruling:` lines. Read `docs/v2/PROGRESS.md` "Open follow-ups" for M6.7 (Transfers screen) and M7.8 (`OfferPermit`): they must be done in Prelude P before M9.1. Commit `M9.0a: refresh plan`, push.

### Task M9.1: Resume protocol (ADVISOR first)
**Files:** `crates/pb-protocol/src/{control.rs,lib.rs}`, `crates/pb-transport/src/files.rs`, `crates/pb-transport/src/tests.rs`, `fuzz/fuzz_targets/offer_resume.rs` (new), `docs/WIRE_PROTOCOL.md`.
- [ ] **Step 1 (ADVISOR):** show the advisor the ruling on fields and the prefix-hash flow before coding; apply its corrections to this plan file.
- [ ] **Step 1b: failing tests.** In `pb-protocol`: `offer_without_resume_token_decodes_as_0_2_0`, `unknown_fields_survive_round_trip`. In `pb-transport/tests.rs`: `resume_with_matching_prefix_continues` (send 3 MiB, cut after 1 MiB, resend: receiver gets bytes 1 MiB..3 MiB only; final sha256 equals the whole file), `resume_with_wrong_prefix_restarts_at_zero` (sender file changed), `no_resume_without_both_capabilities` (peer lacks `resume`: whole file sent), `a_0_2_0_peer_still_talks`.
- [ ] **Step 2:** `cargo test -p pb-protocol -p pb-transport resume` Expected: FAIL (fields missing).
- [ ] **Step 3: implement.** Add the fields; `local_capabilities()` adds `resume` and `RESERVED_CAPS` drops it in this same commit; sender picks a token per transfer and keeps it for retries; receiver replies `resume_from`/`prefix_sha256` only when a claim exists; sender seeks only after the hash matches. Size limits: token exactly 16 bytes, hash exactly 32, else the offer is rejected.
- [ ] **Step 4:** same command Expected: PASS; full gates; `cargo +nightly fuzz` not required, `(cd fuzz && cargo check --bins)` must pass.
- [ ] **Step 5:** PROGRESS, WIRE_PROTOCOL text (capability no longer reserved, new fields, size limits), commit `M9.1: resume protocol`, push, CI by full SHA.

### Task M9.2: Partial-file store
**Files:** `crates/pb-transport/src/partials.rs` (new), `files.rs`, `lib.rs`, `tests.rs`.
- [ ] **Step 1: failing tests** in `partials.rs`: `claim_returns_length_for_same_peer_token_name_size`, `token_of_another_peer_never_matches`, `partial_names_cannot_escape` (name `..\\x`, `a/../b`, 300-char name), `expires_after_24_hours`, `per_peer_cap_evicts_oldest`, `caps_hold_against_free_space` (free below `KEEP_FREE` + need: refuse to create), `persists_across_open`, `orphan_partials_are_swept`, `offset_math_is_u64_on_32_bit` (sizes > 4 GiB use u64 only; `#[cfg(target_pointer_width = "32")]` variant runs in the armv7 CI test build).
- [ ] **Step 2:** `cargo test -p pb-transport partials` Expected: FAIL.
- [ ] **Step 3: implement** per Shared interfaces; file name on disk is `<peer 8 hex>-<token 32 hex>.part` (never the peer's name); `resume.txt` lines `peer_hex token_hex name size created_unix`, written atomically (write temp + rename + fsync). Wire `files.rs` to use it and delete the "no partial is ever kept" comment.
- [ ] **Step 4:** tests PASS; full gates. **Step 5:** PROGRESS, DECISIONS (caps and expiry), commit `M9.2: partial-file store`, push, CI.

### Task M9.3: Send queue per device
**Files:** `crates/pb-tray/src/queue.rs` (new), `core.rs`, `pages/transfers.rs`; Android `devices/SendQueue.kt` (new), `EntryPoints.kt`, `ui/transfers/*`, `PbService.kt`.
- [ ] **Step 1: failing tests** (Rust and Kotlin pure): `items_send_in_order`, `queues_are_per_device`, `pause_stops_the_next_item_not_the_current_byte_stream` (current item finishes or is cancelled explicitly), `cancel_while_sending_deletes_the_partial`, `queue_survives_restart_as_paused`, `max_64_items_per_device`, `row_state_matches_engine_state` (UI state derived from the engine, not stored twice).
- [ ] **Step 2:** `cargo test -p pb-tray queue` / `gradlew :app:testDebugUnitTest --tests "*SendQueueTest"` Expected: FAIL.
- [ ] **Step 3: implement.** Queue persisted (`queue.txt` PC; prefs-backed JSON Android); NeedsFile when the source cannot be read after restart. Transfers page and screen show state, pause, resume, cancel with Narrator/TalkBack names.
- [ ] **Step 4/5:** PASS; gates; PROGRESS; commit `M9.3: send queue per device`; push; CI.

### Task M9.4: History and clipboard history (ADVISOR on the crypto)
**Files:** `crates/pb-tray/src/{timeline.rs,clip_history.rs (new),settings.rs,settings_groups.rs}`, Android `ClipHistory.kt` (new), `Timeline.kt`, `Prefs.kt`, `ui/transfers/*`, `ui/settings/SettingsTab.kt`, `strings`.
- [ ] **Step 1 (ADVISOR):** key handling (DPAPI entropy reuse vs new, Keystore key alias new `clip_history_v1`, AES-GCM nonce per record), what a lost key means.
- [ ] **Step 1b: failing tests:** `sensitive_clip_is_never_stored`, `one_time_code_is_never_stored`, `off_by_default`, `entries_older_than_7_days_are_gone`, `wrong_key_fails_closed`, `history_with_lost_key_reads_as_empty_not_garbage`, `search_matches_case_insensitively_in_memory`, `keep_history_off_wipes_clip_history`, `transfers_listed_only_when_keep_history_on`, `settings_page_row_count_stays_9_or_less`.
- [ ] **Step 2-4:** run, implement (record = version byte, nonce, ciphertext, created time; file `clip_history.bin`), PASS, gates. **Step 5:** THREAT_MODEL row for clipboard history, PROGRESS, commit `M9.4: history`, push, CI.

### Task M9.5: Drag-and-drop on the PC
**Files:** `crates/pb-tray/src/{flyout.rs,main_window.rs,screen_window.rs,dnd.rs (new),share.rs,core.rs}`.
- [ ] **Step 1: failing tests** (pure): `dropped_paths_are_filtered_to_files_that_exist`, `drop_with_ask_target_opens_the_picker_once`, `drop_of_a_folder_is_refused_with_a_note`, `drop_goes_to_default_when_target_is_default`.
- [ ] **Step 2-4:** implement `IDropTarget` registration with `RegisterDragDrop` on each window, effects only while visible, a visible drop hint honouring reduced motion; reuse the M8 picker; fixes the open M8 follow-up. **Step 5:** commit `M9.5: drag and drop on the PC`, push, CI.

### Task M9.6: Drag-and-drop on Android large screens
**Files:** `ui/home/HomeTab.kt`, `ui/transfers/*`, `devices/LargeScreen.kt` (new).
- [ ] **Step 1: failing tests:** `large_screen_is_600dp_or_wider`, `drop_with_two_pcs_uses_the_picker`, `dropped_uri_without_read_grant_is_refused`.
- [ ] **Step 2-4:** Compose `dragAndDropTarget` with `requestDragAndDropPermissions`; persist URI permission for queue items where allowed. **Step 5:** commit `M9.6: drag and drop on large screens`, push, CI.

### Task M9.R: Milestone review
- [ ] **Step 1:** one fresh Opus reviewer against the Review Focus and the spec lines; fix Critical and Important test-first; record the rest under Open follow-ups.
- [ ] **Step 2:** DECISIONS, THREAT_MODEL (rows 7, 14 and a new partial-files row), WIRE_PROTOCOL updated; DEVICE-CHECKS lines for M9 (resume across a Wi-Fi drop both directions, restart, cap eviction, queue, history, drop on a large screen/foldable emulator).
- [ ] **Step 3:** CI green by full SHA; **ADVISOR** before declaring done. Commit `M9.R: milestone review`, push.

## Self-Review
Spec coverage: resume (M9.1-M9.2), queue (M9.3), history (M9.4), drag-and-drop (M9.5-M9.6), review (M9.R). Type consistency: `PartialStore`, `Claim`, `SendQueue`, `QueueState` are used under the same names in every task. Proportion: signatures and test names only; bodies are left to the implementer.
