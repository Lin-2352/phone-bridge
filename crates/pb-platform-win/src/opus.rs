//! Opus packets from the phone to the PCM the player takes. The bytes come from
//! the network: a bad packet is concealed like a lost one, never a panic.
//!
//! libopus itself (transpiled to Rust), the reference decoder and heavily
//! fuzzed. A pure-Rust decoder was tried first and took 27 ms for each 20 ms
//! packet from the phone's encoder: it could not keep up.
#![allow(unsafe_code)]

use unsafe_libopus as libopus;

/// 120 ms, the longest an Opus packet can be, per channel at 48 kHz.
const MAX_SAMPLES_PER_CHANNEL: usize = 5760;
/// What is concealed before the first packet says otherwise: 20 ms at 48 kHz.
const DEFAULT_SAMPLES_PER_CHANNEL: i32 = 960;

pub struct OpusDec {
    dec: *mut libopus::OpusDecoder,
    pcm: Vec<i16>,
    channels: usize,
    /// Samples per channel of the last good packet: the length to conceal.
    last_samples: i32,
    /// Packets the decoder refused.
    pub bad_packets: u64,
}

impl OpusDec {
    pub fn new(sample_rate: u32, channels: u32) -> Result<Self, String> {
        let mut err = 0;
        // SAFETY: `err` outlives the call; the arguments are plain values the
        // function checks, returning an error for a bad rate or channel count.
        let dec =
            unsafe { libopus::opus_decoder_create(sample_rate as i32, channels as i32, &mut err) };
        if dec.is_null() || err != 0 {
            return Err(format!(
                "Opus decoder refused {sample_rate} Hz, {channels} channels"
            ));
        }
        let channels = channels as usize;
        Ok(Self {
            dec,
            pcm: vec![0; MAX_SAMPLES_PER_CHANNEL * channels],
            channels,
            last_samples: DEFAULT_SAMPLES_PER_CHANNEL,
            bad_packets: 0,
        })
    }

    /// One decode: `None` packet conceals `last_samples`. Samples per channel,
    /// negative on error.
    fn run(&mut self, packet: Option<&[u8]>) -> i32 {
        let (data, len, room) = match packet {
            Some(p) => (p.as_ptr(), p.len() as i32, MAX_SAMPLES_PER_CHANNEL as i32),
            None => (std::ptr::null(), 0, self.last_samples),
        };
        // SAFETY: `dec` is the live decoder this value owns. `data` is valid for
        // `len` bytes, or null with length 0, which the API defines as a lost
        // packet. `pcm` holds MAX_SAMPLES_PER_CHANNEL samples per channel for the
        // channel count the decoder was created with, and `room` never exceeds
        // that, so the decoder cannot write past it.
        unsafe { libopus::opus_decode(self.dec, data, len, self.pcm.as_mut_ptr(), room, 0) }
    }

    /// 16-bit LE PCM for one packet; an empty packet was lost and is concealed.
    /// Empty output when there is nothing to play.
    pub fn decode(&mut self, packet: &[u8]) -> Vec<u8> {
        let mut n = if packet.is_empty() {
            self.run(None)
        } else {
            let n = self.run(Some(packet));
            if n > 0 {
                self.last_samples = n;
            }
            n
        };
        if n < 0 && !packet.is_empty() {
            self.bad_packets += 1;
            n = self.run(None);
        }
        if n <= 0 {
            return Vec::new();
        }
        let samples = (n as usize)
            .saturating_mul(self.channels)
            .min(self.pcm.len());
        self.pcm[..samples]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect()
    }
}

