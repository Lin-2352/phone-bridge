//! What the source delivers each 33 ms tick: a FIFO of `RequestSample`
//! tokens answered one per tick with the newest frame, the last frame
//! again, or black. Pure (no COM) so it is fully tested.

use std::collections::VecDeque;

use pb_camera_frames::{heartbeat::FRAME_100NS, Deliver, Heartbeat, Read, Ring};

/// The only media types the stream advertises (NV12, 30 fps).
pub const TYPES: [(u32, u32); 2] = [(1920, 1080), (1280, 720)];

/// Black in NV12: Y = 16, UV = 128.
pub fn black(w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut v = vec![16u8; w * h];
    v.resize(w * h * 3 / 2, 128);
    v
}

/// One sample to hand to the app, with the token of the request it answers.
pub struct Sample<T> {
    pub token: T,
    pub data: Vec<u8>,
    pub pts_100ns: i64,
    pub duration_100ns: i64,
}

/// A media type the stream does not offer, or a change after the start.
#[derive(Debug, PartialEq, Eq)]
pub struct TypeRefused;

/// The chosen type: fixed once the stream started.
#[derive(Default)]
pub struct TypeState {
    current: Option<(u32, u32)>,
    started: bool,
}

impl TypeState {
    /// Choose a type (only the advertised ones); refused once started and
    /// different: a type change mid-call breaks apps.
    pub fn set_current(&mut self, w: u32, h: u32) -> Result<(), TypeRefused> {
        if !TYPES.contains(&(w, h)) {
            return Err(TypeRefused);
        }
        if self.started && self.current != Some((w, h)) {
            return Err(TypeRefused);
        }
        self.current = Some((w, h));
        Ok(())
    }

    pub fn current(&self) -> Option<(u32, u32)> {
        self.current
    }

    pub fn start(&mut self) {
        self.started = true;
    }

    /// Stopped: the next `Start` (another app, another size) may pick again.
    pub fn stop(&mut self) {
        self.started = false;
    }
}

pub struct Pump<T> {
    size: (u32, u32),
    queue: VecDeque<T>,
    hb: Heartbeat,
    last: Vec<u8>,
    seen: u64,
}

impl<T> Pump<T> {
    pub fn new(size: (u32, u32)) -> Self {
        Self {
            size,
            queue: VecDeque::new(),
            hb: Heartbeat::new(),
            last: Vec::new(),
            seen: 0,
        }
    }

    /// `RequestSample`: bursts are fine, the tokens wait in order.
    pub fn request(&mut self, token: T) {
        self.queue.push_back(token);
    }

    pub fn waiting(&self) -> usize {
        self.queue.len()
    }

    /// No readable ring at all (no frames file): answer one request with black.
    pub fn black_sample(&mut self, now_100ns: i64) -> Option<Sample<T>> {
        let token = self.queue.pop_front()?;
        Some(Sample {
            token,
            data: black(self.size.0, self.size.1),
            pts_100ns: self.hb.next_pts(now_100ns),
            duration_100ns: FRAME_100NS,
        })
    }

    /// One 33 ms tick: look at the ring, answer at most ONE request.
    /// `tray_alive` false (the tray stopped beating) shows black.
    pub fn tick(&mut self, ring: &Ring, tray_alive: bool, now_100ns: i64) -> Option<Sample<T>> {
        let mut fresh = Vec::new();
        let read = if tray_alive {
            ring.read_newest(self.seen, self.size, &mut fresh)
        } else {
            Read::Empty
        };
        if !tray_alive {
            self.last.clear();
            self.seen = 0;
            self.hb = Heartbeat::new();
        }
        let deliver = self.hb.tick(&read);
        if let Read::New { seq, .. } = read {
            self.seen = seq;
            self.last = fresh;
        }
        let token = self.queue.pop_front()?;
        let data = match deliver {
            Deliver::Black => black(self.size.0, self.size.1),
            Deliver::New | Deliver::Repeat => self.last.clone(),
        };
        Some(Sample {
            token,
            data,
            pts_100ns: self.hb.next_pts(now_100ns),
            duration_100ns: FRAME_100NS,
        })
    }
}

#[cfg(test)]
mod tests {
    use pb_camera_frames::FILE_BYTES;

