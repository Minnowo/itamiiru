//! A small BMP decoder for thumbnails: uncompressed 1, 4, 8, 16, 24 and
//! 32 bits per pixel, with or without bit masks. No RLE, no OS/2 headers.
//! The file is untrusted: sizes are checked before any row is read.

/// What the header says, checked enough that decoding can't go out of bounds.
#[derive(Debug, PartialEq, Eq)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    /// Rows are stored top to bottom (the height field was negative).
    top_down: bool,
    bits: u16,
    pixel_offset: usize,
    stride: usize,
    /// Red, green, blue and alpha masks, for 16 and 32 bits.
    masks: [u32; 4],
    /// Blue, green, red entries, for 1, 4 and 8 bits.
    palette: Vec<[u8; 3]>,
}

const FILE_HEADER: usize = 14;
const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;

fn u16_at(data: &[u8], pos: usize) -> Option<u16> {
    let b = data.get(pos..pos + 2)?;
    return Some(u16::from_le_bytes([b[0], b[1]]));
}

fn u32_at(data: &[u8], pos: usize) -> Option<u32> {
    let b = data.get(pos..pos + 4)?;
    return Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
}

/// Reads and checks the header. None if this isn't a BMP we can decode, or
/// if the pixel data it describes doesn't fit in the file.
pub fn read_header(data: &[u8]) -> Option<Header> {
    if !data.starts_with(b"BM") {
        return None;
    }
    let pixel_offset = u32_at(data, 10)? as usize;
    let header_size = u32_at(data, 14)? as usize;
    if header_size < 40 {
        return None;
    }
    let width = u32_at(data, 18)? as i32;
    let height = u32_at(data, 22)? as i32;
    let bits = u16_at(data, 28)?;
    let compression = u32_at(data, 30)?;
    let colors_used = u32_at(data, 46)?;
    if width <= 0 || height == 0 || height == i32::MIN {
        return None;
    }
    let width = width as u32;
    let top_down = height < 0;
    let height = height.unsigned_abs();

    let masks = match (compression, bits) {
        (BI_RGB, 16) => [0x7C00, 0x03E0, 0x001F, 0],
        // The fourth byte of 32-bit BI_RGB is usually junk, so no alpha.
        (BI_RGB, 32) => [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0],
        (BI_RGB, 1 | 4 | 8 | 24) => [0; 4],
        (BI_BITFIELDS | BI_ALPHABITFIELDS, 16 | 32) => {
            // The masks come right after the 40-byte header, or inside a
            // longer one; either way they start at the same place.
            let at = FILE_HEADER + 40;
            let has_alpha = header_size >= 56 || compression == BI_ALPHABITFIELDS;
            let alpha = if has_alpha { u32_at(data, at + 12)? } else { 0 };
            [
                u32_at(data, at)?,
                u32_at(data, at + 4)?,
                u32_at(data, at + 8)?,
                alpha,
            ]
        }
        _ => return None,
    };

    let mut palette = Vec::new();
    if bits <= 8 {
        let max_colors = 1usize << bits;
        let count = if colors_used == 0 || colors_used as usize > max_colors {
            max_colors
        } else {
            colors_used as usize
        };
        let start = FILE_HEADER + header_size;
        // Some files leave out unused entries without saying so.
        let room = pixel_offset.saturating_sub(start) / 4;
        for i in 0..count.min(room) {
            let entry = data.get(start + i * 4..start + i * 4 + 3)?;
            palette.push([entry[0], entry[1], entry[2]]);
        }
    }

    // Each row is padded to a multiple of 4 bytes.
    let stride = (width as u64 * bits as u64).div_ceil(32) * 4;
    let needed = stride.checked_mul(height as u64)?;
    if (pixel_offset as u64).checked_add(needed)? > data.len() as u64 {
        return None;
    }
    return Some(Header {
        width,
        height,
        top_down,
        bits,
        pixel_offset,
        stride: stride as usize,
        masks,
        palette,
    });
}

/// Scales `value & mask` to 0-255.
fn channel(value: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = (mask >> shift) as u64;
    let v = ((value & mask) >> shift) as u64;
    return ((v * 255 + max / 2) / max) as u8;
}

