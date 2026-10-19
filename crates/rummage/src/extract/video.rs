//! Video through an ffmpeg subprocess: 1 frame per second, embedded in 10 s windows, plus
//! the audio track through the audio path, with a poster per window. Locator = t_start/t_end.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use image::DynamicImage;
use rummage_core::store::ChunkRow;
use rummage_core::types::SegmentKind;
use rummage_core::{EmbedInput, ImageData};
use serde_json::Value;

use super::{Ctx, Meta, Outcome, Piece, Sink, audio, run, thumb};

/// Seconds (= frames) per video window.
pub const WINDOW_SECS: usize = 10;
/// Frames are scaled to fit this box before embedding.
const FRAME_BOX: u32 = 768;
/// Longest edge of on-demand frame previews.
pub const PREVIEW_EDGE: u32 = 1280;
/// Longest video indexed, in seconds (3 hours).
const MAX_SECS: usize = 3 * 3600;

pub fn extract(path: &Path, ctx: &Ctx, sink: &mut Sink) -> Result<Outcome> {
    if !ctx.tools.ffmpeg {
        return Ok(Outcome::Skipped(
            "videos need ffmpeg, which was not found; install it and rescan".into(),
        ));
    }
    let probe = probe(path)?;
    if !probe.has_video {
        bail!("no video stream");
    }
    let mut meta = Meta {
        date: probe.created,
        exif: probe.tags,
        ..Default::default()
    };

    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-t", &MAX_SECS.to_string()])
        .args([
            "-vf",
            &format!("fps=1,scale={FRAME_BOX}:{FRAME_BOX}:force_original_aspect_ratio=decrease"),
        ])
        .args(["-f", "image2pipe", "-c:v", "png", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting ffmpeg")?;
    let mut stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let errors = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::BufReader::new(stderr).read_to_string(&mut s);
        s
    });

    let mut frames: Vec<DynamicImage> = vec![];
    let mut window = 0usize;
    let mut sent = 0usize;
    let mut flush =
        |frames: &mut Vec<DynamicImage>, window: usize, meta: &mut Meta| -> Result<()> {
            if frames.is_empty() {
                return Ok(());
            }
            let start = window * WINDOW_SECS;
            // Poster: the window's second frame (the first is often black at the very start).
            let poster = &frames[1.min(frames.len() - 1)];
            let poster_webp = thumb::thumbnail(poster, ctx.thumb_size);
            if window == 0 {
                meta.thumb = Some(poster_webp.clone());
            }
            meta.previews.push((poster_variant(start), poster_webp));
            let input = EmbedInput::VideoFrames(
                frames
                    .drain(..)
                    .map(|f| {
                        let rgb = f.to_rgb8();
                        ImageData::Rgb8 {
                            width: rgb.width(),
                            height: rgb.height(),
                            pixels: rgb.into_raw(),
                        }
                    })
                    .collect(),
            );
            let n = match &input {
                EmbedInput::VideoFrames(f) => f.len(),
                _ => 0,
            };
            sink(Piece {
                chunk: ChunkRow {
                    kind: SegmentKind::Video,
                    t_start: Some(start as f64),
                    t_end: Some((start + n) as f64),
                    ..Default::default()
                },
                input,
            })
        };
    let mut reader = PngStream::new(&mut stdout);
    let read_result: Result<()> = (|| {
        while let Some(png) = reader.next_png()? {
            frames.push(image::load_from_memory(&png).context("decoding a frame")?);
            if frames.len() == WINDOW_SECS {
                flush(&mut frames, window, &mut meta)?;
                window += 1;
                sent += 1;
            }
        }
        if !frames.is_empty() {
            flush(&mut frames, window, &mut meta)?;
            sent += 1;
        }
        Ok(())
    })();
    if read_result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    let err = errors.join().unwrap_or_default();
    read_result?;
    if let Some(codec) = missing_decoder(&err) {
        return Ok(Outcome::Skipped(format!(
            "this ffmpeg can't decode {codec} video (Fedora's ffmpeg-free lacks HEVC/H.265, for \
             example); install a full ffmpeg build and rescan"
        )));
    }
    if !status.success() && sent == 0 {
        bail!(
            "ffmpeg failed: {}",
            err.lines().last().unwrap_or("no output")
        );
    }
    if sent == 0 {
        // Clips shorter than a second give no frame at 1 fps: take the first one.
        let png = run(Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args(["-map", "0:v:0", "-frames:v", "1", "-vf"])
            .arg(format!(
                "scale={FRAME_BOX}:{FRAME_BOX}:force_original_aspect_ratio=decrease"
            ))
            .args(["-f", "image2pipe", "-c:v", "png", "-"]))?;
        if png.is_empty() {
            bail!("no frames could be read");
        }
        frames.push(image::load_from_memory(&png).context("decoding a frame")?);
        flush(&mut frames, 0, &mut meta)?;
    }

    if probe.has_audio {
        let pcm = run(Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args(["-map", "0:a:0", "-t", &MAX_SECS.to_string()])
            .args([
                "-ac",
                "1",
                "-ar",
                &audio::RATE.to_string(),
                "-f",
                "f32le",
                "-",
            ]));
        match pcm {
            Ok(bytes) => {
                let samples: Vec<f32> = bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect();
                // Silent tracks add nothing.
                if samples.iter().any(|s| s.abs() > 1e-4) {
                    audio::emit_windows(&samples, 0.0, sink)?;
                    meta.peaks = Some((
                        samples.len() as f64 / audio::RATE as f64,
                        audio::peaks(&samples, audio::PEAKS),
                    ));
                }
            }
            Err(e) => tracing::debug!("audio track of {}: {e:#}", path.display()),
        }
    }
    Ok(Outcome::Done(meta))
}

