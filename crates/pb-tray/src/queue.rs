//! The send queue of each phone: what waits, what is sending, what is paused.
//!
//! Plain data and rules only (no Windows, no network), so it is tested without a window. The
//! engine (`core.rs`) asks `next` for the item to send, reports `finish`/`fail`, and the transfers
//! page draws `rows`. The state shown is derived from the engine through `reconcile`, never kept
//! as a second truth. Pause, resume and cancel are local only: no wire message.
#![allow(dead_code)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// At most this many unfinished items wait for one phone.
pub const MAX_PER_DEVICE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueState {
    Waiting,
    Sending,
    Paused,
    /// The source file cannot be read any more (after a restart, or it was moved).
    NeedsFile,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: u64,
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    /// The resume token of this item, kept across retries and restarts.
    pub token: [u8; 16],
    pub state: QueueState,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EnqueueError {
    Full,
}

#[derive(Default, Debug)]
pub struct SendQueue {
    items: Vec<Item>,
    paused: bool,
    next_id: u64,
    /// Files whose sending was stopped on purpose to hold them (not the whole queue).
    holding: Vec<u64>,
}

impl SendQueue {
    fn unfinished(&self) -> usize {
        self.items
            .iter()
            .filter(|i| !matches!(i.state, QueueState::Done | QueueState::Failed))
            .count()
    }

    pub fn enqueue(
        &mut self,
        path: PathBuf,
        name: String,
        size: u64,
        token: [u8; 16],
    ) -> Result<u64, EnqueueError> {
        if self.unfinished() >= MAX_PER_DEVICE {
            return Err(EnqueueError::Full);
        }
        self.next_id += 1;
        let id = self.next_id;
        self.items.push(Item {
            id,
            path,
            name,
            size,
            token,
            state: QueueState::Waiting,
        });
        Ok(id)
    }

    /// The next item to send, now marked Sending; None while one is sending, while paused, or
    /// when nothing waits. Pausing stops the next item, never the current byte stream.
    pub fn next(&mut self) -> Option<Item> {
        if self.paused || self.items.iter().any(|i| i.state == QueueState::Sending) {
            return None;
        }
        let item = self
            .items
            .iter_mut()
            .find(|i| i.state == QueueState::Waiting)?;
        item.state = QueueState::Sending;
        Some(item.clone())
    }

    fn set(&mut self, id: u64, state: QueueState) {
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            i.state = state;
        }
    }

    pub fn finish(&mut self, id: u64) {
        self.set(id, QueueState::Done);
    }

    pub fn fail(&mut self, id: u64) {
        self.set(id, QueueState::Failed);
    }

    /// The engine was interrupted (link lost): the item waits again with the same token.
    pub fn retry(&mut self, id: u64) {
        self.set(id, QueueState::Waiting);
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Holds one file: it is skipped until [`release`](Self::release). True when it is the file sending,
    /// which the caller must stop (the engine then reports it through [`stopped_for_hold`](Self::stopped_for_hold)).
    pub fn hold(&mut self, id: u64) -> bool {
        let Some(i) = self.items.iter_mut().find(|i| i.id == id) else {
            return false;
        };
        match i.state {
            QueueState::Waiting | QueueState::NeedsFile => {
                i.state = QueueState::Paused;
                false
            }
            QueueState::Sending => {
                if !self.holding.contains(&id) {
                    self.holding.push(id);
                }
                true
            }
            _ => false,
        }
    }

    /// A held file waits its turn again.
    pub fn release(&mut self, id: u64) {
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            if i.state == QueueState::Paused {
                i.state = QueueState::Waiting;
            }
        }
        self.holding.retain(|h| *h != id);
    }

    /// The engine stopped file `id` because Relay was told to pause. True when that was a hold of this one
    /// file: it is now parked (same token) and the queue goes on; false when it was the whole queue pausing.
    pub fn stopped_for_hold(&mut self, id: u64) -> bool {
        let Some(at) = self.holding.iter().position(|h| *h == id) else {
            return false;
        };
        self.holding.remove(at);
        self.set(id, QueueState::Paused);
        true
    }

    /// Takes out a file that has not started. The one sending is left alone (false): its transfer would
    /// go on without a place in the queue.
    pub fn remove_waiting(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items
            .retain(|i| i.id != id || i.state == QueueState::Sending);
        self.items.len() != before
    }

    /// The file was retried by hand: its place in the queue goes, so it is not sent twice.
    pub fn drop_path(&mut self, path: &Path) {
        self.items
            .retain(|i| i.path != path || i.state == QueueState::Sending);
    }

    /// Files that still have to be sent or are sending.
    pub fn pending(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.state, QueueState::Waiting | QueueState::Sending))
            .count()
    }

    /// Removes every item that has not started (the user cancelled what was queued); the one
    /// sending, if any, is left to the caller to cancel.
    pub fn clear_waiting(&mut self) {
        self.items.retain(|i| {
            !matches!(
                i.state,
                QueueState::Waiting | QueueState::NeedsFile | QueueState::Paused
            )
        });
    }

    /// A file that was missing may be back (a USB stick, a share): Waiting again when `exists` finds it.
    pub fn recheck(&mut self, exists: impl Fn(&Path) -> bool) {
        for i in &mut self.items {
            if i.state == QueueState::NeedsFile && exists(&i.path) {
                i.state = QueueState::Waiting;
            }
        }
    }

    pub fn resume(&mut self) {
        self.paused = false;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Removes an item. Returns true when it was the one sending: the caller must cancel the
    /// transfer, which deletes the partial on the receiver.
    pub fn cancel(&mut self, id: u64) -> bool {
        let was_sending = self
            .items
            .iter()
            .any(|i| i.id == id && i.state == QueueState::Sending);
        self.items.retain(|i| i.id != id);
        was_sending
    }

    /// Makes the state agree with the engine: an item marked Sending that the engine does not
    /// run (it died with the process or the link) waits again instead of showing "sending".
    pub fn reconcile(&mut self, active: &[u64]) {
        for i in &mut self.items {
            if i.state == QueueState::Sending && !active.contains(&i.id) {
                i.state = QueueState::Waiting;
            }
        }
    }

    /// What the page draws. A paused queue shows its waiting items as Paused.
    pub fn rows(&self) -> Vec<Item> {
        self.items
            .iter()
            .map(|i| {
                let mut r = i.clone();
                if self.paused && r.state == QueueState::Waiting {
                    r.state = QueueState::Paused;
                }
                r
            })
            .collect()
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }
}

