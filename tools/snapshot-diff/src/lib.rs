//! Compares two snapshot renders pixel by pixel. A render is raw RGBA with a
//! 12-byte header (width, height, channels as little-endian u32), so the
//! comparison needs no image crate.

/// A channel may differ by this much (0..=255) before a pixel counts as different.
pub const CHANNEL_TOLERANCE: u8 = 8;
/// At most this share of pixels may differ, so a few antialiasing changes pass.
pub const MAX_DIFFERING_FRACTION: f64 = 0.005;

/// Parses a render, returning (width, height, pixels).
pub fn parse(bytes: &[u8]) -> Result<(u32, u32, &[u8]), String> {
    if bytes.len() < 12 {
        return Err("shorter than the header".into());
    }
    let u = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let (w, h, ch) = (u(0), u(4), u(8));
    if ch != 4 {
        return Err(format!("{ch} channels, expected 4 (RGBA)"));
    }
    let want = w as usize * h as usize * 4;
    let px = &bytes[12..];
    if px.len() != want {
        return Err(format!("{} pixel bytes, expected {want}", px.len()));
    }
    Ok((w, h, px))
}

/// Ok when the two renders have the same size and at most `MAX_DIFFERING_FRACTION` of their pixels differ
/// by more than `CHANNEL_TOLERANCE` on any channel.
pub fn compare(actual: &[u8], golden: &[u8]) -> Result<(), String> {
    let (wa, ha, a) = parse(actual)?;
    let (wg, hg, g) = parse(golden)?;
    if (wa, ha) != (wg, hg) {
        return Err(format!("size {wa}x{ha}, golden {wg}x{hg}"));
    }
    let pixels = (wa as usize * ha as usize).max(1);
    let (a, g) = (a.as_chunks::<4>().0, g.as_chunks::<4>().0);
    let differing = a
        .iter()
        .zip(g.iter())
        .filter(|(p, q)| {
            p.iter()
                .zip(q.iter())
                .any(|(x, y)| x.abs_diff(*y) > CHANNEL_TOLERANCE)
        })
        .count();
    let fraction = differing as f64 / pixels as f64;
    if fraction > MAX_DIFFERING_FRACTION {
        return Err(format!(
            "{:.2}% of pixels differ (limit {:.2}%)",
            fraction * 100.0,
            MAX_DIFFERING_FRACTION * 100.0
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(w: u32, h: u32, f: impl Fn(usize) -> u8) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend((0..(w * h * 4) as usize).map(f));
        v
    }

    #[test]
    fn diff_accepts_within_tolerance_and_rejects_beyond() {
        let golden = render(100, 100, |_| 128);
        // Noise of exactly the tolerance on every pixel passes.
        let noisy = render(100, 100, |i| 128 + CHANNEL_TOLERANCE * (i % 2) as u8);
        assert!(compare(&noisy, &golden).is_ok());
        // A shift of 9 on 1% of the pixels fails (the limit is 0.5%).
        let shifted = render(100, 100, |i| if (i / 4) % 100 == 0 { 137 } else { 128 });
        // One pixel in every hundred: 100 of 10_000 pixels, 1%.
        assert!(compare(&shifted, &golden).is_err());
    }

    #[test]
    fn a_different_size_is_rejected() {
        assert!(compare(&render(10, 10, |_| 0), &render(10, 11, |_| 0)).is_err());
    }

    #[test]
    fn a_short_or_wrong_format_render_is_rejected() {
        assert!(parse(&[0; 4]).is_err());
        let mut bad = render(2, 2, |_| 0);
        bad[8] = 3;
        assert!(parse(&bad).is_err());
        let mut short = render(2, 2, |_| 0);
        short.pop();
        assert!(parse(&short).is_err());
    }
}
