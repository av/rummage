//! PDF: the text of each page as passages, and each page rendered as an image (the model
//! reads visual documents well). Locator = page.
//!
//! Text comes from poppler's `pdftotext` when it is installed, else from a built-in reader
//! (lopdf). Page images need poppler's `pdftoppm`; without it PDFs are indexed by text only.
//! poppler is an optional outside tool like ffmpeg, so the program stays one file; pdfium
//! would have meant shipping or downloading a shared library next to it.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use rummage_core::store::ChunkRow;
use rummage_core::types::SegmentKind;
use rummage_core::{EmbedInput, ImageData};

use super::{Ctx, Meta, Outcome, Piece, Sink, TempDir, run, text, thumb};

/// Pages rendered as images per document; text is indexed for every page.
pub const MAX_PAGE_IMAGES: usize = 300;
/// Longest edge of page images for the embedder.
const PAGE_EDGE: u32 = 1024;
/// Longest edge of on-demand page previews.
pub const PREVIEW_EDGE: u32 = 1400;

pub fn extract(path: &Path, ctx: &Ctx, sink: &mut Sink) -> Result<Outcome> {
    let pages = page_texts(path, ctx.tools.pdftotext)?;
    let title = path
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut any_text = false;
    for (i, page) in pages.iter().enumerate() {
        if !page.trim().is_empty() {
            any_text = true;
            text::emit_passages(&title, page, Some(i as i64 + 1), sink)?;
        }
    }
    let mut meta = Meta::default();
    let mut any_page = false;
    if ctx.tools.pdftoppm {
        let count = pages.len().clamp(1, MAX_PAGE_IMAGES);
        let tmp = TempDir::new("pdf")?;
        let prefix = tmp.path().join("p");
        run(Command::new("pdftoppm")
            .args(["-png", "-scale-to", &PAGE_EDGE.to_string()])
            .args(["-f", "1", "-l", &count.to_string()])
            .arg(path)
            .arg(&prefix))?;
        let mut files: Vec<_> = std::fs::read_dir(tmp.path())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "png"))
            .collect();
        // pdftoppm names pages p-1.png or p-001.png; sort by the number.
        files.sort_by_key(|p| page_number(p));
        for file in files {
            let Some(page) = page_number(&file) else {
                continue;
            };
            let img = image::open(&file).context("reading a rendered page")?;
            if page == 1 {
                meta.thumb = Some(thumb::thumbnail(&img, ctx.thumb_size));
            }
            // An empty page embeds as a picture of nothing, which sits close to every query
            // and topped unrelated searches; its text (if any) is indexed above.
            if is_blank(&img.to_luma8()) {
                continue;
            }
            any_page = true;
            let rgb = img.to_rgb8();
            sink(Piece {
                chunk: ChunkRow {
                    kind: SegmentKind::Page,
                    page: Some(page as i64),
                    ..Default::default()
                },
                input: EmbedInput::Image(ImageData::Rgb8 {
                    width: rgb.width(),
                    height: rgb.height(),
                    pixels: rgb.into_raw(),
                }),
            })?;
        }
    } else if !any_text {
        return Ok(Outcome::Skipped(
            "this PDF has no text layer, and its pages can't be rendered without pdftoppm \
             (poppler-utils)"
                .into(),
        ));
    }
    if !any_text && !any_page {
        text::emit_passages(&title, "", None, sink)?;
    }
    Ok(Outcome::Done(meta))
}

/// Whether a page is empty: fewer than 0.05% of its pixels stand out from its background (the
/// most common grey level, so dark slides count too).
fn is_blank(img: &image::GrayImage) -> bool {
    let mut hist = [0usize; 256];
    for p in img.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let bg = (0..256).max_by_key(|&v| hist[v]).unwrap_or(255) as i32;
    let ink: usize = (0..256)
        .filter(|&v| (v as i32 - bg).abs() > 48)
        .map(|v| hist[v])
        .sum();
    ink * 2000 < (img.width() * img.height()) as usize
}

fn page_number(p: &Path) -> Option<usize> {
    p.file_stem()?.to_str()?.rsplit('-').next()?.parse().ok()
}

/// The text of every page.
pub fn page_texts(path: &Path, pdftotext: bool) -> Result<Vec<String>> {
    if pdftotext {
        let out = run(Command::new("pdftotext")
            .args(["-enc", "UTF-8"])
            .arg(path)
            .arg("-"))?;
        let text = String::from_utf8_lossy(&out);
        // Pages end with a form feed.
        let mut pages: Vec<String> = text.split('\u{c}').map(str::to_string).collect();
        if pages.last().is_some_and(|p| p.trim().is_empty()) {
            pages.pop();
        }
        return Ok(pages);
    }
    let bytes = std::fs::read(path)?;
    // The built-in reader can panic on damaged files; treat that as an error.
    std::panic::catch_unwind(|| -> Result<Vec<String>> {
        let doc = lopdf::Document::load_mem(&bytes).context("reading PDF")?;
        let numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
        Ok(numbers
            .iter()
            .map(|n| doc.extract_text(&[*n]).unwrap_or_default())
            .collect())
    })
    .unwrap_or_else(|_| Err(anyhow::anyhow!("the PDF reader crashed on this file")))
}

/// Renders one page for the preview endpoint (WebP).
pub fn render_page(path: &Path, page: u32) -> Result<Vec<u8>> {
    let png = run(Command::new("pdftoppm")
        .args([
            "-png",
            "-singlefile",
            "-scale-to",
            &PREVIEW_EDGE.to_string(),
        ])
        .args(["-f", &page.to_string(), "-l", &page.to_string()])
        // Without an output name, pdftoppm writes the image to stdout.
        .arg(path))?;
    let img = image::load_from_memory(&png).context("reading the rendered page")?;
    Ok(thumb::webp(&img.to_rgb8()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_pages() {
        let white = image::GrayImage::from_pixel(100, 140, image::Luma([255]));
        assert!(is_blank(&white));
        let dark = image::GrayImage::from_pixel(100, 140, image::Luma([20]));
        assert!(is_blank(&dark));
        // A lone page number (5 pixels) is still empty.
        let mut numbered = white.clone();
        for x in 0..5 {
            numbered.put_pixel(50 + x, 130, image::Luma([0]));
        }
        assert!(is_blank(&numbered));
        // One line of text (100 pixels, 0.7%) is not.
        let mut text = white.clone();
        for x in 0..100 {
            text.put_pixel(x, 20, image::Luma([0]));
        }
        assert!(!is_blank(&text));
        // Light grey text on a white page counts as ink too.
        let mut grey = white;
        for x in 0..100 {
            grey.put_pixel(x, 20, image::Luma([180]));
        }
        assert!(!is_blank(&grey));
    }
}
