//! Turns image file bytes into a small RGBA thumbnail (PLAN.md section 7,
//! steps 3 to 9): sniff the format, check the size limits, decode, turn it
//! the right way up, shrink.
//!
//! Image files are untrusted. The dimensions are checked against the
//! limits before any pixel buffer is allocated, each decoder gets a memory
//! cap, and the whole decode runs inside `catch_unwind`, so a decoder bug
//! on a bad file costs one thumbnail, not the thread.

use std::io::Cursor;

use crate::exif;
use crate::resize::{RowScaler, fit};
use crate::rgba::RgbaImage;

/// Files bigger than this aren't thumbnailed.
pub const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024;
/// Images wider or taller than this aren't thumbnailed.
pub const MAX_SIDE: u32 = 16_384;
/// Nor are images with more pixels than this.
pub const MAX_PIXELS: u64 = 100_000_000;
/// The most a decoder may allocate: one RGBA buffer at the pixel limit.
const MAX_DECODE_BYTES: u64 = MAX_PIXELS * 4;
/// How many bytes `sniff` needs.
pub const SNIFF_LEN: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Not a format we decode.
    UnknownFormat,
    /// Over one of the limits above.
    TooLarge,
    /// The decoder rejected the file. The text is for debug logs.
    Malformed(String),
    /// The decoder panicked.
    Panicked,
}

fn malformed(err: impl std::fmt::Display) -> ImageError {
    return ImageError::Malformed(err.to_string());
}

/// The format, from the magic numbers at the start of the file.
pub fn sniff(head: &[u8]) -> Option<Format> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Format::Png);
    }
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Format::Jpeg);
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some(Format::Gif);
    }
    if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return Some(Format::Webp);
    }
    // "BM" alone is too common a start; also want the 40+ byte header size.
    if head.len() >= 18 && head.starts_with(b"BM") {
        let header_size = u32::from_le_bytes([head[14], head[15], head[16], head[17]]);
        if (40..=124).contains(&header_size) {
            return Some(Format::Bmp);
        }
    }
    return None;
}

/// Checks the dimensions a header claims, before anything is allocated.
pub fn check_size(width: u32, height: u32) -> Result<(), ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::Malformed("zero size".to_string()));
    }
    if width > MAX_SIDE || height > MAX_SIDE || width as u64 * height as u64 > MAX_PIXELS {
        return Err(ImageError::TooLarge);
    }
    return Ok(());
}

/// Makes a thumbnail that fits inside `max` x `max` from a whole file's
/// bytes. Never panics.
pub fn thumbnail(data: &[u8], max: u32) -> Result<RgbaImage, ImageError> {
    if data.len() as u64 > MAX_FILE_SIZE {
        return Err(ImageError::TooLarge);
    }
    let Some(format) = sniff(data) else {
        return Err(ImageError::UnknownFormat);
    };
    // catch_unwind turns a panic inside the closure into an Err. The
    // closure only borrows plain bytes, so nothing is left half-changed.
    let result = std::panic::catch_unwind(|| decode(data, format, max));
    return match result {
        Ok(result) => result,
        Err(_) => Err(ImageError::Panicked),
    };
}

fn decode(data: &[u8], format: Format, max: u32) -> Result<RgbaImage, ImageError> {
    return match format {
        Format::Png => decode_png(data, max).map(|(image, _)| image),
        Format::Jpeg => decode_jpeg_file(data, max),
        Format::Gif => decode_gif(data, max),
        Format::Webp => decode_webp(data, max),
        Format::Bmp => decode_bmp(data, max),
    };
}

/// How the pixels in a decoder's buffer are laid out.
#[derive(Clone, Copy)]
enum Layout {
    Gray,
    GrayAlpha,
    Rgb,
    Rgba,
    Cmyk,
    /// 16-bit gray in native byte order (jpeg-decoder's L16).
    Gray16,
}

impl Layout {
    fn bytes_per_pixel(self) -> usize {
        return match self {
            Layout::Gray => 1,
            Layout::GrayAlpha | Layout::Gray16 => 2,
            Layout::Rgb => 3,
            Layout::Rgba | Layout::Cmyk => 4,
        };
    }
}

