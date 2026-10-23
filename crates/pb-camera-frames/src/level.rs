//! How bright the picture is (the Y plane of NV12), sampled: enough to tell
//! "the phone sends black" from "the picture is there".

/// Studio-range black is Y = 16.
pub const DARK_LUMA: u8 = 20;

/// Mean of the Y plane, sampled (not every pixel).
pub fn mean_luma(nv12: &[u8], w: usize, h: usize) -> u8 {
    let y_len = (w * h).min(nv12.len());
    let y = &nv12[..y_len];
    if y.is_empty() {
        return 0;
    }
    // About a thousand samples spread over the plane (a prime step, so rows
    // and columns are both covered).
    let step = (y.len() / 1000).max(1) | 1;
    let (mut sum, mut n) = (0u64, 0u64);
    let mut i = 0;
    while i < y.len() {
        sum += u64::from(y[i]);
        n += 1;
        i += step;
    }
    (sum / n) as u8
}

/// The picture is (nearly) black.
pub fn is_dark(mean: u8) -> bool {
    mean <= DARK_LUMA
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: usize, h: usize, y: u8) -> Vec<u8> {
        let mut v = vec![y; w * h];
        v.resize(w * h * 3 / 2, 128);
        v
    }

    #[test]
    fn a_black_frame_is_dark() {
        let f = frame(1920, 1080, 16);
        assert_eq!(mean_luma(&f, 1920, 1080), 16);
        assert!(is_dark(mean_luma(&f, 1920, 1080)));
    }

    #[test]
    fn a_normal_picture_is_not_dark() {
        let f = frame(1280, 720, 120);
        assert_eq!(mean_luma(&f, 1280, 720), 120);
        assert!(!is_dark(mean_luma(&f, 1280, 720)));
    }

    #[test]
    fn chroma_does_not_count_as_brightness() {
        // Y black, UV bright: still dark.
        let mut f = vec![16u8; 640 * 360];
        f.resize(640 * 360 * 3 / 2, 240);
        assert!(is_dark(mean_luma(&f, 640, 360)));
    }

    #[test]
    fn a_short_buffer_is_not_read_past_its_end() {
        assert_eq!(mean_luma(&[], 1920, 1080), 0);
        assert_eq!(mean_luma(&[50u8; 10], 1920, 1080), 50);
    }

    #[test]
    fn half_bright_half_black_is_in_between() {
        let (w, h) = (640usize, 360usize);
        let mut f = vec![16u8; w * h];
        for y in 0..h / 2 {
            f[y * w..(y + 1) * w].fill(200);
        }
        f.resize(w * h * 3 / 2, 128);
        let m = mean_luma(&f, w, h);
        assert!((100..=116).contains(&m), "{m}");
        assert!(!is_dark(m));
    }
}
