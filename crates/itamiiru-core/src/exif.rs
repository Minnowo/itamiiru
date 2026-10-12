//! The two EXIF fields thumbnails need, read from a JPEG: the orientation
//! and the embedded thumbnail (PLAN.md section 7, steps 6 and 8).
//!
//! The file is untrusted, so every offset is checked against the data
//! before it's used, and only IFD0 and IFD1 are read (no loops to follow).

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExifInfo {
    /// 1 (normal) to 8. 1 when there's no EXIF data or the value is bad.
    pub orientation: u16,
    /// Where the embedded JPEG thumbnail sits inside the file, if any.
    pub thumbnail: Option<Range<usize>>,
}

const NO_EXIF: ExifInfo = ExifInfo {
    orientation: 1,
    thumbnail: None,
};

const TAG_ORIENTATION: u16 = 0x0112;
const TAG_THUMBNAIL_OFFSET: u16 = 0x0201;
const TAG_THUMBNAIL_LENGTH: u16 = 0x0202;

/// Reads the EXIF block of a JPEG file's bytes.
pub fn read_jpeg(data: &[u8]) -> ExifInfo {
    let Some(tiff) = find_exif(data) else {
        return NO_EXIF;
    };
    let mut info = parse_tiff(&data[tiff.clone()]);
    // Offsets inside the TIFF block become offsets inside the file.
    if let Some(thumb) = &info.thumbnail {
        info.thumbnail = Some(thumb.start + tiff.start..thumb.end + tiff.start);
    }
    return info;
}

/// The TIFF data inside the JPEG's `APP1 Exif` segment, as a range of `data`.
fn find_exif(data: &[u8]) -> Option<Range<usize>> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut pos = 2;
    while pos + 4 <= data.len() {
        if data[pos] != 0xFF {
            return None;
        }
        let marker = data[pos + 1];
        // Padding: a run of 0xFF before the marker.
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        // Start of scan or end of image: the metadata is over.
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        let length = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        // The length counts its own two bytes.
        if length < 2 {
            return None;
        }
        let body_start = pos + 4;
        let body_end = pos + 2 + length;
        if body_end > data.len() {
            return None;
        }
        let body = &data[body_start..body_end];
        if marker == 0xE1 && body.starts_with(b"Exif\0\0") {
            return Some(body_start + 6..body_end);
        }
        pos = body_end;
    }
    return None;
}

/// Reads TIFF-structured data in either byte order.
struct Tiff<'a> {
    data: &'a [u8],
    little_endian: bool,
}

impl Tiff<'_> {
    fn u16_at(&self, pos: usize) -> Option<u16> {
        let bytes = self.data.get(pos..pos.checked_add(2)?)?;
        let pair = [bytes[0], bytes[1]];
        if self.little_endian {
            return Some(u16::from_le_bytes(pair));
        }
        return Some(u16::from_be_bytes(pair));
    }

    fn u32_at(&self, pos: usize) -> Option<u32> {
        let bytes = self.data.get(pos..pos.checked_add(4)?)?;
        let quad = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if self.little_endian {
            return Some(u32::from_le_bytes(quad));
        }
        return Some(u32::from_be_bytes(quad));
    }
}

/// One IFD entry's value, read as a number. Only SHORT and LONG values
/// with a count of 1 are needed here.
fn entry_value(tiff: &Tiff, entry: usize) -> Option<u32> {
    const TYPE_SHORT: u16 = 3;
    const TYPE_LONG: u16 = 4;
    let kind = tiff.u16_at(entry + 2)?;
    let count = tiff.u32_at(entry + 4)?;
    if count != 1 {
        return None;
    }
    return match kind {
        TYPE_SHORT => Some(tiff.u16_at(entry + 8)? as u32),
        TYPE_LONG => tiff.u32_at(entry + 8),
        _ => None,
    };
}

