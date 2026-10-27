//! Clipboard image conversion: Windows DIBs to PNG and back. Pure safe Rust,
//! so every odd input is a unit test, and none of it can panic on hostile
//! clipboard content.
//!
//! Accepted DIBs: `BITMAPINFOHEADER` (40) or `BITMAPV5HEADER` (124), 24 or 32
//! bits per pixel, uncompressed (`BI_RGB`) or `BI_BITFIELDS` with the usual BGRA
//! masks. Anything else (palettes, 16 bpp, RLE, JPEG) is refused.

use std::io::Cursor;

/// Widest or tallest image we convert (pixels).
pub const MAX_SIDE: u32 = 8192;
/// Largest PNG we produce or accept; mirrors `pb_protocol::control::MAX_CLIPBOARD_IMAGE`.
pub const MAX_PNG: usize = pb_protocol::control::MAX_CLIPBOARD_IMAGE;

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const HEADER: usize = 40;
const HEADER_V5: usize = 124;

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Most pixels we convert (a 128 MiB RGBA buffer): a small PNG can claim a
/// huge canvas, and a DIB is bounded by what the clipboard holds.
const MAX_PIXELS: u64 = 32 * 1024 * 1024;

fn encode_rgba(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(Cursor::new(&mut out), w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut writer = e.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    if out.len() > MAX_PNG {
        return Err("the image is too large to share".into());
    }
    Ok(out)
}

/// Convert the bytes of a `CF_DIB` / `CF_DIBV5` block to a PNG (always 8-bit
/// RGBA).
pub fn dib_to_png(dib: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "unsupported or damaged bitmap".to_string();
    let size = le32(dib, 0).ok_or_else(bad)? as usize;
    if size < HEADER || size > dib.len() {
        return Err(bad());
    }
    let width = le32(dib, 4).ok_or_else(bad)? as i32;
    let height = le32(dib, 8).ok_or_else(bad)? as i32;
    let bpp = u16::from_le_bytes(
        dib.get(14..16)
            .ok_or_else(bad)?
            .try_into()
            .map_err(|_| bad())?,
    );
    let compression = le32(dib, 16).ok_or_else(bad)?;
    let clr_used = le32(dib, 32).ok_or_else(bad)? as usize;
    if width <= 0 || height == 0 || height == i32::MIN {
        return Err(bad());
    }
    let (w, h) = (width as u32, height.unsigned_abs());
    if w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err("the image is too large to share".into());
    }
    let top_down = height < 0;
    let mut offset = size;
    let mut alpha = false;
    match (bpp, compression) {
        (24, BI_RGB) | (32, BI_RGB) => {
            // A colour table, if present, sits before the pixels.
            offset = offset
                .checked_add(clr_used.min(256).saturating_mul(4))
                .ok_or_else(bad)?;
        }
        (32, BI_BITFIELDS) => {
            let (r, g, b) = (
                le32(dib, 40).ok_or_else(bad)?,
                le32(dib, 44).ok_or_else(bad)?,
                le32(dib, 48).ok_or_else(bad)?,
            );
            if (r, g, b) != (0x00FF_0000, 0x0000_FF00, 0x0000_00FF) {
                return Err(bad());
            }
            if size == HEADER {
                // The masks follow the 40-byte header.
                offset = HEADER + 12;
            } else {
                let a = le32(dib, 52).ok_or_else(bad)?;
                match a {
                    0 => {}
                    0xFF00_0000 => alpha = true,
                    _ => return Err(bad()),
                }
            }
        }
        _ => return Err(bad()),
    }
    let stride = (u64::from(w) * u64::from(bpp)).div_ceil(32) as usize * 4;
    let need = offset
        .checked_add(stride.checked_mul(h as usize).ok_or_else(bad)?)
        .ok_or_else(bad)?;
    if dib.len() < need {
        return Err(bad());
    }
    let px = bpp as usize / 8;
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for r in 0..h as usize {
        let src = if top_down { r } else { h as usize - 1 - r };
        let row = &dib[offset + src * stride..][..stride];
        for x in 0..w as usize {
            let p = &row[x * px..x * px + px];
            let a = if px == 4 && alpha { p[3] } else { 255 };
            rgba.extend([p[2], p[1], p[0], a]);
        }
    }
    encode_rgba(w, h, &rgba)
}