/// One queue per phone.
#[derive(Default, Debug)]
pub struct Queues {
    by_peer: HashMap<String, SendQueue>,
    /// The page's one switch: a phone met later starts paused too, so the page never says
    /// "Resume queue" while a queue is sending.
    paused_all: bool,
}

impl Queues {
    pub fn of(&mut self, peer: &str) -> &mut SendQueue {
        let paused = self.paused_all;
        self.by_peer
            .entry(peer.to_owned())
            .or_insert_with(|| SendQueue {
                paused,
                ..SendQueue::default()
            })
    }

    pub fn get(&self, peer: &str) -> Option<&SendQueue> {
        self.by_peer.get(peer)
    }

    /// The page's one Pause / Resume switch covers every phone.
    /// The phone was unpaired: its queue goes with it.
    pub fn forget(&mut self, peer: &str) {
        self.by_peer.remove(peer);
    }

    /// Holds the file being sent from `path` in whichever queue has it; true when one was sending (the caller
    /// stops its transfer).
    pub fn hold_sending_path(&mut self, path: &Path) -> bool {
        for q in self.by_peer.values_mut() {
            let id = q
                .items
                .iter()
                .find(|i| i.state == QueueState::Sending && i.path == path)
                .map(|i| i.id);
            if let Some(id) = id {
                return q.hold(id);
            }
        }
        false
    }

