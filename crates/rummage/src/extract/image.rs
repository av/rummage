//! Photos and pictures: orientation, EXIF (date, camera, GPS), a thumbnail, one image chunk.

use std::io::{BufReader, Cursor};
use std::path::Path;

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageDecoder, ImageReader};
use rummage_core::store::ChunkRow;
use rummage_core::types::SegmentKind;
use rummage_core::{EmbedInput, ImageData};
use serde_json::{Map, Value, json};

use super::{Ctx, FFMPEG_IMAGE_EXT, Meta, Outcome, Piece, RAW_EXT, Sink, ext, run, thumb};

/// Longest edge of the pixels handed to the embedder; the engine scales to its token budget.
const MAX_EMBED_EDGE: u32 = 2048;

pub fn extract(path: &Path, ctx: &Ctx, sink: &mut Sink) -> Result<Outcome> {
    let e = ext(path);
    if FFMPEG_IMAGE_EXT.contains(&e.as_str()) && !ctx.tools.ffmpeg {
        return Ok(Outcome::Skipped(format!(
            "{} photos need ffmpeg, which was not found",
            e.to_uppercase()
        )));
    }
    let img = load(path, ctx.tools.ffmpeg)?;
    let (exif, date) = read_exif(path);
    let meta = Meta {
        date,
        exif: Some(with_size(exif, &img)),
        thumb: Some(thumb::thumbnail(&img, ctx.thumb_size)),
        ..Default::default()
    };
    sink(Piece {
        chunk: ChunkRow {
            kind: SegmentKind::Image,
            ..Default::default()
        },
        input: EmbedInput::Image(embed_pixels(&img)),
    })?;
    Ok(Outcome::Done(meta))
}

/// Decodes any supported image file, upright.
pub fn load(path: &Path, ffmpeg: bool) -> Result<DynamicImage> {
    let e = ext(path);
    if FFMPEG_IMAGE_EXT.contains(&e.as_str()) {
        return via_ffmpeg(path);
    }
    if RAW_EXT.contains(&e.as_str()) {
        let bytes = std::fs::read(path)?;
        if let Some(jpeg) = embedded_jpeg(&bytes)
            && let Ok(img) = decode(&jpeg)
        {
            return Ok(img);
        }
        if ffmpeg {
            return via_ffmpeg(path);
        }
        bail!("no preview found in this RAW file (and no ffmpeg to decode it)");
    }
    let reader = ImageReader::new(BufReader::new(
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?,
    ))
    .with_guessed_format()?;
    upright(reader)
}

/// Decodes an image held in memory, upright.
pub fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    upright(ImageReader::new(Cursor::new(bytes)).with_guessed_format()?)
}

fn upright<R: std::io::BufRead + std::io::Seek>(reader: ImageReader<R>) -> Result<DynamicImage> {
    let mut decoder = reader.into_decoder().context("unrecognised image")?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).context("decoding image")?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(img)
}

