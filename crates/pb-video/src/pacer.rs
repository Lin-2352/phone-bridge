//! Keeps the video close to live without artefacts: when frames are in
//! flight too long or the send queue fills, drop to the next key frame and
//! lower the bitrate; recover gradually once things are clean again.
//!
//! Lag counts only frames in flight: now − send time of the oldest frame the
//! PC has not confirmed. An idle screen (nothing sent) is never lag.

use std::{collections::VecDeque, time::Duration, time::Instant};

use crate::record::{FLAG_KEY, FLAG_REPEAT};

const SECOND: Duration = Duration::from_secs(1);

pub const MAX_LAG: Duration = Duration::from_millis(150);
/// Send queue cap (bytes); full = drop.
pub const QUEUE_CAP: usize = 1024 * 1024;
/// Recovery only while the queue is below this.
pub const RECOVER_QUEUE: usize = 256 * 1024;
pub const FLOOR_BPS: u32 = 1_500_000;
/// At most this many consecutive repeat frames are sent.
pub const MAX_REPEATS: u32 = 3;

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
    repeats: u32,
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
            repeats: 0,
        }
    }

    pub fn bitrate(&self) -> u32 {
        self.bitrate
    }

    /// A frame came out of the encoder: send it or drop it.
    pub fn on_frame(
        &mut self,
        now: Instant,
        pts_us: i64,
        flags: u8,
        queued_bytes: usize,
    ) -> Decision {
        let key = flags & FLAG_KEY != 0;
        let skip = Decision {
            send: false,
            request_key: false,
            new_bitrate: None,
        };
        // A still screen: a few sharpening repeats, then silence.
        if flags & FLAG_REPEAT != 0 && !key {
            if self.repeats >= MAX_REPEATS {
                return skip;
            }
            self.repeats += 1;
        } else {
            self.repeats = 0;
        }
        let lag = self.in_flight.front().map_or(Duration::ZERO, |(_, sent)| {
            now.saturating_duration_since(*sent)
        });
        if lag > MAX_LAG || queued_bytes >= QUEUE_CAP {
            self.last_drop = Some(now);
            if self.dropping {
                return skip;
            }
            self.dropping = true;
            self.bitrate = (self.bitrate as u64 * 7 / 10).max(u64::from(FLOOR_BPS)) as u32;
            return Decision {
                send: false,
                request_key: true,
                new_bitrate: Some(self.bitrate),
            };
        }
        if self.dropping {
            if !key {
                return skip;
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
    pub fn on_feedback(&mut self, _now: Instant, presented_pts_us: i64) {
        while self
            .in_flight
            .front()
            .is_some_and(|(pts, _)| *pts <= presented_pts_us)
        {
            self.in_flight.pop_front();
        }
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
    fn idle_gap_is_not_lag() {
        let t0 = Instant::now();
        let mut p = Pacer::new(T);
        assert!(p.on_frame(t0, 0, FLAG_KEY, 0).send);
        p.on_feedback(ms(t0, 30), 0);
        let d = p.on_frame(ms(t0, 5000), 5_000_000, 0, 0);
        assert!(d.send && !d.request_key && d.new_bitrate.is_none(), "{d:?}");
    }

    #[test]
    fn lag_over_150ms_drops_until_key_and_backs_off() {
        let t0 = Instant::now();
        let mut p = Pacer::new(T);
        assert!(p.on_frame(t0, 0, FLAG_KEY, 0).send);
        assert!(p.on_frame(ms(t0, 33), 33_000, 0, 0).send);
        assert!(p.on_frame(ms(t0, 66), 66_000, 0, 0).send);
        // Nothing confirmed for 200 ms.
        let d = p.on_frame(ms(t0, 200), 200_000, 0, 0);
        assert_eq!(
            d,
            Decision {
                send: false,
                request_key: true,
                new_bitrate: Some(8_400_000)
            }
        );
        // The PC catches up, but non-key frames still wait for the key.
        p.on_feedback(ms(t0, 210), 66_000);
        assert!(!p.on_frame(ms(t0, 233), 233_000, 0, 0).send);
        let k = p.on_frame(ms(t0, 266), 266_000, FLAG_KEY, 0);
        assert!(k.send && !k.request_key && k.new_bitrate.is_none(), "{k:?}");
        assert!(p.on_frame(ms(t0, 300), 300_000, 0, 0).send);
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
    fn only_three_repeats_are_sent() {
        let t0 = Instant::now();
        let mut p = Pacer::new(T);
        let sent: Vec<bool> = (0..5u64)
            .map(|i| {
                let d = p.on_frame(ms(t0, i * 100), (i * 100_000) as i64, FLAG_REPEAT, 0);
                p.on_feedback(ms(t0, i * 100 + 1), (i * 100_000) as i64);
                assert!(!d.request_key && d.new_bitrate.is_none());
                d.send
            })
            .collect();
        assert_eq!(sent, [true, true, true, false, false]);
        // A real change resets the count.
        assert!(p.on_frame(ms(t0, 600), 600_000, 0, 0).send);
        assert!(p.on_frame(ms(t0, 700), 700_000, FLAG_REPEAT, 0).send);
    }
}
