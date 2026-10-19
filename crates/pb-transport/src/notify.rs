//! Phone notifications on the PC: helpers shared by both sides.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use pb_identity::PeerId;
use pb_protocol::control::{
    ControlMsg, Notification, NotifyAction, NotifyActionResult, NotifyIcon, NotifyRemoved,
    NotifySync, NotifySyncRequest, MAX_NOTIFY_ICON, MAX_NOTIFY_KEY, MAX_NOTIFY_LINE,
    MAX_NOTIFY_LINES, MAX_NOTIFY_PICTURE, MAX_NOTIFY_SYNC_KEYS, MAX_NOTIFY_TEXT, MAX_NOTIFY_TITLE,
};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::{lock, Error, Node, Session};

/// Words that mark a message as carrying a one-time code.
const OTP_WORDS: [&str; 8] = [
    "otp",
    "code",
    "passcode",
    "verification",
    "verify",
    "one-time",
    "pin",
    "password",
];

/// The one-time code in a notification, if any: a 4–8 character token of
/// digits (or capitals and digits, at least 4 digits) in a message that
/// mentions a code. Parts of prices, dates, times, decimals and phone
/// numbers are not codes.
pub fn find_otp(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let mentions_code = OTP_WORDS.iter().any(|w| {
        lower.match_indices(w).any(|(i, _)| {
            !lower[..i]
                .chars()
                .next_back()
                .is_some_and(char::is_alphabetic)
        })
    });
    if !mentions_code {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_alphanumeric() {
            i += 1;
        }
        let token = &chars[start..i];
        if is_code(token) && !joined_to_number(&chars, start, i) {
            return Some(token.iter().collect());
        }
    }
    None
}

fn is_code(token: &[char]) -> bool {
    let digits = token.iter().filter(|c| c.is_ascii_digit()).count();
    (4..=8).contains(&token.len()) && digits >= 4 && !token.iter().any(char::is_ascii_lowercase)
}

/// The token at `start..end` continues a number (`1,299`, `12/10/2026`,
/// `10:30`, `45231.50`) or follows a currency sign.
fn joined_to_number(chars: &[char], start: usize, end: usize) -> bool {
    const JOINERS: [char; 4] = [',', '.', ':', '/'];
    const CURRENCY: [char; 4] = ['₹', '$', '€', '£'];
    let before = start.checked_sub(1).map(|i| chars[i]);
    let before2 = start.checked_sub(2).map(|i| chars[i]);
    if before.is_some_and(|c| CURRENCY.contains(&c))
        || (before == Some(' ') && before2.is_some_and(|c| CURRENCY.contains(&c)))
    {
        return true;
    }
    if before.is_some_and(|c| JOINERS.contains(&c)) && before2.is_some_and(|c| c.is_ascii_digit()) {
        return true;
    }
    let after = chars.get(end).copied();
    let after2 = chars.get(end + 1).copied();
    after.is_some_and(|c| JOINERS.contains(&c)) && after2.is_some_and(|c| c.is_ascii_digit())
}

/// The longest prefix of `s` that fits in `max_bytes` without splitting a
/// character.
pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// A short stable id for a notification key: safe for logs (says nothing
/// about the content) and short enough for a Windows toast tag.
pub fn key_hash(key: &str) -> String {
    Sha256::digest(key.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Counts events in one-second windows; `hit` is false once more than
/// `limit` events fell inside the current window.
pub struct RateWindow {
    limit: u32,
    count: u32,
    since: Instant,
}

impl RateWindow {
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            count: 0,
            since: Instant::now(),
        }
    }

    pub fn hit(&mut self) -> bool {
        if self.since.elapsed() >= Duration::from_secs(1) {
            self.since = Instant::now();
            self.count = 0;
        }
        self.count += 1;
        self.count <= self.limit
    }
}

/// A notify session stays open this long after its last message.
pub const OUTBOX_LINGER: Duration = Duration::from_secs(15);
/// Notifications re-sent (quietly) to a PC that missed them, and in the
/// answer to a sync request.
pub const MAX_RESEND: usize = 50;
/// Receiver limits: notify records per second (PC) and actions per second (phone).
pub const MAX_NOTIFY_PER_SEC: u32 = 50;
pub const MAX_ACTIONS_PER_SEC: u32 = 10;
/// App icons kept for sending.
const MAX_ICONS: usize = 64;
const MAX_ACTIONS: usize = 5;
const MAX_ACTION_LABEL: usize = 64;
/// How long the PC waits for the phone's answer to an action.
const ACTION_TIMEOUT: Duration = Duration::from_secs(10);

