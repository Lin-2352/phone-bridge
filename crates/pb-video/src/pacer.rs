//! Keeps the video close to live without artefacts: when frames are in
//! flight too long or the send queue fills, drop everything up to the next
//! key frame (never a lone frame: every frame is a reference for the next)
//! and lower the bitrate; recover gradually once things are clean again.
//!
//! Lag counts only frames in flight: now − send time of the oldest frame the
//! PC has not confirmed. An idle screen (nothing sent) is never lag.

use std::{collections::VecDeque, time::Duration, time::Instant};

use crate::record::{FLAG_CONFIG, FLAG_KEY};

const SECOND: Duration = Duration::from_secs(1);

/// Frames unconfirmed this long count as late (Wi-Fi jitter, the PC's
/// display wait and the feedback trip all fit under it).
pub const MAX_LAG: Duration = Duration::from_millis(300);
/// Send queue cap (bytes); full = drop.
pub const QUEUE_CAP: usize = 1024 * 1024;
/// Recovery only while the queue is below this.
pub const RECOVER_QUEUE: usize = 256 * 1024;
pub const FLOOR_BPS: u32 = 1_500_000;
/// While waiting for a key frame, ask the encoder again this often.
pub const KEY_RETRY: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub send: bool,
    pub request_key: bool,
    /// The encoder's new bitrate, when it changes.
    pub new_bitrate: Option<u32>,
}

pub struct Pacer {
    target: u32,
    bitrate: u32,
    /// (pts, sent at) of frames not yet confirmed by the PC.
    in_flight: VecDeque<(i64, Instant)>,
    dropping: bool,
    last_drop: Option<Instant>,
    last_raise: Option<Instant>,
    last_key_request: Option<Instant>,
    /// Send → confirmed times (ms) since the last `take_confirm_ms`.
    confirm_ms: Vec<u32>,
}

impl Pacer {
    pub fn new(target_bps: u32) -> Self {
        Self {
            target: target_bps,
            bitrate: target_bps,
            in_flight: VecDeque::new(),
            dropping: false,
            last_drop: None,
            last_raise: None,
            last_key_request: None,
            confirm_ms: Vec::new(),
        }
    }

    pub fn bitrate(&self) -> u32 {
        self.bitrate
    }

    /// Start dropping up to the next key frame: back off once, ask for a key.
    fn enter_drop(&mut self, now: Instant) -> Decision {
        self.last_drop = Some(now);
        if self.dropping {
            return self.waiting(now);
        }
        self.dropping = true;
        self.in_flight.clear();
        self.last_key_request = Some(now);
        self.bitrate = (self.bitrate as u64 * 7 / 10).max(u64::from(FLOOR_BPS)) as u32;
        Decision {
            send: false,
            request_key: true,
            new_bitrate: Some(self.bitrate),
        }
    }

    /// A frame dropped while waiting for the key; ask again now and then.
    fn waiting(&mut self, now: Instant) -> Decision {
        let again = self
            .last_key_request
            .is_none_or(|t| now.duration_since(t) >= KEY_RETRY);
        if again {
            self.last_key_request = Some(now);
        }
        Decision {
            send: false,
            request_key: again,
            new_bitrate: None,
        }
    }

    /// The send queue refused a frame: what follows has no reference.
    pub fn queue_full(&mut self, now: Instant) -> Decision {
        self.enter_drop(now)
    }

    /// A frame came out of the encoder: send it or drop it.
    pub fn on_frame(
        &mut self,
        now: Instant,
        pts_us: i64,
        flags: u8,
        queued_bytes: usize,
    ) -> Decision {
        // A new encoder's headers: tiny, and the next key frame cannot be
        // decoded without them. Never dropped, never lag.
        if flags & FLAG_CONFIG != 0 {
            return Decision {
                send: true,
                request_key: false,
                new_bitrate: None,
            };
        }
        let key = flags & FLAG_KEY != 0;
        let lag = self.in_flight.front().map_or(Duration::ZERO, |(_, sent)| {
            now.saturating_duration_since(*sent)
        });
        if lag > MAX_LAG || queued_bytes >= QUEUE_CAP {
            return self.enter_drop(now);
        }
        if self.dropping {
            if !key {
                return self.waiting(now);
            }
            self.dropping = false;
        }
        self.in_flight.push_back((pts_us, now));
        let clean = self
            .last_drop
            .is_none_or(|d| now.duration_since(d) >= SECOND);
        let due = self
            .last_raise
            .is_none_or(|r| now.duration_since(r) >= SECOND);
        let mut new_bitrate = None;
        if self.bitrate < self.target && clean && due && queued_bytes < RECOVER_QUEUE {
            self.bitrate = (self.bitrate as u64 * 11 / 10).min(u64::from(self.target)) as u32;
            self.last_raise = Some(now);
            new_bitrate = Some(self.bitrate);
        }
        Decision {
            send: true,
            request_key: false,
            new_bitrate,
        }
    }