fn via_ffmpeg(path: &Path) -> Result<DynamicImage> {
    let mut cmd = super::ffmpeg();
    cmd.args(["-v", "error", "-i"]).arg(path);
    cmd.args(["-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "-"]);
    let png = run(&mut cmd)?;
    image::load_from_memory(&png).context("decoding ffmpeg output")
}

/// The largest JPEG embedded in a file (RAW files carry a full-size preview).
fn embedded_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut best: Option<&[u8]> = None;
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == 0xFF && bytes[i + 1] == 0xD8 && bytes[i + 2] == 0xFF {
            // Find the matching end-of-image marker.
            if let Some(end) = bytes[i + 2..]
                .windows(2)
                .position(|w| w == [0xFF, 0xD9])
                .map(|p| i + 2 + p + 2)
            {
                let candidate = &bytes[i..end];
                if best.is_none_or(|b| candidate.len() > b.len()) {
                    best = Some(candidate);
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    best.filter(|b| b.len() > 16 * 1024).map(<[u8]>::to_vec)
}

/// Pixels for the embedder: upright RGB, at most [`MAX_EMBED_EDGE`] on the long side.
pub fn embed_pixels(img: &DynamicImage) -> ImageData {
    let rgb = if img.width().max(img.height()) > MAX_EMBED_EDGE {
        img.resize(
            MAX_EMBED_EDGE,
            MAX_EMBED_EDGE,
            image::imageops::FilterType::CatmullRom,
        )
        .to_rgb8()
    } else {
        img.to_rgb8()
    };
    ImageData::Rgb8 {
        width: rgb.width(),
        height: rgb.height(),
        pixels: rgb.into_raw(),
    }
}

fn with_size(exif: Option<Value>, img: &DynamicImage) -> Value {
    let mut map = match exif {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    map.insert("width".into(), json!(img.width()));
    map.insert("height".into(), json!(img.height()));
    Value::Object(map)
}

/// The EXIF fields worth keeping (as JSON), and the date taken (Unix seconds, read as UTC).
pub fn read_exif(path: &Path) -> (Option<Value>, Option<i64>) {
    let Ok(file) = std::fs::File::open(path) else {
        return (None, None);
    };
    let Ok(exif) = exif::Reader::new().read_from_container(&mut BufReader::new(file)) else {
        return (None, None);
    };
    use exif::{In, Tag};
    let text = |tag: Tag| {
        exif.get_field(tag, In::PRIMARY).map(|f| {
            f.display_value()
                .to_string()
                .trim_matches('"')
                .trim()
                .to_string()
        })
    };
    let mut map = Map::new();
    let taken = text(Tag::DateTimeOriginal).or_else(|| text(Tag::DateTime));
    let date = taken.as_deref().and_then(parse_exif_date);
    // Keys are the EXIF tag names, as the web UI expects.
    for (key, tag) in [
        ("Make", Tag::Make),
        ("Model", Tag::Model),
        ("LensModel", Tag::LensModel),
        ("FNumber", Tag::FNumber),
        ("ExposureTime", Tag::ExposureTime),
        ("ISO", Tag::PhotographicSensitivity),
        ("FocalLength", Tag::FocalLength),
        ("Software", Tag::Software),
        ("ImageDescription", Tag::ImageDescription),
        ("Artist", Tag::Artist),
    ] {
        if let Some(v) = text(tag).filter(|v| !v.is_empty()) {
            map.insert(key.into(), Value::String(v));
        }
    }
    if let Some(t) = taken {
        map.insert("DateTimeOriginal".into(), Value::String(t));
    }
    let coord = |tag: Tag, reference: Tag| -> Option<f64> {
        let f = exif.get_field(tag, In::PRIMARY)?;
        let exif::Value::Rational(r) = &f.value else {
            return None;
        };
        if r.len() < 3 {
            return None;
        }
        let deg = r[0].to_f64() + r[1].to_f64() / 60.0 + r[2].to_f64() / 3600.0;
        let negative = text(reference).is_some_and(|s| s.starts_with('S') || s.starts_with('W'));
        deg.is_finite().then_some(if negative { -deg } else { deg })
    };
    if let (Some(lat), Some(lon)) = (
        coord(Tag::GPSLatitude, Tag::GPSLatitudeRef),
        coord(Tag::GPSLongitude, Tag::GPSLongitudeRef),
    ) {
        map.insert("GPSLatitude".into(), json!(lat));
        map.insert("GPSLongitude".into(), json!(lon));
    }
    ((!map.is_empty()).then_some(Value::Object(map)), date)
}

/// `2024:05:01 12:30:00` (or with dashes) to Unix seconds, as UTC.
pub fn parse_exif_date(s: &str) -> Option<i64> {
    let digits: Vec<i64> = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let [y, mo, d, rest @ ..] = digits.as_slice() else {
        return None;
    };
    let (h, mi, sec) = (
        rest.first().copied().unwrap_or(0),
        rest.get(1).copied().unwrap_or(0),
        rest.get(2).copied().unwrap_or(0),
    );
    if !(1900..=2200).contains(y) || !(1..=12).contains(mo) || !(1..=31).contains(d) {
        return None;
    }
    Some(days_from_civil(*y, *mo, *d) * 86400 + h * 3600 + mi * 60 + sec)
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exif_dates() {
        assert_eq!(parse_exif_date("1970:01:01 00:00:00"), Some(0));
        assert_eq!(parse_exif_date("2024:05:01 12:30:15"), Some(1714566615));
        assert_eq!(parse_exif_date("2024-05-01"), Some(1714521600));
        assert_eq!(parse_exif_date("0000:00:00 00:00:00"), None);
        assert_eq!(parse_exif_date("garbage"), None);
    }

    #[test]
    fn finds_the_largest_embedded_jpeg() {
        let mut big = vec![0xFF, 0xD8, 0xFF, 0xE0];
        big.extend(std::iter::repeat_n(7u8, 20_000));
        big.extend([0xFF, 0xD9]);
        let mut file = b"RAWHEADER".to_vec();
        file.extend([0xFF, 0xD8, 0xFF, 0x01, 0xFF, 0xD9]);
        file.extend(&big);
        assert_eq!(embedded_jpeg(&file).unwrap(), big);
        assert!(embedded_jpeg(b"nothing here").is_none());
    }

    #[test]
    fn orientation_is_applied() {
        // A 2x1 JPEG with EXIF orientation 6 (rotate 90° clockwise) decodes as 1x2.
        let img = image::RgbImage::from_pixel(20, 10, image::Rgb([255, 0, 0]));
        let mut jpeg = vec![];
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .unwrap();
        // Insert an APP1 EXIF segment with Orientation = 6 after SOI.
        let tiff: Vec<u8> = [
            b"MM\x00\x2a\x00\x00\x00\x08".as_slice(),
            &[0x00, 0x01],
            &[
                0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00,
            ],
            &[0, 0, 0, 0],
        ]
        .concat();
        let mut app1 = b"Exif\x00\x00".to_vec();
        app1.extend(&tiff);
        let len = (app1.len() + 2) as u16;
        let mut out = jpeg[..2].to_vec();
        out.extend([0xFF, 0xE1]);
        out.extend(len.to_be_bytes());
        out.extend(&app1);
        out.extend(&jpeg[2..]);
        let img = decode(&out).unwrap();
        assert_eq!((img.width(), img.height()), (10, 20));
    }
}
