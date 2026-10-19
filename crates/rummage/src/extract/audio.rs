//! Audio: decoded with symphonia, mixed to mono at 16 kHz, embedded in 30 s windows that
//! overlap by 5 s, with a waveform thumbnail.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rummage_core::EmbedInput;
use rummage_core::store::ChunkRow;
use rummage_core::types::SegmentKind;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use super::{Ctx, Meta, Outcome, Piece, Sink, thumb};

/// The model's audio sample rate.
pub const RATE: u32 = 16_000;
/// Window length and step, in seconds.
pub const WINDOW_SECS: f64 = 30.0;
pub const STEP_SECS: f64 = 25.0;
/// A trailing window shorter than this is dropped (unless it is the only one).
const MIN_TAIL_SECS: f64 = 2.0;
/// Longest audio indexed, in seconds (6 hours).
const MAX_SECS: u64 = 6 * 3600;

pub fn extract(path: &Path, ctx: &Ctx, sink: &mut Sink) -> Result<Outcome> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let hint = path.extension().and_then(|e| e.to_str());
    let samples = decode(Box::new(file), hint)?;
    if samples.is_empty() {
        bail!("no audio in this file");
    }
    emit_windows(&samples, 0.0, sink)?;
    Ok(Outcome::Done(Meta {
        thumb: Some(thumb::waveform(&samples, ctx.thumb_size)),
        peaks: Some((samples.len() as f64 / RATE as f64, peaks(&samples, PEAKS))),
        ..Default::default()
    }))
}

/// Peak levels kept per file; the API scales them down to what is asked for.
pub const PEAKS: usize = 2000;

/// The loudest sample (0 to 1) in each of `n` equal slices of 16 kHz mono `samples`.
pub fn peaks(samples: &[f32], n: usize) -> Vec<f32> {
    if samples.is_empty() || n == 0 {
        return vec![];
    }
    let n = n.min(samples.len());
    (0..n)
        .map(|i| {
            let (a, b) = (i * samples.len() / n, (i + 1) * samples.len() / n);
            samples[a..b.max(a + 1)]
                .iter()
                .fold(0f32, |m, s| m.max(s.abs()))
                .min(1.0)
        })
        .collect()
}

/// Scales peaks down to `n` points by keeping the loudest of each group.
pub fn downsample_peaks(peaks: &[f32], n: usize) -> Vec<f32> {
    if n == 0 || peaks.len() <= n {
        return peaks.to_vec();
    }
    (0..n)
        .map(|i| {
            let (a, b) = (i * peaks.len() / n, (i + 1) * peaks.len() / n);
            peaks[a..b.max(a + 1)].iter().fold(0f32, |m, p| m.max(*p))
        })
        .collect()
}

/// Sends 16 kHz mono `samples` as overlapping windows; `offset` shifts the locators.
pub fn emit_windows(samples: &[f32], offset: f64, sink: &mut Sink) -> Result<()> {
    for (start, end) in windows(samples.len()) {
        sink(Piece {
            chunk: ChunkRow {
                kind: SegmentKind::Audio,
                t_start: Some(offset + start as f64 / RATE as f64),
                t_end: Some(offset + end as f64 / RATE as f64),
                ..Default::default()
            },
            input: EmbedInput::Audio {
                samples_16k_mono: samples[start..end].to_vec(),
            },
        })?;
    }
    Ok(())
}

/// Sample ranges of the windows over `n` samples.
pub fn windows(n: usize) -> Vec<(usize, usize)> {
    let win = (WINDOW_SECS * RATE as f64) as usize;
    let step = (STEP_SECS * RATE as f64) as usize;
    let min_tail = (MIN_TAIL_SECS * RATE as f64) as usize;
    let mut out = vec![];
    let mut start = 0;
    while start < n {
        let end = (start + win).min(n);
        if out.is_empty() || end - start >= min_tail {
            out.push((start, end));
        }
        if end == n {
            break;
        }
        start += step;
    }
    out
}

