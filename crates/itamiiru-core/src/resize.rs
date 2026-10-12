//! Box-filter downscaling.
//!
//! Every source pixel lands in exactly one destination pixel, which is the
//! average of the pixels that landed in it. Rows are fed in one at a time,
//! so a big PNG never has to be in memory all at once.

use crate::rgba::RgbaImage;

/// The size that fits inside `max` x `max` with the same shape. Never
/// bigger than the source, and never 0 on a side.
pub fn fit(width: u32, height: u32, max: u32) -> (u32, u32) {
    if width <= max && height <= max {
        return (width.max(1), height.max(1));
    }
    let (w, h) = (width as u64, height as u64);
    let max64 = max as u64;
    if w >= h {
        let new_h = (h * max64 / w).max(1);
        return (max, new_h as u32);
    }
    let new_w = (w * max64 / h).max(1);
    return (new_w as u32, max);
}

/// Shrinks an image fed to it row by row.
pub struct RowScaler {
    src_width: u32,
    src_height: u32,
    out: RgbaImage,
    /// The destination column of each source column.
    column_of: Vec<u32>,
    /// Sums for the destination row being filled: red, green and blue
    /// weighted by alpha, then alpha, then the pixel count.
    sums: Vec<u64>,
    /// The destination row `sums` belongs to.
    sum_row: u32,
    /// Source rows seen so far.
    rows_in: u32,
}

const SUM_FIELDS: usize = 5;

impl RowScaler {
    /// `out_width` and `out_height` must not be bigger than the source;
    /// use `fit` to choose them.
    pub fn new(src_width: u32, src_height: u32, out_width: u32, out_height: u32) -> RowScaler {
        let out_width = out_width.clamp(1, src_width.max(1));
        let out_height = out_height.clamp(1, src_height.max(1));
        let mut column_of = Vec::with_capacity(src_width as usize);
        for x in 0..src_width as u64 {
            column_of.push((x * out_width as u64 / src_width as u64) as u32);
        }
        return RowScaler {
            src_width,
            src_height,
            out: RgbaImage::new(out_width, out_height),
            column_of,
            sums: vec![0; out_width as usize * SUM_FIELDS],
            sum_row: 0,
            rows_in: 0,
        };
    }

    /// Adds the next source row: `src_width` RGBA pixels. Extra rows, and
    /// rows of the wrong length, are ignored.
    pub fn push_row(&mut self, row: &[u8]) {
        if self.rows_in >= self.src_height || row.len() != self.src_width as usize * 4 {
            return;
        }
        let dest_row =
            (self.rows_in as u64 * self.out.height as u64 / self.src_height as u64) as u32;
        if dest_row != self.sum_row {
            self.flush();
            self.sum_row = dest_row;
        }
        for (x, pixel) in row.chunks_exact(4).enumerate() {
            let s = self.column_of[x] as usize * SUM_FIELDS;
            let alpha = pixel[3] as u64;
            self.sums[s] += pixel[0] as u64 * alpha;
            self.sums[s + 1] += pixel[1] as u64 * alpha;
            self.sums[s + 2] += pixel[2] as u64 * alpha;
            self.sums[s + 3] += alpha;
            self.sums[s + 4] += 1;
        }
        self.rows_in += 1;
    }

    /// Writes the sums into the destination row and clears them.
    fn flush(&mut self) {
        let width = self.out.width as usize;
        let start = self.sum_row as usize * width * 4;
        for x in 0..width {
            let s = x * SUM_FIELDS;
            let alpha_sum = self.sums[s + 3];
            let count = self.sums[s + 4];
            if count > 0 && alpha_sum > 0 {
                // Weighting by alpha keeps transparent pixels' colors from
                // bleeding into the edges.
                let p = start + x * 4;
                self.out.pixels[p] = (self.sums[s] / alpha_sum) as u8;
                self.out.pixels[p + 1] = (self.sums[s + 1] / alpha_sum) as u8;
                self.out.pixels[p + 2] = (self.sums[s + 2] / alpha_sum) as u8;
                self.out.pixels[p + 3] = (alpha_sum / count) as u8;
            }
        }
        for value in self.sums.iter_mut() {
            *value = 0;
        }
    }

