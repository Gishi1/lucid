//! Hand-encoded 24-bit BMP files — used for the demo package's `assets/logo.bmp`.
//!
//! MSHC assets are resolved case-insensitively, and BMP is the one image format the demo can
//! produce without pulling in an encoder crate, so the packer writes the file itself.

use crate::error::Error;

/// Bytes of the `BITMAPFILEHEADER` plus `BITMAPINFOHEADER` that precede the pixel data.
const HEADER_BYTES: usize = 14 + 40;

/// Encode a 24-bit BMP from `width * height` RGB triples (row-major, top row first).
///
/// BMP stores rows bottom-up and each row is padded to a 4-byte boundary; the file uses no
/// compression and 72 dpi, which is what every decoder expects.
///
/// # Errors
///
/// Returns [`Error::Input`] when a dimension is zero, when the pixel buffer does not contain
/// exactly `width * height * 3` bytes, or when the image would be larger than 4 GiB.
pub fn encode_rgb24(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>, Error> {
    if width == 0 || height == 0 {
        return Err(Error::Input(format!(
            "a {width}x{height} image has no pixels"
        )));
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or_else(|| Error::Input(format!("{width}x{height} is too large to encode")))?;
    if rgb.len() != expected {
        return Err(Error::Input(format!(
            "expected {expected} bytes of RGB data for {width}x{height}, got {}",
            rgb.len()
        )));
    }
    if HEADER_BYTES as u64 + row_bytes(width) as u64 * u64::from(height) > u64::from(u32::MAX) {
        return Err(Error::Input(format!(
            "a {width}x{height} bitmap does not fit in a BMP file"
        )));
    }
    Ok(encode(width, height, rgb))
}

/// The demo package's logo: a slate-to-teal gradient with a light `L` motif and a dark border.
///
/// Deterministic — the same arguments always produce the same bytes.
pub fn logo(width: u32, height: u32) -> Vec<u8> {
    let width = width.max(1);
    let height = height.max(1);
    let (w, h) = (width as usize, height as usize);
    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let fx = (x as u32 * 255) / (width - 1).max(1);
            let fy = (y as u32 * 255) / (height - 1).max(1);
            let mut red = 18 + fx * 26 / 255 + fy * 8 / 255;
            let mut green = 32 + fx * 96 / 255 + fy * 12 / 255;
            let mut blue = 48 + fx * 70 / 255 + fy * 24 / 255;
            let column = (x as u32 * 100) / width;
            let row = (y as u32 * 100) / height;
            let stem = (18..=26).contains(&column) && (20..=80).contains(&row);
            let foot = (18..=54).contains(&column) && (70..=80).contains(&row);
            if stem || foot {
                red = 238;
                green = 244;
                blue = 250;
            }
            if x < 2 || y < 2 || x + 2 >= w || y + 2 >= h {
                red = 10;
                green = 16;
                blue = 26;
            }
            let offset = (y * w + x) * 3;
            rgb[offset] = clamp_byte(red);
            rgb[offset + 1] = clamp_byte(green);
            rgb[offset + 2] = clamp_byte(blue);
        }
    }
    encode(width, height, &rgb)
}

fn clamp_byte(value: u32) -> u8 {
    value.min(255) as u8
}

fn byte_at(rgb: &[u8], offset: usize) -> u8 {
    rgb.get(offset).copied().unwrap_or(0)
}

fn row_bytes(width: u32) -> usize {
    (width as usize * 3).div_ceil(4) * 4
}

/// Encode without validation: missing pixels read as black, extra pixels are ignored.
fn encode(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let stride = row_bytes(width);
    let image_size = stride * h;
    let file_size = HEADER_BYTES + image_size;
    let mut out = Vec::with_capacity(file_size);

    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
    // BITMAPINFOHEADER
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(image_size as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    for y in (0..h).rev() {
        for x in 0..w {
            let offset = (y * w + x) * 3;
            out.push(byte_at(rgb, offset + 2));
            out.push(byte_at(rgb, offset + 1));
            out.push(byte_at(rgb, offset));
        }
        out.resize(out.len() + (stride - w * 3), 0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_matches_the_bmp_spec() {
        let bytes = encode_rgb24(3, 2, &[0u8; 18]).unwrap();
        assert_eq!(&bytes[..2], b"BM");
        // 3 px * 3 B = 9 B per row, padded to 12 B (4-byte boundary), two rows.
        assert_eq!(bytes.len(), HEADER_BYTES + 12 * 2);
        let file_size = u32::from_le_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
        assert_eq!(file_size as usize, bytes.len());
        let data_offset = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        assert_eq!(data_offset, HEADER_BYTES as u32);
        let width = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
        let height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!((width, height), (3, 2));
        let planes = u16::from_le_bytes([bytes[26], bytes[27]]);
        let depth = u16::from_le_bytes([bytes[28], bytes[29]]);
        assert_eq!((planes, depth), (1, 24));
    }

    #[test]
    fn rows_are_stored_bottom_up_in_bgr() {
        // Three rows of two pixels: red on top, blue below.
        let pixels = [
            255, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 255,
        ];
        let bytes = encode_rgb24(2, 3, &pixels).unwrap();
        let data = &bytes[HEADER_BYTES..];
        let stride = 8; // 6 bytes of pixels padded to 8
        assert_eq!(&data[..3], &[255, 0, 0]); // bottom row first: BGR of blue
        assert_eq!(&data[6..8], &[0, 0]); // row padding
        assert_eq!(&data[stride * 2..stride * 2 + 3], &[0, 0, 255]); // top row: BGR of red
        assert_eq!(data.len(), stride * 3);
    }

    #[test]
    fn wrong_buffer_size_is_an_error() {
        assert!(encode_rgb24(4, 4, &[0u8; 4]).is_err());
        assert!(encode_rgb24(0, 4, &[]).is_err());
    }

    #[test]
    fn logo_is_deterministic_and_well_sized() {
        let first = logo(320, 120);
        assert_eq!(first, logo(320, 120));
        assert_eq!(first.len(), HEADER_BYTES + 320 * 3 * 120);
        assert_eq!(&first[..2], b"BM");
        assert!(first.iter().any(|byte| *byte > 200), "the motif is light");
    }
}
