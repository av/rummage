//! Thumbnails and previews served by the API: from the cache, else made now and cached.

use std::path::{Path, PathBuf};

use rummage_core::service::{Error, Result, Service};
use rummage_core::types::{Kind, Waveform};

use super::{Tools, audio, image, pdf, thumb, video};

/// Where a file's sound peaks are kept: `previews/<id>/peaks.json`.
pub fn peaks_path(svc: &Service, id: i64) -> PathBuf {
    svc.cache_path("previews", id, "peaks")
        .with_extension("json")
}

/// The peaks the indexer stored for an audio or video file, scaled to `points`.
pub fn waveform(svc: &Service, id: i64, points: u32) -> Result<Waveform> {
    let kind = svc.item_kind(id)?;
    if !matches!(kind, Kind::Audio | Kind::Video) {
        return Err(Error::Unsupported(format!(
            "item {id} is not audio or video"
        )));
    }
    let stored: Option<Waveform> = std::fs::read(peaks_path(svc, id))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let Some(w) = stored else {
        return Err(Error::NotFound(format!(
            "no waveform of item {id} (not indexed yet, or it has no sound)"
        )));
    };
    Ok(Waveform {
        duration: w.duration,
        peaks: audio::downsample_peaks(&w.peaks, points.clamp(1, 2000) as usize),
    })
}

/// What to send back for a thumbnail or preview request.
pub enum Render {
    /// A cached WebP file.
    File(PathBuf),
    /// The original file (an image that couldn't be scaled, or that is its own preview).
    Original,
}

fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::Internal(e.into()))?;
    }
    std::fs::write(path, bytes).map_err(|e| Error::Internal(e.into()))
}

/// A thumbnail at most `size` pixels on its long edge; for video, of the window holding `t`.
pub fn thumbnail(svc: &Service, id: i64, size: u32, t: Option<f64>) -> Result<Render> {
    let size = size.clamp(32, 2048);
    let kind = svc.item_kind(id)?;
    if kind == Kind::Video
        && let Some(t) = t
    {
        let start = (t.max(0.0) as usize / video::WINDOW_SECS) * video::WINDOW_SECS;
        let poster = svc.cache_path("previews", id, &video::poster_variant(start));
        if poster.exists() {
            return Ok(Render::File(poster));
        }
    }
    let cached = svc.cache_path("thumbs", id, &size.to_string());
    if cached.exists() {
        return Ok(Render::File(cached));
    }
    if kind == Kind::Image {
        let (path, _) = svc.item_path(id)?;
        return Ok(
            match image::load(Path::new(&path), Tools::cached().ffmpeg) {
                Ok(img) => {
                    save(&cached, &thumb::thumbnail(&img, size))?;
                    Render::File(cached)
                }
                Err(_) => Render::Original,
            },
        );
    }
    // Other kinds: scale the thumbnail made while indexing.
    let default = svc.cache_path("thumbs", id, &svc.config().index.thumbnail_size.to_string());
    match ::image::open(&default) {
        Ok(img) => {
            save(&cached, &thumb::thumbnail(&img, size))?;
            Ok(Render::File(cached))
        }
        Err(_) => Err(Error::NotFound(format!("no thumbnail of item {id} yet"))),
    }
}

/// Longest edge of an image's preview.
pub const PREVIEW_SIZE: u32 = 2048;

/// Image files every browser shows as they are (and turns upright from their EXIF).
const BROWSER_IMAGE_EXT: &[&str] = &["jpg", "jpeg", "jpe", "png", "webp", "gif", "bmp"];

/// An image's preview: the file itself when a browser can show it and it is at most
/// [`PREVIEW_SIZE`] (always for GIFs, which may move), else an upright WebP scaled to fit.
fn image_preview(path: &Path, cached: &Path) -> Result<Render> {
    let e = super::ext(path);
    let small = BROWSER_IMAGE_EXT.contains(&e.as_str())
        && ::image::image_dimensions(path).is_ok_and(|(w, h)| w.max(h) <= PREVIEW_SIZE);
    if small || e == "gif" {
        return Ok(Render::Original);
    }
    if cached.exists() {
        return Ok(Render::File(cached.to_path_buf()));
    }
    Ok(match image::load(path, Tools::cached().ffmpeg) {
        Ok(img) => {
            save(cached, &thumb::thumbnail(&img, PREVIEW_SIZE))?;
            Render::File(cached.to_path_buf())
        }
        Err(_) => Render::Original,
    })
}