/// Converts one row to RGBA into `out` (resized to fit).
fn row_to_rgba(row: &[u8], layout: Layout, out: &mut Vec<u8>) {
    out.clear();
    let bpp = layout.bytes_per_pixel();
    for p in row.chunks_exact(bpp) {
        let pixel = match layout {
            Layout::Gray => [p[0], p[0], p[0], 255],
            Layout::GrayAlpha => [p[0], p[0], p[0], p[1]],
            Layout::Rgb => [p[0], p[1], p[2], 255],
            Layout::Rgba => [p[0], p[1], p[2], p[3]],
            Layout::Cmyk => {
                let k = 255 - p[3] as u32;
                let r = (255 - p[0] as u32) * k / 255;
                let g = (255 - p[1] as u32) * k / 255;
                let b = (255 - p[2] as u32) * k / 255;
                [r as u8, g as u8, b as u8, 255]
            }
            Layout::Gray16 => {
                let v = (u16::from_ne_bytes([p[0], p[1]]) >> 8) as u8;
                [v, v, v, 255]
            }
        };
        out.extend_from_slice(&pixel);
    }
}

/// Shrinks a whole decoded buffer of `width` x `height` pixels.
fn scale_buffer(
    pixels: &[u8],
    width: u32,
    height: u32,
    layout: Layout,
    max: u32,
) -> Result<RgbaImage, ImageError> {
    let row_len = width as usize * layout.bytes_per_pixel();
    if pixels.len() < row_len * height as usize {
        return Err(malformed("decoder returned too few pixels"));
    }
    let (out_w, out_h) = fit(width, height, max);
    let mut scaler = RowScaler::new(width, height, out_w, out_h);
    let mut rgba = Vec::with_capacity(width as usize * 4);
    for row in pixels.chunks_exact(row_len).take(height as usize) {
        row_to_rgba(row, layout, &mut rgba);
        scaler.push_row(&rgba);
    }
    return Ok(scaler.finish());
}

/// Decodes a PNG and returns its tEXt chunks too (the thumbnail cache
/// stores the source URI and mtime in them). Rows are shrunk as they
/// arrive, so even a big PNG only needs one row in memory.
pub fn decode_png(data: &[u8], max: u32) -> Result<(RgbaImage, Vec<(String, String)>), ImageError> {
    let limits = png::Limits {
        bytes: MAX_DECODE_BYTES as usize,
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(data), limits);
    // Expand palettes, low bit depths and tRNS transparency; drop 16-bit
    // samples to 8. The output is then gray, gray+alpha, RGB or RGBA.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(malformed)?;
    let (width, height, interlaced) = {
        let info = reader.info();
        (info.width, info.height, info.interlaced)
    };
    check_size(width, height)?;
    let layout = match reader.output_color_type().0 {
        png::ColorType::Grayscale => Layout::Gray,
        png::ColorType::GrayscaleAlpha => Layout::GrayAlpha,
        png::ColorType::Rgb => Layout::Rgb,
        png::ColorType::Rgba => Layout::Rgba,
        png::ColorType::Indexed => return Err(malformed("palette not expanded")),
    };

    let image = if interlaced {
        // Interlaced rows arrive in seven passes, so decode it all first.
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| malformed("size overflow"))?;
        let mut pixels = vec![0; size];
        reader.next_frame(&mut pixels).map_err(malformed)?;
        scale_buffer(&pixels, width, height, layout, max)?
    } else {
        let (out_w, out_h) = fit(width, height, max);
        let mut scaler = RowScaler::new(width, height, out_w, out_h);
        let mut rgba = Vec::with_capacity(width as usize * 4);
        while let Some(row) = reader.next_row().map_err(malformed)? {
            row_to_rgba(row.data(), layout, &mut rgba);
            scaler.push_row(&rgba);
        }
        scaler.finish()
    };

    let mut text = Vec::new();
    for chunk in &reader.info().uncompressed_latin1_text {
        text.push((chunk.keyword.clone(), chunk.text.clone()));
    }
    return Ok((image, text));
}

/// A JPEG file: tries the EXIF thumbnail first, if it's big enough, then
/// decodes the image itself at 1/2, 1/4 or 1/8 size where that's enough.
fn decode_jpeg_file(data: &[u8], max: u32) -> Result<RgbaImage, ImageError> {
    let exif = exif::read_jpeg(data);
    if let Some(range) = exif.thumbnail
        && let Ok(Some(image)) = decode_jpeg(&data[range], max, max)
    {
        return Ok(image.oriented(exif.orientation));
    }
    let image = decode_jpeg(data, max, 0)?;
    let Some(image) = image else {
        return Err(malformed("no image"));
    };
    return Ok(image.oriented(exif.orientation));
}