fn trim_string(s: &mut String, max: usize) {
    let end = truncate_utf8(s, max).len();
    s.truncate(end);
}

/// Apply the wire limits so the notification always fits one control frame.
/// Too much is dropped (oldest lines, the picture), never an error.
pub fn trim(n: &mut Notification) {
    trim_string(&mut n.key, MAX_NOTIFY_KEY);
    for s in [
        &mut n.package,
        &mut n.app_name,
        &mut n.title,
        &mut n.conversation,
    ] {
        trim_string(s, MAX_NOTIFY_TITLE);
    }
    trim_string(&mut n.text, MAX_NOTIFY_TEXT);
    trim_string(&mut n.otp, 16);
    if n.lines.len() > MAX_NOTIFY_LINES {
        n.lines.drain(..n.lines.len() - MAX_NOTIFY_LINES);
    }
    for l in &mut n.lines {
        trim_string(l, MAX_NOTIFY_LINE);
    }
    n.actions.truncate(MAX_ACTIONS);
    for a in &mut n.actions {
        trim_string(&mut a.label, MAX_ACTION_LABEL);
    }
    if n.icon_hash.len() > 32 {
        n.icon_hash.clear();
    }
    if n.picture.len() > MAX_NOTIFY_PICTURE {
        n.picture.clear();
    }
}

enum Out {
    Post(Box<Notification>),
    Remove(String),
}

impl Out {
    fn key(&self) -> &str {
        match self {
            Self::Post(n) => &n.key,
            Self::Remove(k) => k,
        }
    }
}

/// Only the newest item per key, in the order of those newest items: ten
/// updates of one chat become one message, a removal cancels a queued post.
fn coalesce(items: Vec<Out>) -> Vec<Out> {
    let mut last: HashMap<String, usize> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        last.insert(item.key().to_owned(), i);
    }
    items
        .into_iter()
        .enumerate()
        .filter(|(i, item)| last.get(item.key()) == Some(i))
        .map(|(_, item)| item)
        .collect()
}

/// An app icon by package: (SHA-256, PNG).
type Icon = (Vec<u8>, Vec<u8>);

#[derive(Default)]
struct PeerState {
    /// Feeds the running notify session; `None` when no session runs.
    tx: Option<mpsc::UnboundedSender<Out>>,
    /// Keys that failed to reach this peer, oldest first.
    missed: VecDeque<String>,
}

struct Outbox {
    node: Node,
    linger: Duration,
    /// What is on the phone now (pictures stripped), for sync and re-sends.
    active: Mutex<HashMap<String, Notification>>,
    icons: Mutex<HashMap<String, Icon>>,
    peers: Mutex<HashMap<PeerId, PeerState>>,
}

/// Phone side: sends notifications to every paired PC on short sessions
/// (see `docs/WIRE_PROTOCOL.md`, "Phone notifications").
#[derive(Clone)]
pub struct NotifyOutbox(Arc<Outbox>);

impl NotifyOutbox {
    pub fn new(node: Node) -> Self {
        Self(Arc::new(Outbox {
            node,
            linger: OUTBOX_LINGER,
            active: Mutex::default(),
            icons: Mutex::default(),
            peers: Mutex::default(),
        }))
    }

    #[cfg(test)]
    pub(crate) fn with_linger(self, linger: Duration) -> Self {
        let o = Arc::try_unwrap(self.0).unwrap_or_else(|_| panic!("outbox already shared"));
        Self(Arc::new(Outbox { linger, ..o }))
    }

    /// Whether any notify session is running or being opened.
    #[cfg(test)]
    pub(crate) fn busy(&self) -> bool {
        lock(&self.0.peers).values().any(|p| p.tx.is_some())
    }

    /// What is already on the phone when it starts: known for sync, not sent.
    pub fn seed(&self, list: Vec<Notification>) {
        let mut active = lock(&self.0.active);
        for mut n in list {
            trim(&mut n);
            n.picture.clear();
            active.insert(n.key.clone(), n);
        }
    }

    pub fn post(&self, mut n: Notification, icon_png: Option<Vec<u8>>) {
        trim(&mut n);
        {
            let mut icons = lock(&self.0.icons);
            if let Some(png) = icon_png.filter(|p| !p.is_empty() && p.len() <= MAX_NOTIFY_ICON) {
                if icons.len() >= MAX_ICONS && !icons.contains_key(&n.package) {
                    icons.clear();
                }
                let hash = Sha256::digest(&png).to_vec();
                icons.insert(n.package.clone(), (hash, png));
            }
            n.icon_hash = icons
                .get(&n.package)
                .map(|(h, _)| h.clone())
                .unwrap_or_default();
        }
        let mut stored = n.clone();
        stored.picture.clear();
        lock(&self.0.active).insert(n.key.clone(), stored);
        self.queue(|| Out::Post(Box::new(n.clone())));
    }

