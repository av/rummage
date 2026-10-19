//! Thumbnails and previews, as lossy WebP.

use image::{DynamicImage, RgbImage, imageops::FilterType};

/// WebP quality for thumbnails and previews.
const QUALITY: f32 = 78.0;

/// Encodes an image as lossy WebP.
pub fn webp(img: &RgbImage) -> Vec<u8> {
    webp::Encoder::from_rgb(img.as_raw(), img.width(), img.height())
        .encode(QUALITY)
        .to_vec()
}

/// Scales an image down so its longest edge is at most `size` (never up).
pub fn fit(img: &DynamicImage, size: u32) -> RgbImage {
    let (w, h) = (img.width(), img.height());
    if w.max(h) <= size {
        return img.to_rgb8();
    }
    img.resize(size, size, FilterType::Triangle).to_rgb8()
}

/// A WebP thumbnail whose longest edge is at most `size`.
pub fn thumbnail(img: &DynamicImage, size: u32) -> Vec<u8> {
    webp(&fit(img, size))
}

/// A waveform picture of mono samples: `width` x `width / 4`, one min/max bar per column.
pub fn waveform(samples: &[f32], width: u32) -> Vec<u8> {
    let width = width.max(16);
    let height = (width / 4).max(8);
    let mut img = RgbImage::from_pixel(width, height, image::Rgb([24, 26, 32]));
    if !samples.is_empty() {
        let per = samples.len().div_ceil(width as usize).max(1);
        let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs())).max(1e-3);
        let mid = height as f32 / 2.0;
        for (x, col) in samples.chunks(per).enumerate().take(width as usize) {
            let (lo, hi) = col
                .iter()
                .fold((0f32, 0f32), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
            let top = (mid - hi / peak * (mid - 1.0)).floor().max(0.0) as u32;
            let bottom = (mid - lo / peak * (mid - 1.0))
                .ceil()
                .min(height as f32 - 1.0) as u32;
            for y in top..=bottom.max(top) {
                img.put_pixel(x as u32, y, image::Rgb([96, 165, 250]));
            }
        }
    }
    webp(&img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_are_webp_and_fit() {
        let img =
            DynamicImage::ImageRgb8(RgbImage::from_pixel(800, 400, image::Rgb([200, 10, 10])));
        let t = thumbnail(&img, 100);
        assert_eq!(&t[..4], b"RIFF");
        assert_eq!(&t[8..12], b"WEBP");
        let back = image::load_from_memory(&t).unwrap();
        assert_eq!((back.width(), back.height()), (100, 50));
        let small = DynamicImage::ImageRgb8(RgbImage::new(10, 10));
        assert_eq!(fit(&small, 100).width(), 10);
        let w = waveform(&[0.0, 0.5, -0.5, 1.0], 64);
        let back = image::load_from_memory(&w).unwrap();
        assert_eq!((back.width(), back.height()), (64, 16));
    }
}
