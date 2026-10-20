//! NV12 readback from a GPU texture: rows are `row_pitch` apart there.

/// `h` rows of `w` Y bytes, then `h / 2` rows of `w` UV bytes, each source
/// row read at `row_pitch * row`; `dst` ends up tightly packed.
pub fn copy_nv12(
    src_y: &[u8],
    src_uv: &[u8],
    row_pitch: usize,
    w: usize,
    h: usize,
    dst: &mut [u8],
) {
    for r in 0..h {
        dst[r * w..(r + 1) * w].copy_from_slice(&src_y[r * row_pitch..r * row_pitch + w]);
    }
    let uv_dst = &mut dst[w * h..];
    for r in 0..h / 2 {
        uv_dst[r * w..(r + 1) * w].copy_from_slice(&src_uv[r * row_pitch..r * row_pitch + w]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_pitch_copy_strips_padding() {
        let (w, h, pitch, padded_h) = (1920usize, 1080usize, 2048usize, 1088usize);
        let mut y = vec![0xEEu8; pitch * padded_h];
        let mut uv = vec![0xEEu8; pitch * (padded_h / 2)];
        for r in 0..h {
            y[r * pitch..r * pitch + w].fill((r % 200) as u8);
        }
        for r in 0..h / 2 {
            uv[r * pitch..r * pitch + w].fill((20 + r % 200) as u8);
        }
        let mut dst = vec![0u8; w * h * 3 / 2];
        copy_nv12(&y, &uv, pitch, w, h, &mut dst);
        assert_eq!(dst.len(), w * h * 3 / 2);
        for r in 0..h {
            assert!(
                dst[r * w..(r + 1) * w]
                    .iter()
                    .all(|b| *b == (r % 200) as u8),
                "Y row {r}"
            );
        }
        for r in 0..h / 2 {
            let row = &dst[w * h + r * w..w * h + (r + 1) * w];
            assert!(row.iter().all(|b| *b == (20 + r % 200) as u8), "UV row {r}");
        }
        assert!(!dst.contains(&0xEE), "no padding byte reaches the output");
    }
}