    /// Cancel all: every phone's queue loses what has not started (held files too).
    pub fn clear_all_waiting(&mut self) {
        for q in self.by_peer.values_mut() {
            q.clear_waiting();
        }
    }

    pub fn set_all_paused(&mut self, paused: bool) {
        self.paused_all = paused;
        for q in self.by_peer.values_mut() {
            q.paused = paused;
            if !paused {
                // Resume queue means everything: files held one by one go too.
                for i in &mut q.items {
                    if i.state == QueueState::Paused {
                        i.state = QueueState::Waiting;
                    }
                }
            }
        }
    }

    pub fn any_paused(&self) -> bool {
        self.paused_all || self.by_peer.values().any(|q| q.paused)
    }

    /// Every queue's rows (paused shown as such), with the phone's queue key, in a stable order.
    pub fn rows_all(&self) -> Vec<(String, Item)> {
        let mut peers: Vec<_> = self.by_peer.keys().cloned().collect();
        peers.sort();
        peers
            .into_iter()
            .flat_map(|p| {
                self.by_peer[&p]
                    .rows()
                    .into_iter()
                    .map(move |i| (p.clone(), i))
            })
            .collect()
    }

    /// After Resume: files that were missing are looked for again.
    pub fn recheck(&mut self, exists: impl Fn(&Path) -> bool) {
        for q in self.by_peer.values_mut() {
            q.recheck(&exists);
        }
    }

    /// Phones that have an item ready to send (after Resume, each needs a worker).
    pub fn peers_with_waiting(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .by_peer
            .iter()
            .filter(|(_, q)| q.items.iter().any(|i| i.state == QueueState::Waiting))
            .map(|(p, _)| p.clone())
            .collect();
        v.sort();
        v
    }

    /// `queue.txt`: one line per unfinished item, `peer \t id \t token \t size \t name \t path`
    /// with tab, newline and `%` percent-escaped in the free-text fields. Done and failed items
    /// are not kept.
    pub fn save(&self) -> String {
        let mut peers: Vec<_> = self.by_peer.keys().collect();
        peers.sort();
        let mut out = String::new();
        for p in peers {
            for i in &self.by_peer[p].items {
                if matches!(i.state, QueueState::Done | QueueState::Failed) {
                    continue;
                }
                out.push_str(&format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\n",
                    esc(p),
                    i.id,
                    hex(&i.token),
                    i.size,
                    esc(&i.name),
                    esc(&i.path.to_string_lossy())
                ));
            }
        }
        out
    }

    /// Reads `queue.txt` after a start. Every queue comes back paused (nothing sends before the
    /// user resumes) and an item whose source `exists` no longer finds is NeedsFile. Lines that
    /// do not parse are skipped.
    pub fn load(text: &str, exists: impl Fn(&Path) -> bool) -> Self {
        let mut q = Queues::default();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 6 {
                continue;
            }
            let (Ok(id), Ok(size), Some(token)) =
                (f[1].parse::<u64>(), f[3].parse::<u64>(), unhex(f[2]))
            else {
                continue;
            };
            let path = PathBuf::from(unesc(f[5]));
            let queue = q.of(&unesc(f[0]));
            if queue.unfinished() >= MAX_PER_DEVICE {
                continue;
            }
            queue.paused = true;
            queue.next_id = queue.next_id.max(id);
            let state = if exists(&path) {
                QueueState::Waiting
            } else {
                QueueState::NeedsFile
            };
            queue.items.push(Item {
                id,
                path,
                name: unesc(f[4]),
                size,
                token,
                state,
            });
        }
        // Everything that came back is paused, and so is a phone met after the restart.
        q.paused_all = !q.by_peer.is_empty();
        q
    }
}

/// Retries a send after a broken link at most this many times.
pub const MAX_RETRIES: u32 = 3;