type Renderer = Box<dyn Fn(&Path) -> anyhow::Result<Vec<u8>>>;

/// A document page (1-based), a video frame at `t` seconds, or an image fit for a browser.
pub fn preview(svc: &Service, id: i64, page: Option<u32>, t: Option<f64>) -> Result<Render> {
    let kind = svc.item_kind(id)?;
    let tools = Tools::cached();
    let (variant, make): (String, Renderer) = match kind {
        Kind::Image => {
            let (path, _) = svc.item_path(id)?;
            let cached = svc.cache_path("thumbs", id, &PREVIEW_SIZE.to_string());
            return image_preview(Path::new(&path), &cached);
        }
        Kind::Doc if tools.pdftoppm => {
            let page = page.unwrap_or(1).max(1);
            (
                format!("p{page}"),
                Box::new(move |p| pdf::render_page(p, page)),
            )
        }
        Kind::Video if tools.ffmpeg => {
            let t = t.unwrap_or(0.0).max(0.0).floor();
            (
                format!("t{t}"),
                Box::new(move |p| video::render_frame(p, t)),
            )
        }
        _ => {
            return Err(Error::NotFound(format!(
                "no preview of item {id}: {}",
                match kind {
                    Kind::Doc => "page rendering needs pdftoppm (poppler-utils)",
                    Kind::Video => "frame previews need ffmpeg",
                    _ => "only documents and videos have previews",
                }
            )));
        }
    };
    let cached = svc.cache_path("previews", id, &variant);
    if cached.exists() {
        return Ok(Render::File(cached));
    }
    let (path, _) = svc.item_path(id)?;
    let bytes = make(Path::new(&path))
        .map_err(|e| Error::NotFound(format!("can't render item {id}: {e:#}")))?;
    save(&cached, &bytes)?;
    Ok(Render::File(cached))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use ::image::{ImageFormat, Rgb, RgbImage};

    use super::*;

    /// A JPEG of `w` x `h` stored pixels, red down its left edge, tagged EXIF Orientation 6:
    /// shown turned 90° clockwise, so the red edge is the top of the upright picture.
    fn rotated_jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = RgbImage::from_fn(w, h, |x, _| {
            if x < w / 10 {
                Rgb([220, 30, 30])
            } else {
                Rgb([40, 90, 200])
            }
        });
        let mut jpeg = Vec::new();
        img.write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
            .unwrap();
        // APP1 "Exif": a big-endian TIFF header and one IFD entry, Orientation (0x0112) = 6.
        let tiff = [
            b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0,
            0,
        ];
        let mut app1 = vec![0xFF, 0xE1];
        app1.extend_from_slice(&((2 + 6 + tiff.len()) as u16).to_be_bytes());
        app1.extend_from_slice(b"Exif\0\0");
        app1.extend_from_slice(&tiff);
        jpeg.splice(2..2, app1);
        jpeg
    }

    #[test]
    fn image_previews_are_upright_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("portrait.jpg");
        std::fs::write(&big, rotated_jpeg(2400, 1350)).unwrap();
        let cached = dir.path().join("cache").join("2048.webp");
        let Render::File(file) = image_preview(&big, &cached).unwrap() else {
            panic!("a large photo gets a scaled preview");
        };
        assert_eq!(file, cached);
        let img = ::image::open(&file).unwrap().to_rgb8();
        assert_eq!(
            img.dimensions(),
            (1152, 2048),
            "upright portrait, long edge capped"
        );
        let top = img.get_pixel(576, 30);
        assert!(top[0] > 150 && top[2] < 100, "red edge on top: {top:?}");
        assert!(matches!(
            image_preview(&big, &cached).unwrap(),
            Render::File(_)
        ));

        // Small files a browser can show (it turns them upright itself) are sent as they are.
        let small = dir.path().join("small.jpg");
        std::fs::write(&small, rotated_jpeg(400, 225)).unwrap();
        let other = dir.path().join("small.webp");
        assert!(matches!(
            image_preview(&small, &other).unwrap(),
            Render::Original
        ));
        assert!(!other.exists());
    }
}
