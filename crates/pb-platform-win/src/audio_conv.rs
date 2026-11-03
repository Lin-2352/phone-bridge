//! Pure audio conversion: sample formats, channel remix, linear resampling and
//! 20 ms framing. No device or thread code here, so it is unit-tested exactly.

/// Capture/wire frame length in milliseconds.
pub const FRAME_MS: u32 = 20;
/// Capture output rate and channel count (the wire format Relay sends).
pub const OUT_RATE: u32 = 48_000;
pub const OUT_CHANNELS: usize = 2;
/// Frames (per channel) in one 20 ms frame at 48 kHz.
pub const FRAME_SAMPLES: usize = (OUT_RATE * FRAME_MS / 1000) as usize;
/// Bytes in one 20 ms 48 kHz stereo i16 frame.
pub const FRAME_BYTES: usize = FRAME_SAMPLES * OUT_CHANNELS * 2;

/// i16 to f32 in [-1.0, 1.0).
pub fn i16_to_f32(s: i16) -> f32 {
    f32::from(s) / 32768.0
}

/// f32 to i16 with clamping; NaN becomes 0.
pub fn f32_to_i16(x: f32) -> i16 {
    if x.is_nan() {
        return 0;
    }
    // `as` saturates, so out-of-range values clamp.
    (x * 32768.0).round() as i16
}

/// Remix interleaved `samples` from `from_ch` to `to_ch` channels.
/// To mono: average. From mono to n>=2: copy into channels 0 and 1, rest silent.
/// From stereo to n>2: L, R, then silence. Trailing partial frames are dropped.
pub fn remix(samples: &[i16], from_ch: usize, to_ch: usize) -> Vec<i16> {
    if from_ch == 0 || to_ch == 0 {
        return Vec::new();
    }
    let frames = samples.len() / from_ch;
    let samples = &samples[..frames * from_ch];
    if from_ch == to_ch {
        return samples.to_vec();
    }
    let mut out = Vec::with_capacity(frames * to_ch);
    for f in samples.chunks_exact(from_ch) {
        if to_ch == 1 {
            let sum: i32 = f.iter().map(|&s| i32::from(s)).sum();
            out.push((sum / from_ch as i32) as i16);
        } else {
            let l = f[0];
            let r = if from_ch >= 2 { f[1] } else { f[0] };
            out.push(l);
            out.push(r);
            out.extend(std::iter::repeat_n(0, to_ch - 2));
        }
    }
    out
}

/// Streaming linear-interpolation resampler over interleaved samples.
/// Output positions are exact rationals, so chunking never changes the result.
pub struct Resampler {
    from: u64,
    to: u64,
    channels: usize,
    /// Unconsumed input, interleaved; `buf[0..channels]` is the frame at `i`.
    buf: Vec<i16>,
    /// Phase within the current input frame, in units of 1/`to`.
    ph: u64,
}

impl Resampler {
    pub fn new(from_rate: u32, to_rate: u32, channels: usize) -> Self {
        Self {
            from: u64::from(from_rate),
            to: u64::from(to_rate),
            channels: channels.max(1),
            buf: Vec::new(),
            ph: 0,
        }
    }

    pub fn process(&mut self, input: &[i16]) -> Vec<i16> {
        let ch = self.channels;
        let input = &input[..input.len() / ch * ch];
        if self.from == self.to {
            return input.to_vec();
        }
        self.buf.extend_from_slice(input);
        let frames = self.buf.len() / ch;
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + 1 < frames {
            let ph = self.ph as i64;
            let to = self.to as i64;
            for c in 0..ch {
                let a = i64::from(self.buf[i * ch + c]);
                let b = i64::from(self.buf[(i + 1) * ch + c]);
                out.push(((a * (to - ph) + b * ph) / to) as i16);
            }
            self.ph += self.from;
            i += (self.ph / self.to) as usize;
            self.ph %= self.to;
        }
        // Keep the frame at `i` (or nothing if input ran out exactly).
        let keep = i.min(frames);
        self.buf.drain(..keep * ch);
        out
    }
}

/// Turns arbitrary device audio into 20 ms 48 kHz stereo i16 LE frames.
pub struct FrameBuilder {
    in_channels: usize,
    resampler: Resampler,
    pending: Vec<u8>,
}

impl FrameBuilder {
    pub fn new(in_rate: u32, in_channels: usize) -> Self {
        Self {
            in_channels,
            resampler: Resampler::new(in_rate, OUT_RATE, OUT_CHANNELS),
            pending: Vec::with_capacity(FRAME_BYTES * 2),
        }
    }

    pub fn push_f32(&mut self, input: &[f32], out: &mut impl FnMut(Vec<u8>)) {
        let s: Vec<i16> = input.iter().map(|&x| f32_to_i16(x)).collect();
        self.push_i16(&s, out);
    }