impl Drop for OpusDec {
    fn drop(&mut self) {
        // SAFETY: `dec` came from `opus_decoder_create` and is destroyed once, here.
        unsafe { libopus::opus_decoder_destroy(self.dec) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real encoder, so the tests hear what the phone's would send.
    struct Enc(*mut libopus::OpusEncoder);

    impl Enc {
        fn new() -> Self {
            let mut err = 0;
            let st = unsafe {
                libopus::opus_encoder_create(48_000, 2, libopus::OPUS_APPLICATION_AUDIO, &mut err)
            };
            assert_eq!(err, 0);
            Self(st)
        }

        fn encode(&mut self, pcm: &[i16]) -> Vec<u8> {
            let mut out = vec![0u8; 1275];
            let n = unsafe {
                libopus::opus_encode(
                    self.0,
                    pcm.as_ptr(),
                    (pcm.len() / 2) as i32,
                    out.as_mut_ptr(),
                    1275,
                )
            };
            assert!(n > 0, "encode failed: {n}");
            out.truncate(n as usize);
            out
        }
    }

    impl Drop for Enc {
        fn drop(&mut self) {
            unsafe { libopus::opus_encoder_destroy(self.0) };
        }
    }

    /// 20 ms of a stereo tone starting at sample `at`, peak 8000.
    fn tone(at: usize) -> Vec<i16> {
        (at..at + 960)
            .flat_map(|i| {
                let v = ((i as f32 / 109.0 * std::f32::consts::TAU).sin() * 8000.0) as i16;
                [v, v]
            })
            .collect()
    }

    fn as_i16(b: &[u8]) -> Vec<i16> {
        b.as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect()
    }

    fn peak(s: &[i16]) -> i32 {
        s.iter().map(|v| i32::from(*v).abs()).max().unwrap_or(0)
    }

    #[test]
    fn a_tone_survives_the_round_trip_at_full_length() {
        let (mut e, mut d) = (Enc::new(), OpusDec::new(48_000, 2).unwrap());
        let mut last = Vec::new();
        for f in 0..20 {
            last = d.decode(&e.encode(&tone(f * 960)));
            assert_eq!(last.len(), 3840, "20 ms of 16-bit stereo");
        }
        let p = peak(&as_i16(&last));
        assert!((6000..=10000).contains(&p), "peak {p}, sent 8000");
        assert_eq!(d.bad_packets, 0);
    }

    #[test]
    fn a_lost_packet_is_concealed_at_full_length() {
        let (mut e, mut d) = (Enc::new(), OpusDec::new(48_000, 2).unwrap());
        for f in 0..20 {
            d.decode(&e.encode(&tone(f * 960)));
        }
        let hidden = d.decode(&[]);
        assert_eq!(hidden.len(), 3840, "playback keeps its timing");
        assert!(peak(&as_i16(&hidden)) > 0, "concealment, not a hole");
    }

    #[test]
    fn garbage_never_panics_and_is_counted_or_concealed() {
        let mut d = OpusDec::new(48_000, 2).unwrap();
        for seed in 0..=255u8 {
            for len in [1usize, 2, 3, 7, 60, 400, 1275] {
                let junk: Vec<u8> = (0..len)
                    .map(|i| (i as u8).wrapping_mul(seed) ^ seed)
                    .collect();
                let out = d.decode(&junk);
                assert!(out.len() <= MAX_SAMPLES_PER_CHANNEL * 2 * 2);
            }
        }
    }

    /// 20 ms that is hard to compress, like dense music: tones plus noise.
    fn dense(at: usize, seed: &mut u32) -> Vec<i16> {
        (at..at + 960)
            .flat_map(|i| {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (*seed >> 16) as i16 / 8;
                let t = i as f32 / 48_000.0 * std::f32::consts::TAU;
                let v = ((t * 220.0).sin() + (t * 1760.0).sin() + (t * 7040.0).sin()) * 3000.0;
                [v as i16 + noise, (v * 0.7) as i16 - noise]
            })
            .collect()
    }

    #[test]
    #[ignore = "timing, run with a release profile"]
    fn decoding_at_the_real_bitrate_is_far_faster_than_real_time() {
        let mut e = Enc::new();
        unsafe {
            libopus::opus_encoder_ctl!(e.0, libopus::OPUS_SET_BITRATE_REQUEST, 160_000i32);
        }
        let mut seed = 1;
        let packets: Vec<Vec<u8>> = (0..500)
            .map(|f| e.encode(&dense(f * 960, &mut seed)))
            .collect();
        let size: usize = packets.iter().map(Vec::len).sum::<usize>() / 500;
        let mut d = OpusDec::new(48_000, 2).unwrap();
        let t = std::time::Instant::now();
        let mut total = 0;
        for p in &packets {
            total += d.decode(p).len();
        }
        let per = t.elapsed() / 500;
        eprintln!("{size} bytes per packet, decode {per:?}");
        assert_eq!(total, 500 * 3840, "every packet decoded in full");
        assert!(per.as_millis() < 5, "{per:?}");
    }

    #[test]
    fn a_bad_format_is_an_error() {
        assert!(OpusDec::new(44_100, 2).is_err());
        assert!(OpusDec::new(48_000, 3).is_err());
    }
}