    use super::*;

    fn words() -> Vec<u64> {
        vec![0u64; FILE_BYTES.div_ceil(8)]
    }

    fn put(r: &mut Ring, w: u32, h: u32, v: u8) {
        r.write_frame(w, h, 1, |s| s[..(w * h * 3 / 2) as usize].fill(v));
    }

    #[test]
    fn without_a_ring_requests_still_get_black() {
        let mut p = Pump::new((1920, 1080));
        p.request(1);
        p.request(2);
        let s = p.black_sample(1_000_000).unwrap();
        assert_eq!((s.token, s.data[0]), (1, 16));
        assert_eq!(s.data.len(), 1920 * 1080 * 3 / 2);
        assert_eq!(p.waiting(), 1);
    }

    #[test]
    fn media_types_are_fixed_16_9() {
        assert_eq!(TYPES, [(1920, 1080), (1280, 720)]);
    }

    #[test]
    fn media_type_never_changes_after_start() {
        let mut t = TypeState::default();
        assert!(t.set_current(1280, 720).is_ok());
        assert!(t.set_current(1920, 1080).is_ok(), "free before the start");
        t.start();
        assert!(t.set_current(1920, 1080).is_ok(), "the same type again");
        assert!(t.set_current(1280, 720).is_err(), "a change mid-call");
        assert_eq!(t.current(), Some((1920, 1080)));
        assert!(TypeState::default().set_current(640, 480).is_err());
        // The next client after a Stop may use the other size.
        t.stop();
        assert!(t.set_current(1280, 720).is_ok(), "free again after Stop");
    }

    #[test]
    fn samples_flow_from_an_empty_ring_as_black() {
        let mut b = words();
        let ring = Ring::init(&mut b);
        let mut p = Pump::new((1280, 720));
        p.request(1);
        let s = p.tick(&ring, true, 1_000_000).expect("a sample");
        assert_eq!(s.data.len(), 1280 * 720 * 3 / 2);
        assert_eq!(s.data[0], 16, "Y of black");
        assert_eq!(*s.data.last().unwrap(), 128, "UV of black");
        assert_eq!(s.duration_100ns, 333_333);
    }

    #[test]
    fn burst_of_requests_is_answered_one_per_tick() {
        let mut b = words();
        let ring = Ring::init(&mut b);
        let mut p = Pump::new((1280, 720));
        for token in 0..5 {
            p.request(token);
        }
        let mut got = Vec::new();
        let mut last_pts = 0;
        for tick in 0..5 {
            let s = p.tick(&ring, true, 1_000_000 + tick * 333_333).unwrap();
            assert!(s.pts_100ns > last_pts, "timestamps increase");
            last_pts = s.pts_100ns;
            got.push(s.token);
            assert_eq!(p.waiting(), 4 - tick as usize, "one answered per tick");
        }
        assert_eq!(got, vec![0, 1, 2, 3, 4], "FIFO order");
        assert!(p.tick(&ring, true, 9_000_000).is_none(), "nothing queued");
    }

    #[test]
    fn samples_show_the_newest_ring_frame() {
        let mut b = words();
        let mut ring = Ring::init(&mut b);
        let mut p = Pump::new((1280, 720));
        put(&mut ring, 1280, 720, 200);
        p.request(1);
        assert_eq!(p.tick(&ring, true, 1).unwrap().data[0], 200);
        // No new frame: the last one again (a lens switch, a Wi-Fi hiccup).
        p.request(2);
        assert_eq!(p.tick(&ring, true, 2).unwrap().data[0], 200);
        // The tray stopped beating: black, nothing of the picture stays.
        p.request(3);
        assert_eq!(p.tick(&ring, false, 3).unwrap().data[0], 16);
    }

    #[test]
    fn a_frame_of_the_other_size_is_never_copied_in() {
        let mut b = words();
        let mut ring = Ring::init(&mut b);
        let mut p = Pump::new((1280, 720));
        put(&mut ring, 1920, 1080, 99);
        p.request(1);
        let s = p.tick(&ring, true, 1).unwrap();
        assert_eq!(s.data.len(), 1280 * 720 * 3 / 2, "the 720p sample size");
        assert_eq!(s.data[0], 16, "black: the tray has not resized yet");
    }
}