    pub fn remove(&self, key: &str) {
        lock(&self.0.active).remove(key);
        self.queue(|| Out::Remove(key.to_owned()));
    }

    fn queue(&self, item: impl Fn() -> Out) {
        let mut peers = lock(&self.0.peers);
        for p in self.0.node.paired_peers() {
            let st = peers.entry(p.id).or_default();
            if let Some(tx) = &st.tx {
                if tx.send(item()).is_ok() {
                    continue;
                }
            }
            let (tx, rx) = mpsc::unbounded_channel();
            let _ = tx.send(item());
            st.tx = Some(tx);
            tokio::spawn(run_peer(self.0.clone(), p.id, rx));
        }
    }

    /// Answer a PC's `NotifySyncRequest` on its session: quiet copies of the
    /// newest active notifications, then the list of active keys.
    pub async fn answer_sync_request(&self, s: &Session) -> Result<(), Error> {
        let newest = self.0.newest(MAX_RESEND);
        let mut sent_icons = HashSet::new();
        for mut n in newest {
            n.quiet = true;
            self.0.send_post(s, n, &mut sent_icons).await?;
        }
        s.send(ControlMsg::NotifySync(self.0.sync())).await
    }
}

impl Outbox {
    /// The newest `max` active notifications, newest first.
    fn newest(&self, max: usize) -> Vec<Notification> {
        let mut all: Vec<Notification> = lock(&self.active).values().cloned().collect();
        all.sort_by_key(|n| std::cmp::Reverse(n.when_ms));
        all.truncate(max);
        all
    }

    fn sync(&self) -> NotifySync {
        NotifySync {
            keys: self
                .newest(MAX_NOTIFY_SYNC_KEYS)
                .into_iter()
                .map(|n| n.key)
                .collect(),
        }
    }

    /// Send a notification, preceded by its app icon once per session.
    async fn send_post(
        &self,
        s: &Session,
        n: Notification,
        sent_icons: &mut HashSet<Vec<u8>>,
    ) -> Result<(), Error> {
        if !n.icon_hash.is_empty() && !sent_icons.contains(&n.icon_hash) {
            let png = lock(&self.icons)
                .get(&n.package)
                .filter(|(h, _)| *h == n.icon_hash)
                .map(|(_, p)| p.clone());
            if let Some(png) = png {
                s.send(ControlMsg::NotifyIcon(NotifyIcon {
                    hash: n.icon_hash.clone(),
                    png,
                }))
                .await?;
                sent_icons.insert(n.icon_hash.clone());
            }
        }
        s.send(ControlMsg::Notification(n)).await
    }

    /// Delivery failed: remember the posts for a quiet re-send, drop the
    /// queue and mark the peer idle (under the lock, so nothing queued in
    /// the meantime is lost).
    fn failed(&self, peer: PeerId, mut queue: Vec<Out>, rx: &mut mpsc::UnboundedReceiver<Out>) {
        let mut peers = lock(&self.peers);
        while let Ok(item) = rx.try_recv() {
            queue.push(item);
        }
        let st = peers.entry(peer).or_default();
        for item in queue {
            if let Out::Post(n) = item {
                st.missed.retain(|k| *k != n.key);
                st.missed.push_back(n.key);
                if st.missed.len() > MAX_RESEND {
                    st.missed.pop_front();
                }
            }
        }
        st.tx = None;
    }
}