    /// The PC presented up to this pts.
    pub fn on_feedback(&mut self, now: Instant, presented_pts_us: i64) {
        while let Some(&(pts, sent)) = self.in_flight.front() {
            if pts > presented_pts_us {
                break;
            }
            self.in_flight.pop_front();
            if self.confirm_ms.len() < 1000 {
                self.confirm_ms
                    .push(now.saturating_duration_since(sent).as_millis() as u32);
            }
        }
    }

    /// Send → shown-on-the-PC round trips (ms) measured since the last call.
    pub fn take_confirm_ms(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.confirm_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{FLAG_KEY, FLAG_REPEAT};

    const T: u32 = 12_000_000;

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    #[test]
    fn confirm_times_are_measured() {
        let t0 = Instant::now();
        let mut p = Pacer::new(T);
        assert!(p.on_frame(t0, 0, FLAG_KEY, 0).send);
        assert!(p.on_frame(ms(t0, 10), 10_000, 0, 0).send);
        p.on_feedback(ms(t0, 40), 0);
        p.on_feedback(ms(t0, 60), 10_000);
        assert_eq!(p.take_confirm_ms(), [40, 50]);
        assert!(p.take_confirm_ms().is_empty(), "taken once");
    }

    #[test]
    fn idle_gap_is_not_lag() {
        let t0 = Instant::now();
        let mut p = Pacer::new(T);
        assert!(p.on_frame(t0, 0, FLAG_KEY, 0).send);
        p.on_feedback(ms(t0, 30), 0);
        let d = p.on_frame(ms(t0, 5000), 5_000_000, 0, 0);
        assert!(d.send && !d.request_key && d.new_bitrate.is_none(), "{d:?}");
    }

    #[test]
    fn config_records_are_always_sent() {
        use crate::record::FLAG_CONFIG;
        let t0 = Instant::now();
        let mut p = Pacer::new(T);
        assert!(p.on_frame(t0, 0, FLAG_KEY, 0).send);
        // Lag over the limit: drop mode, yet a new encoder's headers go out
        // (the key frame after them is decoded with them).
        assert!(!p.on_frame(ms(t0, 350), 350_000, 0, 0).send);
        assert!(p.on_frame(ms(t0, 360), 360_000, FLAG_CONFIG, 0).send);
        // ...without ending drop mode or counting as lag.
        assert!(!p.on_frame(ms(t0, 383), 383_000, 0, 0).send);
        // A full queue never stops them either.
        assert!(
            p.on_frame(ms(t0, 400), 400_000, FLAG_CONFIG, QUEUE_CAP)
                .send
        );
    }

    #[test]
    fn lag_over_300ms_drops_until_key_and_backs_off() {
        let t0 = Instant::now();
        let mut p = Pacer::new(T);
        assert!(p.on_frame(t0, 0, FLAG_KEY, 0).send);
        assert!(p.on_frame(ms(t0, 33), 33_000, 0, 0).send);
        // 200 ms unconfirmed is normal Wi-Fi + feedback delay: no drop.
        assert!(p.on_frame(ms(t0, 200), 200_000, 0, 0).send);
        // Nothing confirmed for 350 ms: drop.
        let d = p.on_frame(ms(t0, 350), 350_000, 0, 0);
        assert_eq!(
            d,
            Decision {
                send: false,
                request_key: true,
                new_bitrate: Some(8_400_000)
            }
        );
        // The PC catches up, but non-key frames still wait for the key.
        p.on_feedback(ms(t0, 360), 200_000);
        assert!(!p.on_frame(ms(t0, 383), 383_000, 0, 0).send);
        let k = p.on_frame(ms(t0, 416), 416_000, FLAG_KEY, 0);
        assert!(k.send && !k.request_key && k.new_bitrate.is_none(), "{k:?}");
        assert!(p.on_frame(ms(t0, 450), 450_000, 0, 0).send);
    }

    #[test]
    fn drop_mode_asks_for_a_key_again_every_300ms() {
        let t0 = Instant::now();
        let mut p = Pacer::new(T);
        assert!(p.on_frame(t0, 0, 0, QUEUE_CAP).request_key);
        let d = p.on_frame(ms(t0, 100), 100_000, 0, 0);
        assert!(!d.send && !d.request_key, "{d:?}");
        let d = p.on_frame(ms(t0, 310), 310_000, 0, 0);
        assert!(!d.send && d.request_key, "asked again: {d:?}");
        let d = p.on_frame(ms(t0, 400), 400_000, 0, 0);
        assert!(!d.send && !d.request_key, "{d:?}");
        assert!(p.on_frame(ms(t0, 420), 420_000, FLAG_KEY, 0).send);
    }

    #[test]
    fn queue_full_enters_drop_mode() {
        let t0 = Instant::now();
        let mut p = Pacer::new(T);
        assert!(p.on_frame(t0, 0, FLAG_KEY, 0).send);
        p.on_feedback(ms(t0, 5), 0);
        let d = p.queue_full(ms(t0, 10));
        assert!(d.request_key && d.new_bitrate == Some(8_400_000), "{d:?}");
        // The P-frame after a refused one has no reference: wait for a key.
        assert!(!p.on_frame(ms(t0, 33), 33_000, 0, 0).send);
        assert!(p.on_frame(ms(t0, 66), 66_000, FLAG_KEY, 0).send);
    }

    #[test]
    fn full_queue_drops() {
        let t0 = Instant::now();
        let mut p = Pacer::new(T);
        let d = p.on_frame(t0, 0, 0, QUEUE_CAP);
        assert!(!d.send && d.request_key);
    }

    #[test]
    fn bitrate_has_a_floor() {
        let t0 = Instant::now();
        let mut p = Pacer::new(2_000_000);
        for i in 0..10u64 {
            p.on_frame(ms(t0, i * 10), (i * 10_000) as i64, FLAG_KEY, QUEUE_CAP);
        }
        assert_eq!(p.bitrate(), FLOOR_BPS);
    }

    #[test]
    fn recovery_is_ten_percent_per_second_after_a_clean_second() {
        let t0 = Instant::now();
        let mut p = Pacer::new(T);
        p.on_frame(t0, 0, FLAG_KEY, QUEUE_CAP); // drop: 8.4 M
        assert_eq!(p.bitrate(), 8_400_000);
        // Under a second clean: nothing yet.
        let d = p.on_frame(ms(t0, 900), 900_000, FLAG_KEY, 0);
        assert!(d.new_bitrate.is_none(), "{d:?}");
        p.on_feedback(ms(t0, 901), 900_000);
        // A full queue blocks recovery.
        let d = p.on_frame(ms(t0, 1100), 1_100_000, 0, RECOVER_QUEUE);
        assert!(d.new_bitrate.is_none(), "{d:?}");
        p.on_feedback(ms(t0, 1101), 1_100_000);
        let d = p.on_frame(ms(t0, 1200), 1_200_000, 0, 0);
        assert_eq!(d.new_bitrate, Some(9_240_000));
        p.on_feedback(ms(t0, 1201), 1_200_000);
        // The next raise waits another second.
        assert!(p
            .on_frame(ms(t0, 1700), 1_700_000, 0, 0)
            .new_bitrate
            .is_none());
        p.on_feedback(ms(t0, 1701), 1_700_000);
        assert_eq!(
            p.on_frame(ms(t0, 2200), 2_200_000, 0, 0).new_bitrate,
            Some(10_164_000)
        );
        p.on_feedback(ms(t0, 2201), 2_200_000);
        // Never above the target.
        let mut t = 2200;
        for _ in 0..10 {
            t += 1000;
            p.on_frame(ms(t0, t), (t * 1000) as i64, 0, 0);
            p.on_feedback(ms(t0, t + 1), (t * 1000) as i64);
        }
        assert_eq!(p.bitrate(), T);
    }

    #[test]
    fn repeats_are_never_dropped() {
        // Every frame is a reference for the next: dropping a "repeat"
        // would smear the picture until the next key frame.
        let t0 = Instant::now();
        let mut p = Pacer::new(T);
        for i in 0..10u64 {
            let d = p.on_frame(ms(t0, i * 100), (i * 100_000) as i64, FLAG_REPEAT, 0);
            p.on_feedback(ms(t0, i * 100 + 1), (i * 100_000) as i64);
            assert!(d.send && !d.request_key, "{d:?}");
        }
    }
}