/// Decodes a JPEG, or returns None if its longer side is under `min_side`
/// (an embedded thumbnail too small to use).
fn decode_jpeg(data: &[u8], max: u32, min_side: u32) -> Result<Option<RgbaImage>, ImageError> {
    let mut decoder = jpeg_decoder::Decoder::new(data);
    decoder.set_max_decoding_buffer_size(MAX_DECODE_BYTES as usize);
    decoder.read_info().map_err(malformed)?;
    let Some(info) = decoder.info() else {
        return Err(malformed("no header"));
    };
    let (width, height) = (info.width as u32, info.height as u32);
    check_size(width, height)?;
    if width.max(height) < min_side {
        return Ok(None);
    }
    // Ask for the final size; the decoder picks the smallest scale that
    // is at least that big.
    let (want_w, want_h) = fit(width, height, max);
    let (out_w, out_h) = decoder
        .scale(want_w as u16, want_h as u16)
        .map_err(malformed)?;
    let pixels = decoder.decode().map_err(malformed)?;
    let Some(info) = decoder.info() else {
        return Err(malformed("no header"));
    };
    let layout = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => Layout::Gray,
        jpeg_decoder::PixelFormat::L16 => Layout::Gray16,
        jpeg_decoder::PixelFormat::RGB24 => Layout::Rgb,
        jpeg_decoder::PixelFormat::CMYK32 => Layout::Cmyk,
    };
    let image = scale_buffer(&pixels, out_w as u32, out_h as u32, layout, max)?;
    return Ok(Some(image));
}

/// The first frame of a GIF.
fn decode_gif(data: &[u8], max: u32) -> Result<RgbaImage, ImageError> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let limit = std::num::NonZeroU64::new(MAX_DECODE_BYTES).expect("the limit is not zero");
    options.set_memory_limit(gif::MemoryLimit::Bytes(limit));
    // Frames must fit inside the screen size we check below.
    options.check_frame_consistency(true);
    let mut decoder = options.read_info(data).map_err(malformed)?;
    check_size(decoder.width() as u32, decoder.height() as u32)?;
    let Some(frame) = decoder.read_next_frame().map_err(malformed)? else {
        return Err(malformed("no frames"));
    };
    let (width, height) = (frame.width as u32, frame.height as u32);
    check_size(width, height)?;
    return scale_buffer(&frame.buffer, width, height, Layout::Rgba, max);
}

/// A WebP (the first frame, if animated).
fn decode_webp(data: &[u8], max: u32) -> Result<RgbaImage, ImageError> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(data)).map_err(malformed)?;
    decoder.set_memory_limit(MAX_DECODE_BYTES as usize);
    let (width, height) = decoder.dimensions();
    check_size(width, height)?;
    let layout = if decoder.has_alpha() {
        Layout::Rgba
    } else {
        Layout::Rgb
    };
    let Some(size) = decoder.output_buffer_size() else {
        return Err(ImageError::TooLarge);
    };
    let mut pixels = vec![0; size];
    decoder.read_image(&mut pixels).map_err(malformed)?;
    return scale_buffer(&pixels, width, height, layout, max);
}