    pub fn push_i16(&mut self, input: &[i16], out: &mut impl FnMut(Vec<u8>)) {
        let stereo = remix(input, self.in_channels, OUT_CHANNELS);
        for s in self.resampler.process(&stereo) {
            self.pending.extend_from_slice(&s.to_le_bytes());
        }
        while self.pending.len() >= FRAME_BYTES {
            let rest = self.pending.split_off(FRAME_BYTES);
            out(std::mem::replace(&mut self.pending, rest));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_format_conversions_are_exact() {
        assert_eq!(i16_to_f32(-32768), -1.0);
        assert_eq!(i16_to_f32(0), 0.0);
        assert_eq!(i16_to_f32(16384), 0.5);
        assert_eq!(f32_to_i16(1.0), 32767);
        assert_eq!(f32_to_i16(-1.0), -32768);
        assert_eq!(f32_to_i16(0.5), 16384);
        assert_eq!(f32_to_i16(2.0), 32767);
        assert_eq!(f32_to_i16(-3.0), -32768);
        assert_eq!(f32_to_i16(f32::NAN), 0);
    }

    #[test]
    fn remix_covers_mono_stereo_and_surround() {
        assert_eq!(remix(&[1, 2], 1, 2), vec![1, 1, 2, 2]);
        assert_eq!(remix(&[10, 20, -4, -6], 2, 1), vec![15, -5]);
        assert_eq!(remix(&[1, 2, 3, 4], 2, 2), vec![1, 2, 3, 4]);
        assert_eq!(remix(&[1, 2], 2, 6), vec![1, 2, 0, 0, 0, 0]);
        assert_eq!(remix(&[5], 1, 6), vec![5, 5, 0, 0, 0, 0]);
        assert_eq!(remix(&[1, 2, 3], 2, 2), vec![1, 2]);
        assert_eq!(remix(&[32767, 32767], 2, 1), vec![32767]);
    }

    #[test]
    fn upsample_by_two_interpolates_exactly_and_carries_state() {
        let mut r = Resampler::new(24000, 48000, 1);
        assert_eq!(r.process(&[0, 100, 200]), vec![0, 50, 100, 150]);
        assert_eq!(r.process(&[300]), vec![200, 250]);
    }

    #[test]
    fn downsample_by_two_picks_every_other_sample() {
        let mut r = Resampler::new(48000, 24000, 1);
        assert_eq!(r.process(&[0, 10, 20, 30, 40]), vec![0, 20]);
    }

    #[test]
    fn stereo_channels_are_interpolated_independently() {
        let mut r = Resampler::new(24000, 48000, 2);
        assert_eq!(r.process(&[0, 1000, 100, 1100]), vec![0, 1000, 50, 1050]);
    }

    #[test]
    fn same_rate_is_a_passthrough() {
        let mut r = Resampler::new(48000, 48000, 2);
        assert_eq!(r.process(&[1, 2, 3, 4]), vec![1, 2, 3, 4]);
    }

    #[test]
    fn resampling_44100_to_48000_has_the_exact_length_and_is_chunk_invariant() {
        let input: Vec<i16> = (0..4410).map(|i| (i % 1000) as i16).collect();
        let whole = Resampler::new(44100, 48000, 1).process(&input);
        assert_eq!(whole.len(), 4799);
        let mut r = Resampler::new(44100, 48000, 1);
        let mut chunked = Vec::new();
        for c in input.chunks(441) {
            chunked.extend(r.process(c));
        }
        assert_eq!(chunked, whole);
        assert_eq!(whole[0], 0);
        // A constant signal stays constant.
        let flat = Resampler::new(48000, 44100, 1).process(&[700; 480]);
        assert!(flat.iter().all(|&s| s == 700));
    }

    fn collect(fb: &mut FrameBuilder, f: &[f32]) -> Vec<Vec<u8>> {
        let mut v = Vec::new();
        fb.push_f32(f, &mut |x| v.push(x));
        v
    }

    #[test]
    fn framing_emits_exact_20ms_frames_and_keeps_the_remainder() {
        let mut fb = FrameBuilder::new(48000, 2);
        let a = collect(&mut fb, &vec![0.0; 2000 * 2]);
        assert_eq!(a.len(), 2);
        assert!(a.iter().all(|f| f.len() == FRAME_BYTES && f.len() == 3840));
        let b = collect(&mut fb, &vec![0.0; 880 * 2]);
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn mono_f32_becomes_stereo_i16_le() {
        let mut fb = FrameBuilder::new(48000, 1);
        let f = collect(&mut fb, &vec![0.5; FRAME_SAMPLES]);
        assert_eq!(f.len(), 1);
        let first = i16::from_le_bytes([f[0][0], f[0][1]]);
        let second = i16::from_le_bytes([f[0][2], f[0][3]]);
        assert_eq!((first, second), (16384, 16384));
    }

    #[test]
    fn i16_input_and_44100_input_are_framed_at_48k() {
        let mut fb = FrameBuilder::new(44100, 2);
        let mut n = 0;
        fb.push_i16(&vec![0i16; 4410 * 2], &mut |x| {
            assert_eq!(x.len(), 3840);
            n += 1;
        });
        assert_eq!(n, 4); // 4799 output frames -> 4 x 960
    }
}
