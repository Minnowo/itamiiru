//! An 8-bit RGBA image in memory (not premultiplied), plus the eight EXIF
//! orientations.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    /// Row by row, 4 bytes per pixel: red, green, blue, alpha.
    pub pixels: Vec<u8>,
}

impl RgbaImage {
    /// A fully transparent image.
    pub fn new(width: u32, height: u32) -> RgbaImage {
        let len = width as usize * height as usize * 4;
        return RgbaImage {
            width,
            height,
            pixels: vec![0; len],
        };
    }

    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        return [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ];
    }

    /// The image turned the right way up for an EXIF `Orientation` value
    /// (1 to 8). Unknown values leave it as it is.
    pub fn oriented(self, orientation: u16) -> RgbaImage {
        if !(2..=8).contains(&orientation) {
            return self;
        }
        let (w, h) = (self.width, self.height);
        // 5 to 8 swap width and height.
        let swaps = orientation >= 5;
        let (out_w, out_h) = if swaps { (h, w) } else { (w, h) };
        let mut out = RgbaImage::new(out_w, out_h);
        let mut i = 0;
        for y in 0..out_h {
            for x in 0..out_w {
                // Where this output pixel comes from in the stored image.
                let (sx, sy) = match orientation {
                    2 => (w - 1 - x, y),
                    3 => (w - 1 - x, h - 1 - y),
                    4 => (x, h - 1 - y),
                    5 => (y, x),
                    6 => (y, h - 1 - x),
                    7 => (w - 1 - y, h - 1 - x),
                    _ => (w - 1 - y, x),
                };
                out.pixels[i..i + 4].copy_from_slice(&self.pixel(sx, sy));
                i += 4;
            }
        }
        return out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3x2 image whose pixels are numbered 1 to 6 (in the red channel):
    /// ```text
    /// 1 2 3
    /// 4 5 6
    /// ```
    fn numbered() -> RgbaImage {
        let mut image = RgbaImage::new(3, 2);
        for n in 0..6 {
            image.pixels[n * 4] = n as u8 + 1;
        }
        return image;
    }

    fn reds(image: &RgbaImage) -> Vec<u8> {
        let mut out = Vec::new();
        for pixel in image.pixels.chunks_exact(4) {
            out.push(pixel[0]);
        }
        return out;
    }

    #[test]
    fn all_eight_orientations() {
        let cases: [(u16, u32, u32, [u8; 6]); 8] = [
            (1, 3, 2, [1, 2, 3, 4, 5, 6]),
            (2, 3, 2, [3, 2, 1, 6, 5, 4]),
            (3, 3, 2, [6, 5, 4, 3, 2, 1]),
            (4, 3, 2, [4, 5, 6, 1, 2, 3]),
            // Transpose.
            (5, 2, 3, [1, 4, 2, 5, 3, 6]),
            // Turned 90 degrees clockwise.
            (6, 2, 3, [4, 1, 5, 2, 6, 3]),
            (7, 2, 3, [6, 3, 5, 2, 4, 1]),
            // Turned 90 degrees counter-clockwise.
            (8, 2, 3, [3, 6, 2, 5, 1, 4]),
        ];
        for (orientation, w, h, expected) in cases {
            let out = numbered().oriented(orientation);
            assert_eq!((out.width, out.height), (w, h), "orientation {orientation}");
            assert_eq!(reds(&out), expected, "orientation {orientation}");
        }
    }

    #[test]
    fn unknown_orientation_is_ignored() {
        assert_eq!(numbered().oriented(0), numbered());
        assert_eq!(numbered().oriented(9), numbered());
    }
}
