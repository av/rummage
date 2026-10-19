//! Input preparation that has to match the official EmbeddingGemma 2 processor.

use anyhow::{Context, Result, bail};
use image::{RgbImage, imageops::FilterType};
use rummage_core::embed::ImageData;

/// Vision patch size and pooling kernel of the EmbeddingGemma 2 image encoder.
const PATCH: u32 = 16;
const POOL: u32 = 3;

/// Audio sample rate the model takes.
pub const AUDIO_RATE: usize = 16_000;
/// The official processor caps one audio part at 30 s (750 soft tokens); longer input is
/// split into 30 s parts, each wrapped in its own `<|audio>` ... `<audio|>` block.
pub const AUDIO_PART_SAMPLES: usize = 30 * AUDIO_RATE;

/// Decodes an image (if needed) and resizes it the way the official processor does: keep the
/// aspect ratio, scale (up or down) to the largest size whose patch count fits `max_tokens`
/// pooled tokens, with both sides a multiple of 48 px, using bicubic filtering.
///
/// llama.cpp's own preprocessing only rescales images that fall outside its token limits, so
/// small images would otherwise get far fewer tokens than the reference model gives them.
pub fn prepare_image(img: &ImageData, max_tokens: u32) -> Result<RgbImage> {
    let rgb = match img {
        ImageData::Encoded(bytes) => image::load_from_memory(bytes)
            .context("decoding image")?
            .to_rgb8(),
        ImageData::Rgb8 {
            width,
            height,
            pixels,
        } => RgbImage::from_raw(*width, *height, pixels.clone())
            .context("Rgb8 image: pixel buffer is not width * height * 3 bytes")?,
    };
    if rgb.width() == 0 || rgb.height() == 0 {
        bail!("empty image");
    }
    let (h, w) = target_size(rgb.height(), rgb.width(), max_tokens * POOL * POOL)?;
    if (h, w) == (rgb.height(), rgb.width()) {
        return Ok(rgb);
    }
    // CatmullRom is the a = -0.5 cubic that PIL and torchvision's antialiased bicubic use.
    Ok(image::imageops::resize(&rgb, w, h, FilterType::CatmullRom))
}

/// `get_aspect_ratio_preserving_size` from transformers' EmbeddingGemma 2 processor.
/// Returns (height, width).
pub fn target_size(height: u32, width: u32, max_patches: u32) -> Result<(u32, u32)> {
    let total_px = height as f64 * width as f64;
    let target_px = max_patches as f64 * (PATCH * PATCH) as f64;
    let factor = (target_px / total_px).sqrt();
    let side = (POOL * PATCH) as f64;
    let mut th = ((factor * height as f64) / side).floor() as u32 * side as u32;
    let mut tw = ((factor * width as f64) / side).floor() as u32 * side as u32;
    if th == 0 && tw == 0 {
        bail!("image {width}x{height} is too thin to resize");
    }
    let max_side = (max_patches / (POOL * POOL)) * side as u32;
    if th == 0 {
        th = side as u32;
        tw = ((width / height) * side as u32).min(max_side);
    } else if tw == 0 {
        tw = side as u32;
        th = ((height / width) * side as u32).min(max_side);
    }
    if th as f64 * tw as f64 > target_px {
        bail!("image {width}x{height} does not fit {max_patches} patches");
    }
    Ok((th, tw))
}

/// Pooled vision tokens an image of this size gets (excluding the begin/end markers).
pub fn image_tokens(height: u32, width: u32) -> u32 {
    (height / PATCH) * (width / PATCH) / (POOL * POOL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_the_reference_processor() {
        // Token counts from the reference run (tests/reference/reference.json): n_tokens - 4.
        let (h, w) = target_size(416, 500, 280 * 9).unwrap();
        assert_eq!(image_tokens(h, w), 270);
        let (h, w) = target_size(343, 500, 280 * 9).unwrap();
        assert_eq!(image_tokens(h, w), 260);
        let (h, w) = target_size(333, 500, 280 * 9).unwrap();
        assert_eq!(image_tokens(h, w), 260);
        // Video frames use the 140-token budget.
        let (h, w) = target_size(328, 480, 140 * 9).unwrap();
        assert_eq!((h, w), (432, 672));
        assert_eq!(image_tokens(h, w), 126);
    }

    #[test]
    fn thin_images_get_one_row() {
        let (h, w) = target_size(10, 4000, 280 * 9).unwrap();
        assert_eq!((h, w), (48, 13440));
        assert_eq!(image_tokens(h, w), 280);
    }
}