fn parse_tiff(data: &[u8]) -> ExifInfo {
    let little_endian = match data.get(0..4) {
        Some(b"II*\0") => true,
        Some(b"MM\0*") => false,
        _ => return NO_EXIF,
    };
    let tiff = Tiff {
        data,
        little_endian,
    };
    let mut info = NO_EXIF;

    // IFD0 holds the orientation; the offset after its entries leads to
    // IFD1, which describes the thumbnail.
    let Some(ifd0) = tiff.u32_at(4) else {
        return info;
    };
    let mut next_ifd = None;
    if let Some((entries, next)) = read_ifd(&tiff, ifd0 as usize) {
        for entry in entries {
            if tiff.u16_at(entry) == Some(TAG_ORIENTATION)
                && let Some(value) = entry_value(&tiff, entry)
                && (1..=8).contains(&value)
            {
                info.orientation = value as u16;
            }
        }
        next_ifd = next;
    }

    let Some(ifd1) = next_ifd else {
        return info;
    };
    let Some((entries, _)) = read_ifd(&tiff, ifd1) else {
        return info;
    };
    let mut offset = None;
    let mut length = None;
    for entry in entries {
        match tiff.u16_at(entry) {
            Some(TAG_THUMBNAIL_OFFSET) => offset = entry_value(&tiff, entry),
            Some(TAG_THUMBNAIL_LENGTH) => length = entry_value(&tiff, entry),
            _ => {}
        }
    }
    if let (Some(offset), Some(length)) = (offset, length) {
        let start = offset as usize;
        let end = start.saturating_add(length as usize);
        // It must fit inside the block and look like a JPEG.
        if end <= data.len() && length >= 4 && data[start] == 0xFF && data[start + 1] == 0xD8 {
            info.thumbnail = Some(start..end);
        }
    }
    return info;
}