/// One notify session to one PC: open on the first item, send what comes,
/// finish after `linger` with nothing to send.
async fn run_peer(o: Arc<Outbox>, peer: PeerId, mut rx: mpsc::UnboundedReceiver<Out>) {
    let mut queue: Vec<Out> = rx.recv().await.into_iter().collect();
    let s = match o.node.connect(&peer).await {
        Ok(s) => s,
        Err(_) => return o.failed(peer, queue, &mut rx),
    };
    let missed: Vec<String> = lock(&o.peers)
        .get_mut(&peer)
        .map(|st| st.missed.drain(..).collect())
        .unwrap_or_default();
    let mut sent_icons = HashSet::new();
    let resend: Vec<Notification> = {
        let active = lock(&o.active);
        missed
            .iter()
            .filter_map(|k| active.get(k).cloned())
            .collect()
    };
    let start = async {
        s.send(ControlMsg::NotifySync(o.sync())).await?;
        for mut n in resend {
            n.quiet = true;
            o.send_post(&s, n, &mut sent_icons).await?;
        }
        Ok::<_, Error>(())
    };
    if start.await.is_err() {
        // The re-sends did not get through either: keep them for next time.
        if let Some(st) = lock(&o.peers).get_mut(&peer) {
            for k in missed.into_iter().rev() {
                st.missed.push_front(k);
            }
        }
        return o.failed(peer, queue, &mut rx);
    }
    loop {
        while let Ok(item) = rx.try_recv() {
            queue.push(item);
        }
        let mut batch = coalesce(std::mem::take(&mut queue)).into_iter();
        while let Some(item) = batch.next() {
            let sent = match &item {
                Out::Post(n) => o.send_post(&s, (**n).clone(), &mut sent_icons).await,
                Out::Remove(k) => {
                    s.send(ControlMsg::NotifyRemoved(NotifyRemoved { key: k.clone() }))
                        .await
                }
            };
            if sent.is_err() {
                let rest = std::iter::once(item).chain(batch).collect();
                return o.failed(peer, rest, &mut rx);
            }
        }
        match tokio::time::timeout(o.linger, rx.recv()).await {
            Ok(Some(item)) => queue.push(item),
            Ok(None) => break,
            Err(_) => {
                let mut peers = lock(&o.peers);
                match rx.try_recv() {
                    Ok(item) => queue.push(item),
                    Err(_) => {
                        if let Some(st) = peers.get_mut(&peer) {
                            st.tx = None;
                        }
                        break;
                    }
                }
            }
        }
    }
    s.finish().await;
}

impl Node {
    /// PC side: ask the phone to act on a notification and wait for its answer.
    pub async fn notify_action(
        &self,
        peer: &PeerId,
        mut action: NotifyAction,
    ) -> Result<NotifyActionResult, Error> {
        let mut s = self.connect(peer).await?;
        action.id = crate::files::new_id();
        let id = action.id;
        s.send(ControlMsg::NotifyAction(action)).await?;
        let answer = tokio::time::timeout(ACTION_TIMEOUT, async {
            loop {
                match s.recv().await? {
                    Some(ControlMsg::NotifyActionResult(r)) if r.id == id => return Ok(r),
                    Some(_) => continue,
                    None => return Err(Error::Closed),
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout)?;
        s.finish().await;
        answer
    }

    /// PC side, at start: ask the phone what is active. The returned session
    /// carries quiet notifications and a `NotifySync`, then ends.
    pub async fn notify_sync_request(&self, peer: &PeerId) -> Result<Session, Error> {
        let s = self.connect(peer).await?;
        s.send(ControlMsg::NotifySyncRequest(NotifySyncRequest {}))
            .await?;
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_codes_next_to_keywords() {
        for (t, c) in [
            ("Your OTP is 482913", "482913"),
            ("G-582044 is your Google verification code", "582044"),
            ("Use code 7731 to sign in", "7731"),
            ("Passcode: 09AB12", "09AB12"),
            ("123456 is your WhatsApp code", "123456"),
            ("Your login code is 904417. Do not share it.", "904417"),
        ] {
            assert_eq!(find_otp(t).as_deref(), Some(c), "{t}");
        }
    }

    #[test]
    fn ignores_numbers_that_are_not_codes() {
        for t in [
            "Your order of ₹1,299 ships on 12/10/2026",
            "Call me on 9876543210",
            "Meeting at 10:30",
            "Balance 45231.50",
            "See you in 2026",
            "Room 4021 is ready",
        ] {
            assert_eq!(find_otp(t), None, "{t}");
        }
    }

    #[test]
    fn truncation_keeps_char_boundaries() {
        let s = "a😀b";
        assert_eq!(truncate_utf8(s, 2), "a");
        assert_eq!(truncate_utf8(s, 5), "a😀");
        assert_eq!(truncate_utf8("שלום", 3), "ש");
        assert_eq!(truncate_utf8("abc", 10), "abc");
    }

    #[test]
    fn rate_window_allows_limit_then_refuses() {
        let mut w = RateWindow::new(50);
        assert!((0..50).all(|_| w.hit()));
        assert!(!w.hit());
    }

    #[test]
    fn key_hash_is_16_hex() {
        let h = key_hash("0|com.whatsapp|1|null|10123");
        assert_eq!(h.len(), 16);
        assert!(h.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(h, key_hash("x"));
    }
}