/// Convert a PNG to a `CF_DIBV5` block (32 bpp, bottom-up, straight alpha).
pub fn png_to_dibv5(png: &[u8]) -> Result<Vec<u8>, String> {
    if png.len() > MAX_PNG {
        return Err("the image is too large to share".into());
    }
    let mut dec = png::Decoder::new(Cursor::new(png));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let (w, h) = {
        let i = reader.info();
        (i.width, i.height)
    };
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PIXELS
    {
        return Err("the image is too large to share".into());
    }
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or("image too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("unsupported image".into()),
    };
    let line = info.line_size;
    let mut out = Vec::with_capacity(HEADER_V5 + w as usize * h as usize * 4);
    out.extend((HEADER_V5 as u32).to_le_bytes());
    out.extend((w as i32).to_le_bytes());
    out.extend((h as i32).to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    out.extend(BI_BITFIELDS.to_le_bytes());
    out.extend((w * h * 4).to_le_bytes());
    out.extend([0u8; 16]); // pixels per metre x and y, colours used, important
    out.extend(0x00FF_0000u32.to_le_bytes());
    out.extend(0x0000_FF00u32.to_le_bytes());
    out.extend(0x0000_00FFu32.to_le_bytes());
    out.extend(0xFF00_0000u32.to_le_bytes());
    out.extend(0x7352_4742u32.to_le_bytes()); // LCS_sRGB
    out.extend([0u8; 36 + 12]); // endpoints, gamma
    out.extend(4u32.to_le_bytes()); // intent: pictures
    out.extend([0u8; 12]); // profile data, profile size, reserved
    debug_assert_eq!(out.len(), HEADER_V5);
    for r in (0..h as usize).rev() {
        let row = &buf[r * line..r * line + w as usize * channels];
        for p in row.chunks_exact(channels) {
            let (rr, g, b, a) = match channels {
                1 => (p[0], p[0], p[0], 255),
                2 => (p[0], p[0], p[0], p[1]),
                3 => (p[0], p[1], p[2], 255),
                _ => (p[0], p[1], p[2], p[3]),
            };
            out.extend([b, g, rr, a]);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(png: &[u8]) -> (u32, u32, png::ColorType, Vec<u8>) {
        let mut r = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        buf.truncate(info.buffer_size());
        (info.width, info.height, info.color_type, buf)
    }

    fn header(w: i32, h: i32, bpp: u16, compression: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(40u32.to_le_bytes());
        v.extend(w.to_le_bytes());
        v.extend(h.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(bpp.to_le_bytes());
        v.extend(compression.to_le_bytes());
        v.extend([0u8; 20]);
        v
    }

    #[test]
    fn a_bottom_up_24_bit_dib_becomes_an_rgb_png() {
        // 2x2, stride 8 (6 bytes + 2 padding). Bottom row first, BGR order.
        let mut d = header(2, 2, 24, BI_RGB);
        d.extend([0, 0, 255, 0, 255, 0, 0, 0]); // bottom: red, green
        d.extend([255, 0, 0, 255, 255, 255, 0, 0]); // top: blue, white
        let (w, h, _, px) = decode(&dib_to_png(&d).unwrap());
        assert_eq!((w, h), (2, 2));
        let rgba: Vec<u8> = px;
        // Top row blue, white; bottom row red, green (RGB or RGBA output).
        let n = rgba.len() / 4;
        assert_eq!(n, 4);
        assert_eq!(&rgba[0..3], &[0, 0, 255]);
        assert_eq!(&rgba[4..7], &[255, 255, 255]);
        assert_eq!(&rgba[8..11], &[255, 0, 0]);
        assert_eq!(&rgba[12..15], &[0, 255, 0]);
    }

    #[test]
    fn a_top_down_32_bit_rgb_dib_ignores_the_unused_alpha_byte() {
        let mut d = header(1, -2, 32, BI_RGB);
        d.extend([10, 20, 30, 0]); // top, alpha byte 0 but undefined in BI_RGB
        d.extend([40, 50, 60, 0]);
        let (w, h, ct, px) = decode(&dib_to_png(&d).unwrap());
        assert_eq!((w, h), (1, 2));
        assert_eq!(ct, png::ColorType::Rgba);
        assert_eq!(px, [30, 20, 10, 255, 60, 50, 40, 255]);
    }

    #[test]
    fn png_to_dibv5_and_back_keeps_pixels_and_alpha() {
        let mut enc_buf = Vec::new();
        {
            let mut e = png::Encoder::new(Cursor::new(&mut enc_buf), 3, 2);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            let mut w = e.write_header().unwrap();
            let px: Vec<u8> = vec![
                1, 2, 3, 255, 4, 5, 6, 128, 7, 8, 9, 0, //
                10, 11, 12, 255, 13, 14, 15, 64, 16, 17, 18, 255,
            ];
            w.write_image_data(&px).unwrap();
        }
        let dib = png_to_dibv5(&enc_buf).unwrap();
        assert_eq!(le32(&dib, 0), Some(HEADER_V5 as u32));
        let (w, h, _, px) = decode(&dib_to_png(&dib).unwrap());
        assert_eq!((w, h), (3, 2));
        assert_eq!(
            px,
            vec![
                1, 2, 3, 255, 4, 5, 6, 128, 7, 8, 9, 0, //
                10, 11, 12, 255, 13, 14, 15, 64, 16, 17, 18, 255
            ]
        );
    }

    #[test]
    fn a_grayscale_png_converts() {
        let mut enc_buf = Vec::new();
        {
            let mut e = png::Encoder::new(Cursor::new(&mut enc_buf), 2, 1);
            e.set_color(png::ColorType::Grayscale);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()
                .unwrap()
                .write_image_data(&[0, 200])
                .unwrap();
        }
        let dib = png_to_dibv5(&enc_buf).unwrap();
        let (_, _, _, px) = decode(&dib_to_png(&dib).unwrap());
        assert_eq!(&px[..3], &[0, 0, 0]);
        assert_eq!(&px[4..7], &[200, 200, 200]);
    }

    #[test]
    fn unsupported_or_broken_dibs_are_refused_not_panicked_on() {
        assert!(dib_to_png(&[]).is_err());
        assert!(dib_to_png(&[0; 10]).is_err());
        // Zero width, palette (8 bpp), 16 bpp, RLE compression.
        assert!(dib_to_png(&header(0, 1, 24, BI_RGB)).is_err());
        let mut p = header(1, 1, 8, BI_RGB);
        p.extend([0; 1024]);
        assert!(dib_to_png(&p).is_err());
        let mut p = header(1, 1, 16, BI_RGB);
        p.extend([0; 8]);
        assert!(dib_to_png(&p).is_err());
        let mut p = header(1, 1, 24, 1);
        p.extend([0; 8]);
        assert!(dib_to_png(&p).is_err());
        // Pixel data shorter than the header promises.
        let mut p = header(100, 100, 24, BI_RGB);
        p.extend([0; 50]);
        assert!(dib_to_png(&p).is_err());
        // Beyond the side limit, and an i32::MIN height.
        let mut p = header((MAX_SIDE + 1) as i32, 1, 24, BI_RGB);
        p.extend(vec![0; 3 * 8200]);
        assert!(dib_to_png(&p).is_err());
        assert!(dib_to_png(&header(1, i32::MIN, 24, BI_RGB)).is_err());
        // BI_BITFIELDS with masks that are not BGRA.
        let mut p = header(1, 1, 32, BI_BITFIELDS);
        p.extend(0x0000_00FFu32.to_le_bytes());
        p.extend(0x0000_FF00u32.to_le_bytes());
        p.extend(0x00FF_0000u32.to_le_bytes());
        p.extend([0; 4]);
        assert!(dib_to_png(&p).is_err());
    }

    #[test]
    fn corrupt_png_is_refused() {
        assert!(png_to_dibv5(&[]).is_err());
        assert!(png_to_dibv5(b"not a png at all").is_err());
        let mut good = Vec::new();
        {
            let mut e = png::Encoder::new(Cursor::new(&mut good), 4, 4);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()
                .unwrap()
                .write_image_data(&[7; 48])
                .unwrap();
        }
        assert!(png_to_dibv5(&good[..good.len() / 2]).is_err());
    }

    #[test]
    fn random_bytes_never_panic() {
        let mut x = 0x1234_5678u32;
        for round in 0..300 {
            let n = 1 + (round * 7) % 200;
            let mut b: Vec<u8> = (0..n)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x as u8
                })
                .collect();
            // Half the rounds start with a plausible header size.
            if round % 2 == 0 && b.len() >= 4 {
                b[..4].copy_from_slice(&40u32.to_le_bytes());
            }
            let _ = dib_to_png(&b);
            let _ = png_to_dibv5(&b);
        }
    }
}