    /// The finished image. Rows never fed in (a truncated file) stay transparent.
    pub fn finish(mut self) -> RgbaImage {
        if self.rows_in > 0 {
            self.flush();
        }
        return self.out;
    }
}

/// Shrinks a whole image to fit inside `max` x `max`.
pub fn downscale(image: &RgbaImage, max: u32) -> RgbaImage {
    let (w, h) = fit(image.width, image.height, max);
    if w == image.width && h == image.height {
        return image.clone();
    }
    let mut scaler = RowScaler::new(image.width, image.height, w, h);
    let row_len = image.width as usize * 4;
    for row in image.pixels.chunks_exact(row_len) {
        scaler.push_row(row);
    }
    return scaler.finish();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> RgbaImage {
        let mut image = RgbaImage::new(width, height);
        for pixel in image.pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&rgba);
        }
        return image;
    }

    #[test]
    fn fit_keeps_the_shape() {
        assert_eq!(fit(4000, 3000, 256), (256, 192));
        assert_eq!(fit(3000, 4000, 256), (192, 256));
        assert_eq!(fit(100, 50, 256), (100, 50));
        assert_eq!(fit(256, 256, 256), (256, 256));
        // A very thin image still gets one pixel.
        assert_eq!(fit(100_000, 1, 256), (256, 1));
        assert_eq!(fit(0, 0, 256), (1, 1));
    }

    #[test]
    fn solid_color_stays_the_same() {
        let out = downscale(&solid(37, 23, [10, 20, 30, 255]), 8);
        assert_eq!((out.width, out.height), (8, 4));
        for pixel in out.pixels.chunks_exact(4) {
            assert_eq!(pixel, [10, 20, 30, 255]);
        }
    }

    #[test]
    fn averages_blocks() {
        // 4x2 -> 2x1: left block black and white, right block all gray.
        let mut image = solid(4, 2, [100, 100, 100, 255]);
        let black_white = [(0, 0, 0), (1, 0, 255), (0, 1, 255), (1, 1, 0)];
        for (x, y, value) in black_white {
            let i = (y * 4 + x) * 4;
            image.pixels[i] = value;
            image.pixels[i + 1] = value;
            image.pixels[i + 2] = value;
        }
        let out = downscale(&image, 2);
        assert_eq!((out.width, out.height), (2, 1));
        assert_eq!(&out.pixels[0..4], [127, 127, 127, 255]);
        assert_eq!(&out.pixels[4..8], [100, 100, 100, 255]);
    }

    #[test]
    fn transparent_pixels_do_not_darken_edges() {
        // Red and fully transparent black average to half-transparent red,
        // not half-transparent dark red.
        let mut image = solid(2, 1, [255, 0, 0, 255]);
        image.pixels[4..8].copy_from_slice(&[0, 0, 0, 0]);
        let out = downscale(&image, 1);
        assert_eq!(out.pixels, [255, 0, 0, 127]);
    }

    #[test]
    fn missing_rows_stay_transparent() {
        let mut scaler = RowScaler::new(4, 4, 2, 2);
        scaler.push_row(&[255; 16]);
        scaler.push_row(&[255; 16]);
        // A wrong-length row is ignored.
        scaler.push_row(&[255; 3]);
        let out = scaler.finish();
        assert_eq!(&out.pixels[0..8], [255; 8]);
        assert_eq!(&out.pixels[8..16], [0; 8]);
    }

    #[test]
    fn extra_rows_are_ignored() {
        let mut scaler = RowScaler::new(1, 1, 1, 1);
        scaler.push_row(&[1, 2, 3, 255]);
        scaler.push_row(&[9, 9, 9, 255]);
        assert_eq!(scaler.finish().pixels, [1, 2, 3, 255]);
    }
}
