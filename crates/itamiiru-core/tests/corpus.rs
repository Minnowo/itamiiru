//! The malformed-file corpus.
//!
//! `tests/corpus/seeds/` holds small valid images of every format, made
//! with ffmpeg. Each must decode. Then every seed is damaged in many ways
//! (cut short, bytes flipped, header dimensions made huge, the magic number
//! followed by junk) and each damaged copy must come back as a result,
//! never a crash, and the huge ones must be refused as too large.
//! Files placed directly in `tests/corpus/` are decoded as they are.

use std::path::PathBuf;

use itamiiru_core::imagefile::{self, ImageError};

fn corpus_dir() -> PathBuf {
    return PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
}

fn read_dir_files(dir: &PathBuf) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let items = std::fs::read_dir(dir).expect("corpus folder");
    for item in items {
        let path = item.expect("dir entry").path();
        if path.is_file() {
            let name = path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            out.push((name, std::fs::read(&path).expect("read")));
        }
    }
    out.sort();
    return out;
}

fn seeds() -> Vec<(String, Vec<u8>)> {
    let seeds = read_dir_files(&corpus_dir().join("seeds"));
    assert!(seeds.len() >= 10, "seed files missing");
    return seeds;
}

/// A small deterministic random number generator (xorshift), so failures
/// can be reproduced.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        return self.0;
    }
}

/// CRC-32 as PNG uses it, so patched PNG headers still pass the checksum.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    return !crc;
}

/// The seed with its dimensions changed to something enormous. None if the
/// format's header isn't where we expect.
fn header_bomb(name: &str, data: &[u8]) -> Option<Vec<u8>> {
    let mut out = data.to_vec();
    if name.ends_with(".png") {
        // IHDR: width and height at 16..24, CRC over type + data after it.
        out[16..20].copy_from_slice(&60_000u32.to_be_bytes());
        out[20..24].copy_from_slice(&60_000u32.to_be_bytes());
        let crc = crc32(&out[12..29]);
        out[29..33].copy_from_slice(&crc.to_be_bytes());
        return Some(out);
    }
    if name.ends_with(".jpg") {
        // SOF0: FF C0, length, precision, height, width.
        let sof = out.windows(2).position(|w| w == [0xFF, 0xC0])?;
        out[sof + 5..sof + 7].copy_from_slice(&65_000u16.to_be_bytes());
        out[sof + 7..sof + 9].copy_from_slice(&65_000u16.to_be_bytes());
        return Some(out);
    }
    if name.ends_with(".gif") {
        // The logical screen size, then the first frame's size.
        out[6..8].copy_from_slice(&65_000u16.to_le_bytes());
        out[8..10].copy_from_slice(&65_000u16.to_le_bytes());
        let frame = out.iter().position(|&b| b == 0x2C)?;
        out[frame + 5..frame + 7].copy_from_slice(&65_000u16.to_le_bytes());
        out[frame + 7..frame + 9].copy_from_slice(&65_000u16.to_le_bytes());
        return Some(out);
    }
    if name.ends_with(".bmp") {
        out[18..22].copy_from_slice(&60_000i32.to_le_bytes());
        out[22..26].copy_from_slice(&60_000i32.to_le_bytes());
        return Some(out);
    }
    if name.ends_with(".webp") {
        // VP8X holds the canvas size minus one, 24 bits each.
        let vp8x = out.windows(4).position(|w| w == b"VP8X")?;
        out[vp8x + 12..vp8x + 15].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
        out[vp8x + 15..vp8x + 18].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
        return Some(out);
    }
    return None;
}

#[test]
fn every_seed_decodes() {
    for (name, data) in seeds() {
        let image = imagefile::thumbnail(&data, 32).unwrap_or_else(|err| panic!("{name}: {err:?}"));
        assert_eq!((image.width, image.height), (32, 24), "{name}");
        // Something was drawn: not every pixel is transparent.
        let opaque = image.pixels.chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(opaque > 0, "{name}: empty image");
    }
}

#[test]
fn seeds_without_shrinking() {
    for (name, data) in seeds() {
        let image = imagefile::thumbnail(&data, 256).expect("decode");
        assert_eq!((image.width, image.height), (64, 48), "{name}");
    }
}

#[test]
fn truncated_files() {
    for (name, data) in seeds() {
        for len in (0..data.len()).step_by(7) {
            // Any result is fine; reaching the next line means no crash.
            let _ = imagefile::thumbnail(&data[..len], 32);
        }
        assert!(imagefile::thumbnail(&[], 32).is_err(), "{name}");
    }
}

#[test]
fn flipped_bytes() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for (_name, data) in seeds() {
        for _ in 0..300 {
            let mut damaged = data.clone();
            let flips = 1 + rng.next() % 8;
            for _ in 0..flips {
                // Leave the magic number alone so the decoder gets to run.
                let pos = 4 + (rng.next() as usize % (damaged.len() - 4));
                damaged[pos] = rng.next() as u8;
            }
            let _ = imagefile::thumbnail(&damaged, 32);
        }
    }
}

#[test]
fn magic_then_junk() {
    let mut rng = Rng(42);
    for (_name, data) in seeds() {
        for len in [16usize, 64, 1000, 20_000] {
            let mut damaged = data[..16.min(data.len())].to_vec();
            while damaged.len() < len {
                damaged.push(rng.next() as u8);
            }
            let _ = imagefile::thumbnail(&damaged, 32);
        }
    }
}

#[test]
fn huge_dimensions_are_refused() {
    let mut patched = 0;
    for (name, data) in seeds() {
        let Some(bomb) = header_bomb(&name, &data) else {
            continue;
        };
        patched += 1;
        let result = imagefile::thumbnail(&bomb, 128);
        // Too large, or rejected by the decoder's own checks; never decoded.
        match result {
            Err(ImageError::TooLarge) | Err(ImageError::Malformed(_)) => {}
            other => panic!("{name}: expected a refusal, got {other:?}"),
        }
    }
    assert!(patched >= 10, "only {patched} seeds could be patched");
}

#[test]
fn loose_corpus_files() {
    for (_name, data) in read_dir_files(&corpus_dir()) {
        let _ = imagefile::thumbnail(&data, 128);
    }
}