fn decode_bmp(data: &[u8], max: u32) -> Result<RgbaImage, ImageError> {
    let Some(header) = crate::bmp::read_header(data) else {
        return Err(malformed("unsupported or damaged BMP"));
    };
    check_size(header.width, header.height)?;
    let (out_w, out_h) = fit(header.width, header.height, max);
    let mut scaler = RowScaler::new(header.width, header.height, out_w, out_h);
    crate::bmp::decode_rows(data, &header, &mut |row| scaler.push_row(row));
    return Ok(scaler.finish());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG made with the png crate's encoder.
    fn png_bytes(width: u32, height: u32, color: png::ColorType, pixels: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_text_chunk("Thumb::URI".to_string(), "file:///a.png".to_string())
            .expect("text");
        let mut writer = encoder.write_header().expect("header");
        writer.write_image_data(pixels).expect("data");
        writer.finish().expect("finish");
        return out;
    }

    #[test]
    fn sniffs_formats() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some(Format::Png));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Format::Jpeg));
        assert_eq!(sniff(b"GIF89a...."), Some(Format::Gif));
        assert_eq!(sniff(b"GIF87a...."), Some(Format::Gif));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(Format::Webp));
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVEfmt "), None);
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&[0; 12]);
        bmp.extend_from_slice(&40u32.to_le_bytes());
        assert_eq!(sniff(&bmp), Some(Format::Bmp));
        assert_eq!(sniff(b"BMW is not a bitmap"), None);
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"hello world"), None);
    }

    #[test]
    fn size_limits() {
        assert_eq!(check_size(4000, 3000), Ok(()));
        assert_eq!(check_size(MAX_SIDE, 6000), Ok(()));
        assert_eq!(check_size(MAX_SIDE + 1, 1), Err(ImageError::TooLarge));
        assert_eq!(check_size(1, MAX_SIDE + 1), Err(ImageError::TooLarge));
        // Under the side limit, over the pixel limit.
        assert_eq!(check_size(12_000, 12_000), Err(ImageError::TooLarge));
        assert!(check_size(0, 10).is_err());
    }

    #[test]
    fn png_round_trip() {
        let mut pixels = Vec::new();
        for _ in 0..(64 * 32) {
            pixels.extend_from_slice(&[200, 100, 50, 255]);
        }
        let file = png_bytes(64, 32, png::ColorType::Rgba, &pixels);
        let image = thumbnail(&file, 16).expect("thumbnail");
        assert_eq!((image.width, image.height), (16, 8));
        assert_eq!(&image.pixels[0..4], [200, 100, 50, 255]);

        let (_, text) = decode_png(&file, 16).expect("decode");
        assert_eq!(
            text,
            vec![("Thumb::URI".to_string(), "file:///a.png".to_string())]
        );
    }

    #[test]
    fn png_gray_is_expanded() {
        let file = png_bytes(2, 1, png::ColorType::Grayscale, &[0, 255]);
        let image = thumbnail(&file, 16).expect("thumbnail");
        assert_eq!(image.pixels, [0, 0, 0, 255, 255, 255, 255, 255]);
    }

    #[test]
    fn png_header_bomb_is_refused_before_decoding() {
        // A valid 1x1 PNG whose IHDR claims 100,000 x 100,000. The CRC is
        // wrong too, so either check stops it; neither may allocate 40 GB.
        let mut file = png_bytes(1, 1, png::ColorType::Rgba, &[0; 4]);
        file[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        file[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(thumbnail(&file, 128).is_err());
    }

    #[test]
    fn bmp_thumbnail() {
        // 2x1, 24-bit, red then blue, rows padded to 8 bytes.
        let mut file = b"BM".to_vec();
        file.extend_from_slice(&62u32.to_le_bytes());
        file.extend_from_slice(&[0; 4]);
        file.extend_from_slice(&54u32.to_le_bytes());
        file.extend_from_slice(&40u32.to_le_bytes());
        file.extend_from_slice(&2i32.to_le_bytes());
        file.extend_from_slice(&1i32.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&24u16.to_le_bytes());
        file.extend_from_slice(&[0; 24]);
        file.extend_from_slice(&[0, 0, 255, 255, 0, 0, 0, 0]);
        let image = thumbnail(&file, 16).expect("thumbnail");
        assert_eq!(image.pixels, [255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn rejects_unknown_and_oversized() {
        assert_eq!(
            thumbnail(b"plain text", 128),
            Err(ImageError::UnknownFormat)
        );
        let big = vec![0u8; MAX_FILE_SIZE as usize + 1];
        assert_eq!(thumbnail(&big, 128), Err(ImageError::TooLarge));
    }

    #[test]
    fn cmyk_and_gray16_conversion() {
        let mut out = Vec::new();
        row_to_rgba(&[0, 0, 0, 0, 255, 255, 255, 0], Layout::Cmyk, &mut out);
        assert_eq!(out, [255, 255, 255, 255, 0, 0, 0, 255]);
        row_to_rgba(&0xABCDu16.to_ne_bytes(), Layout::Gray16, &mut out);
        assert_eq!(out, [0xAB, 0xAB, 0xAB, 255]);
    }
}
