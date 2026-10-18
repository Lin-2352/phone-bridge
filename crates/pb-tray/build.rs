//! Generates the app icon (`app.ico`, 16/24/32/48/64/256 px) from the design
//! glyph, two overlapping tiles (phone in front of a screen) on a cobalt
//! rounded square, and embeds it as resource 1. Drawn procedurally with 4x4
//! supersampling so there is no binary asset to keep in sync.

use std::{env, fs, path::PathBuf};

/// Signed distance-ish coverage test for a rounded rectangle (unit square coords).
fn in_rrect(x: f32, y: f32, l: f32, t: f32, r: f32, b: f32, rad: f32) -> bool {
    if x < l || x > r || y < t || y > b {
        return false;
    }
    let cx = x.clamp(l + rad, r - rad);
    let cy = y.clamp(t + rad, b - rad);
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= rad * rad
}

/// Premultiplied-free BGRA pixel for (x, y) in [0,1]^2.
fn sample(x: f32, y: f32) -> [f32; 4] {
    // Background tile: cobalt #2446C9, corner radius 22%.
    if !in_rrect(x, y, 0.0, 0.0, 1.0, 1.0, 0.22) {
        return [0.0; 4];
    }
    let bg = [0xC9 as f32, 0x46 as f32, 0x24 as f32, 255.0];
    let white = [0xFF as f32, 0xF7 as f32, 0xF5 as f32]; // BGR of #F5F7FF
                                                         // Glyph in a 24-unit grid, scaled into the middle 70% of the tile.
    let g = |v: f32| (v - 0.15) / 0.70 * 24.0;
    let (gx, gy) = (g(x), g(y));
    let strong = in_rrect(gx, gy, 2.5, 8.0, 10.5, 20.5, 2.0);
    let hole = (gx - 6.5).powi(2) + (gy - 17.6).powi(2) <= 0.9 * 0.9;
    let soft = in_rrect(gx, gy, 8.0, 4.0, 22.0, 14.5, 2.0);
    let mix = |a: f32, c: [f32; 3]| {
        [
            bg[0] + (c[0] - bg[0]) * a,
            bg[1] + (c[1] - bg[1]) * a,
            bg[2] + (c[2] - bg[2]) * a,
            255.0,
        ]
    };
    if strong && !hole {
        mix(1.0, white)
    } else if soft && !strong {
        mix(0.5, white)
    } else {
        bg
    }
}

fn render(size: u32) -> Vec<u8> {
    const SS: u32 = 4;
    let mut px = Vec::with_capacity((size * size * 4) as usize);
    // ICO bitmaps are stored bottom-up.
    for row in (0..size).rev() {
        for col in 0..size {
            let mut acc = [0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (col as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let y = (row as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    let s = sample(x, y);
                    // accumulate premultiplied colour so edges blend correctly
                    let a = s[3] / 255.0;
                    acc[0] += s[0] * a;
                    acc[1] += s[1] * a;
                    acc[2] += s[2] * a;
                    acc[3] += s[3];
                }
            }
            let n = (SS * SS) as f32;
            let a = acc[3] / n;
            let un = |c: f32| if a > 0.0 { (c / n) / (a / 255.0) } else { 0.0 };
            px.extend_from_slice(&[
                un(acc[0]) as u8,
                un(acc[1]) as u8,
                un(acc[2]) as u8,
                a as u8,
            ]);
        }
    }
    px
}

fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let mut bmp = Vec::new();
            // BITMAPINFOHEADER; height is doubled (XOR + AND masks).
            bmp.extend_from_slice(&40u32.to_le_bytes());
            bmp.extend_from_slice(&(s as i32).to_le_bytes());
            bmp.extend_from_slice(&((s * 2) as i32).to_le_bytes());
            bmp.extend_from_slice(&1u16.to_le_bytes());
            bmp.extend_from_slice(&32u16.to_le_bytes());
            bmp.extend_from_slice(&[0u8; 24]);
            bmp.extend(render(s));
            // AND mask: all zero (alpha channel is used), rows padded to 32 bits.
            let mask_row = s.div_ceil(32) * 4;
            bmp.extend(std::iter::repeat_n(0u8, (mask_row * s) as usize));
            bmp
        })
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend(img);
    }
    out
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let ico_path = out.join("app.ico");
    fs::write(&ico_path, ico(&[16, 24, 32, 48, 64, 256])).unwrap();
    let rc = out.join("app.rc");
    fs::write(
        &rc,
        format!(
            "1 ICON \"{}\"\n",
            ico_path.display().to_string().replace('\\', "\\\\")
        ),
    )
    .unwrap();
    embed_resource::compile(&rc, embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