/// The entry positions of the IFD at `pos`, and where the next IFD starts
/// (None for 0, the end of the chain). None if the IFD doesn't fit.
fn read_ifd(tiff: &Tiff, pos: usize) -> Option<(Vec<usize>, Option<usize>)> {
    const ENTRY_SIZE: usize = 12;
    let count = tiff.u16_at(pos)? as usize;
    let first = pos + 2;
    let end = first + count * ENTRY_SIZE;
    if end + 4 > tiff.data.len() {
        return None;
    }
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        entries.push(first + i * ENTRY_SIZE);
    }
    let next = tiff.u32_at(end)? as usize;
    // Pointing back at itself (or nowhere) ends the chain.
    if next == 0 || next == pos {
        return Some((entries, None));
    }
    return Some((entries, Some(next)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny stand-in for an embedded JPEG thumbnail.
    const THUMB: &[u8] = b"\xFF\xD8thumbnail-bytes\xFF\xD9";

    /// Builds TIFF data with an IFD0 (orientation) and, if `thumb` is set,
    /// an IFD1 pointing at a thumbnail stored right after it.
    fn tiff(little_endian: bool, orientation: Option<u16>, thumb: bool) -> Vec<u8> {
        let u16b = |v: u16| {
            return if little_endian {
                v.to_le_bytes()
            } else {
                v.to_be_bytes()
            };
        };
        let u32b = |v: u32| {
            return if little_endian {
                v.to_le_bytes()
            } else {
                v.to_be_bytes()
            };
        };
        let mut out = Vec::new();
        out.extend_from_slice(if little_endian { b"II*\0" } else { b"MM\0*" });
        out.extend_from_slice(&u32b(8));

        // IFD0: one entry (or none), then the offset of IFD1.
        let ifd0_entries: u16 = if orientation.is_some() { 1 } else { 0 };
        out.extend_from_slice(&u16b(ifd0_entries));
        if let Some(value) = orientation {
            out.extend_from_slice(&u16b(TAG_ORIENTATION));
            out.extend_from_slice(&u16b(3));
            out.extend_from_slice(&u32b(1));
            out.extend_from_slice(&u16b(value));
            out.extend_from_slice(&[0, 0]);
        }
        let ifd1_pos = out.len() as u32 + 4;
        out.extend_from_slice(&u32b(if thumb { ifd1_pos } else { 0 }));

        if thumb {
            // IFD1: offset and length, then no next IFD, then the thumbnail.
            let thumb_pos = ifd1_pos + 2 + 2 * 12 + 4;
            out.extend_from_slice(&u16b(2));
            for (tag, value) in [
                (TAG_THUMBNAIL_OFFSET, thumb_pos),
                (TAG_THUMBNAIL_LENGTH, THUMB.len() as u32),
            ] {
                out.extend_from_slice(&u16b(tag));
                out.extend_from_slice(&u16b(4));
                out.extend_from_slice(&u32b(1));
                out.extend_from_slice(&u32b(value));
            }
            out.extend_from_slice(&u32b(0));
            out.extend_from_slice(THUMB);
        }
        return out;
    }

    /// A JPEG with an APP0 segment, then the EXIF segment, then a scan.
    fn jpeg_with(tiff: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        out.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x06, b'J', b'F', b'I', b'F']);
        let length = (2 + 6 + tiff.len()) as u16;
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(b"Exif\0\0");
        out.extend_from_slice(tiff);
        out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0x12, 0x34, 0xFF, 0xD9]);
        return out;
    }

    #[test]
    fn orientation_in_both_byte_orders() {
        for little_endian in [true, false] {
            let file = jpeg_with(&tiff(little_endian, Some(6), false));
            let info = read_jpeg(&file);
            assert_eq!(info.orientation, 6);
            assert_eq!(info.thumbnail, None);
        }
    }

    #[test]
    fn embedded_thumbnail() {
        for little_endian in [true, false] {
            let file = jpeg_with(&tiff(little_endian, Some(3), true));
            let info = read_jpeg(&file);
            assert_eq!(info.orientation, 3);
            let range = info.thumbnail.expect("thumbnail");
            assert_eq!(&file[range], THUMB);
        }
    }

    #[test]
    fn no_exif() {
        let plain = [0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02, 0xFF, 0xD9];
        assert_eq!(read_jpeg(&plain), NO_EXIF);
        assert_eq!(read_jpeg(b""), NO_EXIF);
        assert_eq!(read_jpeg(b"\x89PNG\r\n\x1a\n"), NO_EXIF);
        // EXIF without an orientation tag.
        assert_eq!(read_jpeg(&jpeg_with(&tiff(true, None, false))), NO_EXIF);
    }

    #[test]
    fn bad_values_are_ignored() {
        let info = read_jpeg(&jpeg_with(&tiff(true, Some(9), false)));
        assert_eq!(info.orientation, 1);
        let info = read_jpeg(&jpeg_with(&tiff(true, Some(0), false)));
        assert_eq!(info.orientation, 1);
    }

    #[test]
    fn truncated_files_never_panic() {
        let file = jpeg_with(&tiff(false, Some(8), true));
        for len in 0..file.len() {
            let info = read_jpeg(&file[..len]);
            // Cut anywhere, it either finds nothing or stays in bounds.
            if let Some(range) = info.thumbnail {
                assert!(range.end <= len);
            }
        }
    }

    #[test]
    fn hostile_offsets() {
        // IFD0 offset far past the end, and an IFD that points at itself.
        let mut data = tiff(true, Some(6), false);
        data[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(read_jpeg(&jpeg_with(&data)), NO_EXIF);

        let mut data = tiff(true, None, false);
        // IFD0 has no entries, so its next-IFD offset sits at 10.
        data[10..14].copy_from_slice(&8u32.to_le_bytes());
        assert_eq!(read_jpeg(&jpeg_with(&data)), NO_EXIF);

        // A thumbnail whose length runs past the end.
        let mut data = tiff(true, None, true);
        let length_value = 8 + 2 + 4 + 2 + 12 + 8;
        data[length_value..length_value + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(read_jpeg(&jpeg_with(&data)).thumbnail, None);
    }
}
