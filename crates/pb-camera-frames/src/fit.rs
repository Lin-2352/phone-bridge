//! Fitting the phone's picture into the fixed 16:9 output frame.

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// `src` rotated by `rotation` (0/90/180/270), scaled to fit inside `out`
/// keeping its aspect, centred; sizes even (NV12 chroma).
pub fn fit(src: (u32, u32), rotation: u32, out: (u32, u32)) -> Rect {
    let (sw, sh) = if rotation % 180 == 90 {
        (src.1, src.0)
    } else {
        src
    };
    let (ow, oh) = out;
    if sw == 0 || sh == 0 {
        return Rect {
            x: 0,
            y: 0,
            w: ow,
            h: oh,
        };
    }
    let even = |v: f64| (2.0 * (v / 2.0).round()) as u32;
    let (w, h) = if u64::from(sw) * u64::from(oh) >= u64::from(sh) * u64::from(ow) {
        (
            ow,
            even(f64::from(sh) * f64::from(ow) / f64::from(sw)).min(oh),
        )
    } else {
        (
            even(f64::from(sw) * f64::from(oh) / f64::from(sh)).min(ow),
            oh,
        )
    };
    Rect {
        x: ((ow - w) / 2) & !1,
        y: ((oh - h) / 2) & !1,
        w,
        h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_portrait_is_pillarboxed() {
        assert_eq!(
            fit((1080, 1920), 0, (1920, 1080)),
            Rect {
                x: 656,
                y: 0,
                w: 608,
                h: 1080
            }
        );
    }

    #[test]
    fn fit_landscape_fills() {
        assert_eq!(
            fit((1920, 1080), 0, (1280, 720)),
            Rect {
                x: 0,
                y: 0,
                w: 1280,
                h: 720
            }
        );
    }

    #[test]
    fn fit_rotation_90_swaps() {
        assert_eq!(
            fit((1920, 1080), 90, (1920, 1080)),
            fit((1080, 1920), 0, (1920, 1080))
        );
    }
}