/// The codec ffmpeg reported it has no decoder for, if that is why it failed.
fn missing_decoder(stderr: &str) -> Option<String> {
    let at = stderr.find("no decoder found for: ")? + "no decoder found for: ".len();
    Some(
        stderr[at..]
            .split_whitespace()
            .next()
            .unwrap_or("this")
            .to_uppercase(),
    )
}

/// Cache name of the poster of the window starting at `start` seconds.
pub fn poster_variant(start: usize) -> String {
    format!("poster-{start}")
}

/// What ffprobe says about a file.
struct Probe {
    has_video: bool,
    has_audio: bool,
    created: Option<i64>,
    tags: Option<Value>,
}

fn probe(path: &Path) -> Result<Probe> {
    let out = run(Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path))?;
    let v: Value = serde_json::from_slice(&out).context("reading ffprobe output")?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let has = |t: &str| {
        streams.iter().any(|s| {
            s["codec_type"] == t
                && !(t == "video" && s["disposition"]["attached_pic"].as_i64() == Some(1))
        })
    };
    let fmt_tags = &v["format"]["tags"];
    let created = fmt_tags["creation_time"]
        .as_str()
        .and_then(super::image::parse_exif_date)
        .filter(|t| *t > 86400);
    let mut tags = serde_json::Map::new();
    if let Some(d) = v["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
    {
        tags.insert("duration".into(), serde_json::json!(d));
    }
    if let Some(s) = streams.iter().find(|s| s["codec_type"] == "video") {
        for k in ["width", "height", "codec_name"] {
            if !s[k].is_null() {
                tags.insert(k.into(), s[k].clone());
            }
        }
    }
    for k in [
        "title",
        "artist",
        "comment",
        "location",
        "com.apple.quicktime.make",
        "com.apple.quicktime.model",
    ] {
        if let Some(t) = fmt_tags[k].as_str() {
            tags.insert(
                k.rsplit('.').next().unwrap_or(k).into(),
                Value::String(t.into()),
            );
        }
    }
    Ok(Probe {
        has_video: has("video"),
        has_audio: has("audio"),
        created,
        tags: (!tags.is_empty()).then_some(Value::Object(tags)),
    })
}

/// Splits a stream of concatenated PNG files.
struct PngStream<'a, R: Read> {
    inner: &'a mut R,
    buf: Vec<u8>,
    eof: bool,
}

const IEND: &[u8] = b"IEND\xAE\x42\x60\x82";

impl<'a, R: Read> PngStream<'a, R> {
    fn new(inner: &'a mut R) -> Self {
        Self {
            inner,
            buf: vec![],
            eof: false,
        }
    }

    fn next_png(&mut self) -> Result<Option<Vec<u8>>> {
        let mut searched = 0;
        loop {
            if let Some(p) = self.buf[searched..]
                .windows(IEND.len())
                .position(|w| w == IEND)
            {
                let end = searched + p + IEND.len();
                let png: Vec<u8> = self.buf.drain(..end).collect();
                return Ok(Some(png));
            }
            if self.eof {
                return Ok(None);
            }
            searched = self.buf.len().saturating_sub(IEND.len());
            let mut chunk = [0u8; 64 * 1024];
            let n = self.inner.read(&mut chunk)?;
            if n == 0 {
                self.eof = true;
            } else {
                self.buf.extend_from_slice(&chunk[..n]);
            }
        }
    }
}

/// One frame at `t` seconds, for the preview endpoint (WebP).
pub fn render_frame(path: &Path, t: f64) -> Result<Vec<u8>> {
    let png = run(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-ss",
            &format!("{:.3}", t.max(0.0)),
            "-i",
        ])
        .arg(path)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!(
            "scale={PREVIEW_EDGE}:{PREVIEW_EDGE}:force_original_aspect_ratio=decrease"
        ))
        .args(["-f", "image2pipe", "-c:v", "png", "-"]))?;
    if png.is_empty() {
        bail!("no frame at {t:.1} s");
    }
    let img = image::load_from_memory(&png).context("decoding the frame")?;
    Ok(thumb::webp(&img.to_rgb8()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_missing_decoder() {
        let err = "[vist#0:0/hevc @ 0x56] Decoding requested, but no decoder found for: hevc\n\
                   Error opening output file -.";
        assert_eq!(missing_decoder(err).as_deref(), Some("HEVC"));
        assert_eq!(missing_decoder("Invalid data found"), None);
    }

    #[test]
    fn splits_concatenated_pngs() {
        let mut one = vec![];
        image::DynamicImage::ImageRgb8(image::RgbImage::new(3, 2))
            .write_to(&mut std::io::Cursor::new(&mut one), image::ImageFormat::Png)
            .unwrap();
        let stream: Vec<u8> = [one.clone(), one.clone(), one.clone()].concat();
        let mut src = std::io::Cursor::new(stream);
        let mut s = PngStream::new(&mut src);
        let mut n = 0;
        while let Some(p) = s.next_png().unwrap() {
            assert_eq!(p, one);
            n += 1;
        }
        assert_eq!(n, 3);
    }
}
