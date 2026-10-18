//! Pairing-code QR matrix (no drawing here, so it is testable everywhere).

use qrcode::{EcLevel, QrCode};

/// Square module matrix, `true` = dark. Includes no quiet zone; the renderer adds it.
pub struct Matrix {
    pub size: usize,
    pub dark: Vec<bool>,
}

impl Matrix {
    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        self.dark[y * self.size + x]
    }
}

/// Encode the pairing code. Medium error correction keeps the symbol small
/// enough to scan off a laptop screen while tolerating glare.
pub fn encode(text: &str) -> Result<Matrix, qrcode::types::QrError> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)?;
    let size = code.width();
    let dark = code
        .to_colors()
        .into_iter()
        .map(|c| c == qrcode::Color::Dark)
        .collect();
    Ok(Matrix { size, dark })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render with a quiet zone at `scale` px per module and decode with an
    /// independent decoder.
    fn decode(m: &Matrix, scale: usize) -> String {
        let quiet = 4;
        let px = (m.size + 2 * quiet) * scale;
        let mut img = rqrr::PreparedImage::prepare_from_greyscale(px, px, |x, y| {
            let (mx, my) = (x / scale, y / scale);
            let dark = mx >= quiet
                && my >= quiet
                && mx < m.size + quiet
                && my < m.size + quiet
                && m.is_dark(mx - quiet, my - quiet);
            if dark {
                0
            } else {
                255
            }
        });
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1);
        grids[0].decode().unwrap().1
    }

    #[test]
    fn realistic_pairing_codes_round_trip() {
        let id = "ab".repeat(32);
        let tok = "cd".repeat(16);
        // Short (one address) and long (many IPv6 addresses) codes.
        let short = format!("pb1:{id}:{tok}:10.107.168.180:47200");
        let long = format!(
            "pb1:{id}:{tok}:10.107.168.180:47200,192.168.1.20:47200,[2409:40e4:2059:8536::36]:49267,[2409:40e4:2059:8536:8000::]:49267,[fe80::1234:5678:9abc:def0]:47200"
        );
        for text in [short, long] {
            let m = encode(&text).unwrap();
            assert_eq!(decode(&m, 4), text);
        }
    }
}