/// Calls `row` with each row as RGBA pixels, top to bottom.
pub fn decode_rows(data: &[u8], header: &Header, row: &mut dyn FnMut(&[u8])) {
    let width = header.width as usize;
    let mut out = vec![0u8; width * 4];
    for y in 0..header.height as usize {
        // Bottom-up files store the last row first.
        let stored = if header.top_down {
            y
        } else {
            header.height as usize - 1 - y
        };
        let start = header.pixel_offset + stored * header.stride;
        // read_header checked that every row fits.
        let src = &data[start..start + header.stride];
        for x in 0..width {
            let pixel = match header.bits {
                1 | 4 | 8 => {
                    let bits = header.bits as usize;
                    let bit = x * bits;
                    let byte = src[bit / 8];
                    let index = (byte >> (8 - bits - bit % 8)) & ((1u16 << bits) - 1) as u8;
                    match header.palette.get(index as usize) {
                        Some([b, g, r]) => [*r, *g, *b, 255],
                        None => [0, 0, 0, 255],
                    }
                }
                24 => [src[x * 3 + 2], src[x * 3 + 1], src[x * 3], 255],
                _ => {
                    let value = if header.bits == 16 {
                        u16::from_le_bytes([src[x * 2], src[x * 2 + 1]]) as u32
                    } else {
                        let p = x * 4;
                        u32::from_le_bytes([src[p], src[p + 1], src[p + 2], src[p + 3]])
                    };
                    let [r, g, b, a] = header.masks;
                    let alpha = if a == 0 { 255 } else { channel(value, a) };
                    [
                        channel(value, r),
                        channel(value, g),
                        channel(value, b),
                        alpha,
                    ]
                }
            };
            out[x * 4..x * 4 + 4].copy_from_slice(&pixel);
        }
        row(&out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a BMP with a 40-byte header. `extra` goes between the header
    /// and the pixels (palette or masks).
    fn bmp(
        width: i32,
        height: i32,
        bits: u16,
        compression: u32,
        extra: &[u8],
        pixels: &[u8],
    ) -> Vec<u8> {
        let offset = (14 + 40 + extra.len()) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(offset + pixels.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(&compression.to_le_bytes());
        out.extend_from_slice(&[0; 20]);
        out.extend_from_slice(extra);
        out.extend_from_slice(pixels);
        return out;
    }

    fn rows(data: &[u8]) -> Vec<Vec<u8>> {
        let header = read_header(data).expect("header");
        let mut out = Vec::new();
        decode_rows(data, &header, &mut |row| out.push(row.to_vec()));
        return out;
    }

    #[test]
    fn bottom_up_24_bit() {
        // 2x2; rows padded to 8 bytes. Stored bottom row first.
        let pixels = [
            0, 0, 255, 0, 255, 0, 0, 0, // bottom: red, green
            255, 0, 0, 255, 255, 255, 0, 0, // top: blue, white
        ];
        let out = rows(&bmp(2, 2, 24, BI_RGB, &[], &pixels));
        assert_eq!(out[0], [0, 0, 255, 255, 255, 255, 255, 255]);
        assert_eq!(out[1], [255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn top_down_32_bit_ignores_junk_alpha() {
        let pixels = [10, 20, 30, 0];
        let out = rows(&bmp(1, -1, 32, BI_RGB, &[], &pixels));
        assert_eq!(out[0], [30, 20, 10, 255]);
    }

    #[test]
    fn bitfields_with_alpha() {
        // 16-bit 5-6-5, then 32-bit with an explicit alpha mask.
        let mut extra = Vec::new();
        for mask in [0xF800u32, 0x07E0, 0x001F] {
            extra.extend_from_slice(&mask.to_le_bytes());
        }
        let out = rows(&bmp(1, 1, 16, BI_BITFIELDS, &extra, &[0x00, 0xF8, 0, 0]));
        assert_eq!(out[0], [255, 0, 0, 255]);

        let mut extra = Vec::new();
        for mask in [0x0000_FF00u32, 0x00FF_0000, 0xFF00_0000, 0x0000_00FF] {
            extra.extend_from_slice(&mask.to_le_bytes());
        }
        let out = rows(&bmp(1, 1, 32, BI_ALPHABITFIELDS, &extra, &[128, 1, 2, 3]));
        assert_eq!(out[0], [1, 2, 3, 128]);
    }

    #[test]
    fn paletted() {
        // Two colors: black and yellow. Pixels 0,1,1,0 in one byte, 1 bit each.
        let palette = [0, 0, 0, 0, 0, 255, 255, 0];
        let out = rows(&bmp(4, 1, 1, BI_RGB, &palette, &[0b0110_0000, 0, 0, 0]));
        assert_eq!(
            out[0],
            [
                0, 0, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 0, 0, 0, 255
            ]
        );
        // 8 bits, an index past the palette is black.
        let out = rows(&bmp(2, 1, 8, BI_RGB, &palette, &[1, 200, 0, 0]));
        assert_eq!(out[0], [255, 255, 0, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn rejects_bad_headers() {
        assert_eq!(read_header(b"BM"), None);
        assert_eq!(read_header(b"PNG"), None);
        // Pixel data missing.
        assert_eq!(read_header(&bmp(100, 100, 24, BI_RGB, &[], &[0; 16])), None);
        // Huge dimensions don't allocate or overflow.
        assert_eq!(
            read_header(&bmp(i32::MAX, i32::MAX, 32, BI_RGB, &[], &[])),
            None
        );
        assert_eq!(read_header(&bmp(1, i32::MIN, 24, BI_RGB, &[], &[])), None);
        assert_eq!(read_header(&bmp(0, 1, 24, BI_RGB, &[], &[0; 4])), None);
        // RLE isn't supported.
        assert_eq!(read_header(&bmp(1, 1, 8, 1, &[], &[0; 4])), None);
        // Truncated anywhere: never panics.
        let good = bmp(2, 2, 24, BI_RGB, &[], &[0; 16]);
        for len in 0..good.len() {
            assert_eq!(read_header(&good[..len]), None);
        }
    }
}