/// Decodes the first audio track of a file to 16 kHz mono.
pub fn decode(source: Box<dyn MediaSource>, ext_hint: Option<&str>) -> Result<Vec<f32>> {
    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(e) = ext_hint {
        hint.with_extension(e);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .context("unrecognised audio format")?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .context("no audio track")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("unsupported audio codec")?;
    let mut rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut mono: Vec<f32> = vec![];
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(e).context("reading audio"),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A damaged packet: skip it.
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(e).context("decoding audio"),
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        let channels = spec.channels.count().max(1);
        let need = decoded.capacity() as u64;
        if buf.as_ref().is_none_or(|b| b.capacity() < need as usize) {
            buf = Some(SampleBuffer::new(need, spec));
        }
        let b = buf.as_mut().expect("set above");
        b.copy_interleaved_ref(decoded);
        mono.extend(
            b.samples()
                .chunks(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
        if rate > 0 && mono.len() as u64 > MAX_SECS * rate as u64 {
            break;
        }
    }
    if rate == 0 {
        bail!("unknown sample rate");
    }
    Ok(resample(&mono, rate, RATE))
}

/// Windowed-sinc resampling (with a low-pass when going down).
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let cutoff = ratio.min(1.0);
    const HALF: isize = 16;
    let out_len = (input.len() as f64 * ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let center = i as f64 / ratio;
        let c = center.floor() as isize;
        let mut acc = 0.0f64;
        let mut wsum = 0.0f64;
        for k in (c - HALF + 1)..=(c + HALF) {
            if k < 0 || k as usize >= input.len() {
                continue;
            }
            let x = center - k as f64;
            let sinc = if x.abs() < 1e-9 {
                1.0
            } else {
                let px = std::f64::consts::PI * x * cutoff;
                px.sin() / px
            };
            // Hann window over the kernel.
            let w = 0.5 + 0.5 * (std::f64::consts::PI * x / HALF as f64).cos();
            let weight = sinc * w;
            acc += input[k as usize] as f64 * weight;
            wsum += weight;
        }
        out.push(if wsum.abs() > 1e-12 {
            (acc / wsum) as f32
        } else {
            0.0
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_overlap() {
        let r = RATE as usize;
        assert_eq!(windows(10 * r), vec![(0, 10 * r)]);
        assert_eq!(windows(r / 2), vec![(0, r / 2)]);
        assert_eq!(
            windows(60 * r),
            vec![(0, 30 * r), (25 * r, 55 * r), (50 * r, 60 * r)]
        );
        assert_eq!(
            windows(56 * r),
            vec![(0, 30 * r), (25 * r, 55 * r), (50 * r, 56 * r)]
        );
        assert_eq!(windows(31 * r), vec![(0, 30 * r), (25 * r, 31 * r)]);
        assert_eq!(
            windows(30 * r + r / 2),
            vec![(0, 30 * r), (25 * r, 30 * r + r / 2)]
        );
        assert!(windows(0).is_empty());
    }

    #[test]
    fn peaks_and_downsampling() {
        let s = [0.0, 0.5, -0.9, 0.1, 0.2, -0.3];
        assert_eq!(peaks(&s, 3), vec![0.5, 0.9, 0.3]);
        assert_eq!(peaks(&s, 100).len(), 6);
        assert!(peaks(&[], 10).is_empty());
        assert_eq!(downsample_peaks(&[0.1, 0.5, 0.2, 0.4], 2), vec![0.5, 0.4]);
        assert_eq!(downsample_peaks(&[0.1], 5), vec![0.1]);
    }

    #[test]
    fn resampling_keeps_length_and_tone() {
        let tone: Vec<f32> = (0..44100)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 44100.0).sin())
            .collect();
        let out = resample(&tone, 44100, 16000);
        assert_eq!(out.len(), 16000);
        let rms = (out.iter().map(|x| x * x).sum::<f32>() / out.len() as f32).sqrt();
        assert!((rms - 0.707).abs() < 0.05, "{rms}");
    }

    #[test]
    fn decodes_wav() {
        let mut wav = vec![];
        let rate = 8000u32;
        let samples: Vec<i16> = (0..rate).map(|i| ((i % 40) as i16 - 20) * 500).collect();
        let data_len = samples.len() as u32 * 2;
        wav.extend(b"RIFF");
        wav.extend((36 + data_len).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(rate.to_le_bytes());
        wav.extend((rate * 2).to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(data_len.to_le_bytes());
        samples.iter().for_each(|s| wav.extend(s.to_le_bytes()));
        let out = decode(Box::new(std::io::Cursor::new(wav)), Some("wav")).unwrap();
        assert_eq!(out.len(), 16000);
    }
}
