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

type Renderer = Box<dyn Fn(&Path) -> anyhow::Result<Vec<u8>>>;

/// A document page (1-based) or a video frame at `t` seconds; images are their own preview.
pub fn preview(svc: &Service, id: i64, page: Option<u32>, t: Option<f64>) -> Result<Render> {
    let kind = svc.item_kind(id)?;
    let tools = Tools::cached();
    let (variant, make): (String, Renderer) = match kind {
        Kind::Image => return Ok(Render::Original),
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
