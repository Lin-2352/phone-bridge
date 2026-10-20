//! The source's 30 fps clock: what each 33 ms tick delivers.

use crate::layout::Read;

/// One 30 fps frame in 100 ns units.
pub const FRAME_100NS: i64 = 333_333;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Deliver {
    /// Black NV12 (nothing arrived yet).
    Black,
    /// The last frame again (a gap, a torn or mismatched read).
    Repeat,
    /// The frame just read.
    New,
}

#[derive(Default)]
pub struct Heartbeat {
    has_frame: bool,
    last_pts: i64,
}

impl Heartbeat {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tick(&mut self, read: &Read) -> Deliver {
        if matches!(read, Read::New { .. }) {
            self.has_frame = true;
            Deliver::New
        } else if self.has_frame {
            Deliver::Repeat
        } else {
            Deliver::Black
        }
    }

    /// A timestamp for the next sample: the clock, never behind the
    /// previous one plus a frame.
    pub fn next_pts(&mut self, now_100ns: i64) -> i64 {
        let t = now_100ns.max(self.last_pts + FRAME_100NS);
        self.last_pts = t;
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_frame() -> Read {
        Read::New {
            seq: 1,
            w: 1280,
            h: 720,
            pts_us: 0,
        }
    }

    #[test]
    fn heartbeat_black_before_first_frame() {
        let mut h = Heartbeat::new();
        assert_eq!(h.tick(&Read::Empty), Deliver::Black);
        assert_eq!(h.tick(&Read::Same), Deliver::Black);
    }

    #[test]
    fn heartbeat_repeats_last_frame() {
        let mut h = Heartbeat::new();
        assert_eq!(h.tick(&new_frame()), Deliver::New);
        assert_eq!(h.tick(&Read::Same), Deliver::Repeat);
        assert_eq!(h.tick(&Read::Torn), Deliver::Repeat);
        assert_eq!(h.tick(&Read::Mismatch), Deliver::Repeat);
        assert_eq!(h.tick(&Read::Empty), Deliver::Repeat);
    }

    #[test]
    fn heartbeat_timestamps_increase() {
        let mut h = Heartbeat::new();
        let mut prev = h.next_pts(1_000);
        for _ in 0..100 {
            let t = h.next_pts(1_000);
            assert!(t >= prev + FRAME_100NS, "{t} after {prev}");
            prev = t;
        }
    }
}