/// Whether a failed send is tried again with the same token. Only a link that broke after the
/// other device accepted and bytes flowed (`bytes_sent > 0`) is retried: an unanswered or
/// dismissed prompt also ends in a timeout, and re-offering it would ask the person again.
pub fn should_retry(transient: bool, attempt: u32, cancelled: bool, bytes_sent: u64) -> bool {
    transient && !cancelled && bytes_sent > 0 && attempt < MAX_RETRIES
}

fn esc(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\t', "%09")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
}

fn unesc(s: &str) -> String {
    s.replace("%0D", "\r")
        .replace("%0A", "\n")
        .replace("%09", "\t")
        .replace("%25", "%")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(q: &mut SendQueue, n: &str) -> u64 {
        q.enqueue(
            PathBuf::from(format!("C:/x/{n}")),
            n.into(),
            10,
            [n.len() as u8; 16],
        )
        .unwrap()
    }

    #[test]
    fn only_a_broken_transfer_is_retried_never_an_unanswered_prompt() {
        assert!(should_retry(true, 0, false, 1_000), "link lost mid-file");
        assert!(
            !should_retry(true, 0, false, 0),
            "no byte flowed: the prompt timed out or was dismissed"
        );
        assert!(
            !should_retry(true, 3, false, 1_000),
            "three retries at most"
        );
        assert!(!should_retry(true, 0, true, 1_000), "a cancel is final");
        assert!(
            !should_retry(false, 0, false, 1_000),
            "declined, too large and the like"
        );
    }

    #[test]
    fn items_send_in_order() {
        let mut q = SendQueue::default();
        let (a, b) = (add(&mut q, "a"), add(&mut q, "b"));
        assert_eq!(q.next().unwrap().id, a);
        assert!(q.next().is_none(), "one at a time");
        q.finish(a);
        assert_eq!(q.next().unwrap().id, b);
    }

    #[test]
    fn a_held_file_is_skipped_until_released_and_the_others_go_on() {
        let mut q = SendQueue::default();
        let (a, b, c) = (add(&mut q, "a"), add(&mut q, "b"), add(&mut q, "c"));
        assert!(!q.hold(a), "a waits: nothing to stop");
        assert_eq!(q.next().unwrap().id, b, "a is skipped");
        q.finish(b);
        assert_eq!(q.next().unwrap().id, c);
        q.finish(c);
        assert!(q.next().is_none(), "a stays held");
        let shown = q.rows().into_iter().find(|i| i.id == a).unwrap();
        assert_eq!(shown.state, QueueState::Paused);
        q.release(a);
        assert_eq!(q.next().unwrap().id, a);
    }

    #[test]
    fn holding_the_file_that_sends_parks_it_when_the_engine_stops_it() {
        let mut q = SendQueue::default();
        let (a, b) = (add(&mut q, "a"), add(&mut q, "b"));
        assert_eq!(q.next().unwrap().id, a);
        assert!(q.hold(a), "it is sending: the caller stops the transfer");
        assert!(
            q.stopped_for_hold(a),
            "the stop was a hold, not a queue pause"
        );
        assert!(!q.stopped_for_hold(a), "said once");
        assert_eq!(q.next().unwrap().id, b, "the queue goes on without it");
        let shown = q.rows().into_iter().find(|i| i.id == a).unwrap();
        assert_eq!(shown.state, QueueState::Paused);
        assert!(!q.is_paused(), "only that file is held");
    }

    #[test]
    fn held_files_can_be_removed_cleared_and_released_by_resume() {
        let mut qs = Queues::default();
        let (a, b, c) = {
            let q = qs.of("pc");
            (add(q, "a"), add(q, "b"), add(q, "c"))
        };
        qs.of("pc").hold(a);
        qs.of("pc").hold(b);
        assert!(qs.of("pc").remove_waiting(a), "a held file can be removed");
        qs.set_all_paused(true);
        qs.set_all_paused(false);
        let next = qs.of("pc").next().unwrap();
        assert_eq!(
            next.id, b,
            "Resume queue also releases what was held one by one"
        );
        qs.of("pc").finish(b);
        qs.of("pc").hold(c);
        let d = add(qs.of("other"), "d");
        qs.clear_all_waiting();
        assert!(
            qs.of("pc").items().iter().all(|i| i.id != c),
            "Cancel all takes held files too"
        );
        assert!(
            qs.of("other").items().iter().all(|i| i.id != d),
            "every phone's queue"
        );
    }

    #[test]
    fn queues_are_per_device() {
        let mut qs = Queues::default();
        add(qs.of("phone-1"), "a");
        add(qs.of("phone-2"), "b");
        qs.of("phone-1").pause();
        assert!(qs.of("phone-1").next().is_none());
        assert_eq!(qs.of("phone-2").next().unwrap().name, "b");
    }

    #[test]
    fn pause_stops_the_next_item_not_the_current_byte_stream() {
        let mut q = SendQueue::default();
        let (a, b) = (add(&mut q, "a"), add(&mut q, "b"));
        assert_eq!(q.next().unwrap().id, a);
        q.pause();
        assert_eq!(
            q.items()[0].state,
            QueueState::Sending,
            "current keeps going"
        );
        q.finish(a);
        assert!(q.next().is_none(), "the next one waits for resume");
        assert_eq!(q.rows()[1].state, QueueState::Paused);
        q.resume();
        assert_eq!(q.next().unwrap().id, b);
    }

    #[test]
    fn cancel_while_sending_tells_the_caller_to_cancel_the_transfer() {
        let mut q = SendQueue::default();
        let (a, b) = (add(&mut q, "a"), add(&mut q, "b"));
        q.next();
        assert!(
            q.cancel(a),
            "the caller must stop the stream (deletes the partial)"
        );
        assert!(!q.cancel(b), "a waiting item just leaves");
        assert!(q.items().is_empty());
    }

    #[test]
    fn queue_survives_restart_as_paused() {
        let mut qs = Queues::default();
        let a = add(qs.of("phone-1"), "a");
        add(qs.of("phone-1"), "b");
        qs.of("phone-1").next();
        qs.of("phone-1").finish(a);
        let text = qs.save();
        let mut back = Queues::load(&text, |_| true);
        let q = back.of("phone-1");
        assert!(q.is_paused());
        assert_eq!(q.items().len(), 1, "the finished item is not kept");
        assert_eq!(
            q.items()[0].token,
            [1; 16],
            "the token survives so a resume can match"
        );
        assert!(q.next().is_none(), "nothing sends before the user resumes");
    }

    #[test]
    fn pausing_all_queues_and_resuming_them_is_one_switch() {
        let mut qs = Queues::default();
        add(qs.of("a"), "1");
        add(qs.of("b"), "2");
        assert!(!qs.any_paused());
        qs.set_all_paused(true);
        assert!(qs.any_paused());
        assert!(qs.of("a").next().is_none() && qs.of("b").next().is_none());
        qs.set_all_paused(false);
        assert!(!qs.any_paused());
        assert!(qs.of("a").next().is_some());
        assert_eq!(qs.peers_with_waiting(), vec!["b".to_string()]);
    }

    #[test]
    fn remove_never_touches_the_file_that_is_sending() {
        let mut q = SendQueue::default();
        let a = add(&mut q, "a");
        let b = add(&mut q, "b");
        q.next();
        assert!(!q.remove_waiting(a), "a has started: Remove cannot take it");
        assert!(q.remove_waiting(b));
        assert_eq!(q.items().len(), 1);
        assert_eq!(q.pending(), 1);
    }

    #[test]
    fn a_retried_file_leaves_its_queue_place_and_unpairing_drops_the_queue() {
        let mut qs = Queues::default();
        add(qs.of("a"), "x.bin");
        add(qs.of("a"), "y.bin");
        let path = qs.of("a").items()[0].path.clone();
        qs.of("a").drop_path(&path);
        assert_eq!(qs.of("a").items().len(), 1);
        qs.forget("a");
        assert!(qs.get("a").is_none());
        assert!(qs.rows_all().is_empty());
    }

    #[test]
    fn a_phone_met_after_pausing_starts_paused_too() {
        let mut qs = Queues::default();
        add(qs.of("a"), "1");
        qs.set_all_paused(true);
        // A phone with no queue yet must not send while the page says "Resume queue".
        add(qs.of("b"), "2");
        assert!(qs.of("b").is_paused());
        assert!(qs.of("b").next().is_none());
        qs.set_all_paused(false);
        assert!(qs.of("b").next().is_some());
        assert!(!qs.any_paused());
    }

    #[test]
    fn resume_rechecks_files_that_were_missing() {
        let mut qs = Queues::default();
        add(qs.of("p"), "usb.bin");
        let mut back = Queues::load(&qs.save(), |_| false);
        assert_eq!(back.of("p").items()[0].state, QueueState::NeedsFile);
        // The stick is back: Resume finds the file again.
        back.recheck(|_| true);
        assert_eq!(back.of("p").items()[0].state, QueueState::Waiting);
        // Still gone: it stays "needs the file" and does not block a new send forever.
        let mut gone = Queues::load(&qs.save(), |_| false);
        gone.recheck(|_| false);
        assert_eq!(gone.of("p").items()[0].state, QueueState::NeedsFile);
        let id = gone.of("p").items()[0].id;
        assert!(
            !gone.of("p").cancel(id),
            "removing a missing file is not a running transfer"
        );
        assert!(gone.of("p").items().is_empty());
    }

    #[test]
    fn cancelling_what_is_waiting_keeps_the_file_that_is_sending() {
        let mut q = SendQueue::default();
        let a = add(&mut q, "a");
        add(&mut q, "b");
        add(&mut q, "c");
        q.next();
        q.clear_waiting();
        assert_eq!(q.items().len(), 1);
        assert_eq!(q.items()[0].id, a);
        assert_eq!(q.items()[0].state, QueueState::Sending);
    }

    #[test]
    fn a_missing_source_after_restart_needs_the_file() {
        let mut qs = Queues::default();
        add(qs.of("p"), "gone.bin");
        let mut back = Queues::load(&qs.save(), |_| false);
        assert_eq!(back.of("p").items()[0].state, QueueState::NeedsFile);
    }

    #[test]
    fn max_64_items_per_device() {
        let mut q = SendQueue::default();
        for i in 0..MAX_PER_DEVICE {
            add(&mut q, &format!("f{i}"));
        }
        assert_eq!(
            q.enqueue(PathBuf::from("x"), "x".into(), 1, [0; 16]),
            Err(EnqueueError::Full)
        );
        let id = q.items()[0].id;
        q.next();
        q.finish(id);
        assert!(
            q.enqueue(PathBuf::from("y"), "y".into(), 1, [0; 16])
                .is_ok(),
            "done frees a slot"
        );
    }

    #[test]
    fn row_state_matches_engine_state() {
        let mut q = SendQueue::default();
        let a = add(&mut q, "a");
        q.next();
        assert_eq!(q.rows()[0].state, QueueState::Sending);
        q.reconcile(&[]);
        assert_eq!(
            q.rows()[0].state,
            QueueState::Waiting,
            "the engine is not sending it"
        );
        q.next();
        q.reconcile(&[a]);
        assert_eq!(q.rows()[0].state, QueueState::Sending);
    }

    #[test]
    fn odd_names_and_paths_round_trip_and_bad_lines_are_skipped() {
        let mut qs = Queues::default();
        qs.of("a\tb")
            .enqueue(
                PathBuf::from("C:/we ird/na\tme\n%.bin"),
                "na\tme\n%.bin".into(),
                5,
                [7; 16],
            )
            .unwrap();
        let text = format!("{}garbage\n1\t2\n", qs.save());
        let back = Queues::load(&text, |_| true);
        let q = back.get("a\tb").unwrap();
        assert_eq!(q.items()[0].name, "na\tme\n%.bin");
        assert_eq!(q.items().len(), 1);
    }
}
