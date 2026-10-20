//! Codec negotiation: the PC lists what it can decode, best first; the phone
//! uses the first of those it can encode.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Codec {
    /// HEVC Main10 (10-bit: no added banding in gradients).
    H265Main10 = 1,
    H265 = 2,
    H264 = 3,
}

impl Codec {
    pub fn from_u32(v: u32) -> Option<Codec> {
        match v {
            1 => Some(Self::H265Main10),
            2 => Some(Self::H265),
            3 => Some(Self::H264),
            _ => None,
        }
    }
}

pub fn choose(pc: &[Codec], phone: &[Codec]) -> Option<Codec> {
    pc.iter().copied().find(|c| phone.contains(c))
}

/// The PC's next choice after `failed` (decoding failed), if any.
pub fn next_after(failed: Codec, pc: &[Codec]) -> Option<Codec> {
    let i = pc.iter().position(|c| *c == failed)?;
    pc.get(i + 1).copied()
}

#[cfg(test)]
mod tests {
    use super::{Codec::*, *};

    #[test]
    fn codec_choice_follows_pc_order() {
        assert_eq!(choose(&[H265Main10, H265, H264], &[H265, H264]), Some(H265));
        assert_eq!(choose(&[H264], &[H265Main10, H265]), None);
        assert_eq!(Codec::from_u32(1), Some(H265Main10));
        assert_eq!(Codec::from_u32(9), None);
    }

    #[test]
    fn codec_fallback_stops_after_last_codec() {
        let pc = [H265Main10, H265, H264];
        assert_eq!(next_after(H265Main10, &pc), Some(H265));
        assert_eq!(next_after(H264, &pc), None);
    }
}
