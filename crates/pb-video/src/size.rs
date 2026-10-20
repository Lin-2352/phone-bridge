//! Encoder sizes: scale so the long side fits, then round each side to the
//! encoder's alignment (at least 16).

pub fn fit_size(native: (u32, u32), max_long: u32, align: (u32, u32)) -> (u32, u32) {
    let (w, h) = (f64::from(native.0), f64::from(native.1));
    let long = w.max(h);
    let scale = if long > f64::from(max_long) && long > 0.0 {
        f64::from(max_long) / long
    } else {
        1.0
    };
    // Nearest multiple of the alignment, ties up.
    let round = |v: f64, a: u32| {
        let a = f64::from(a.max(16));
        ((v / a + 0.5).floor() * a).max(a) as u32
    };
    (round(w * scale, align.0), round(h * scale, align.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_size_realme_default_and_sharp() {
        assert_eq!(fit_size((1080, 2412), 1920, (16, 16)), (864, 1920));
        assert_eq!(fit_size((1080, 2412), 2412, (16, 16)), (1088, 2416));
        // Landscape keeps its orientation.
        assert_eq!(fit_size((2412, 1080), 1920, (16, 16)), (1920, 864));
        // An alignment under 16 is raised to 16; 0 cannot divide.
        assert_eq!(fit_size((1080, 2412), 1920, (2, 0)), (864, 1920));
        // Never larger than asked when already small.
        assert_eq!(fit_size((720, 1280), 1920, (16, 16)), (720, 1280));
    }
}
