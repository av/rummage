//! The interface between rummage and an embedding engine.
//!
//! The real engine (EmbeddingGemma 2 on llama.cpp) lives in `rummage-embed`. Everything else
//! talks to it through [`Embedder`], so tests can use the deterministic [`FakeEmbedder`].

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Turns content into unit-length vectors that share one space across modalities.
pub trait Embedder: Send + Sync {
    /// Length of the vectors this embedder returns.
    fn dim(&self) -> usize;
    /// Embeds a batch. Returns one unit-length vector of [`Embedder::dim`] floats per input, in order.
    fn embed(&self, inputs: &[EmbedInput]) -> Result<Vec<Vec<f32>>>;
    /// Which kinds of input this embedder accepts.
    fn modalities(&self) -> Modalities;
    /// Short human-readable name, shown by `/api/v1/health` and `rummage doctor`.
    fn name(&self) -> String {
        "unnamed".into()
    }
}

/// One thing to embed. Text prefixes are applied by the engine, not by callers.
#[derive(Debug, Clone, PartialEq)]
pub enum EmbedInput {
    /// A search query; embedded as `task: search result | query: {q}`.
    Query(String),
    /// A document passage; embedded as `title: {title or none} | text: {text}`.
    Document { title: Option<String>, text: String },
    /// A single image (a photo, a rendered PDF page).
    Image(ImageData),
    /// Mono audio resampled to 16 kHz, samples in -1.0..=1.0.
    Audio { samples_16k_mono: Vec<f32> },
    /// Video frames sampled at 1 fps.
    VideoFrames(Vec<ImageData>),
}

/// Image pixels, either still encoded (PNG, JPEG, WebP, ...) or already decoded.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageData {
    /// The bytes of an encoded image file.
    Encoded(Vec<u8>),
    /// Packed 8-bit RGB pixels, row-major, `width * height * 3` bytes.
    Rgb8 {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
}

impl ImageData {
    fn bytes(&self) -> &[u8] {
        match self {
            ImageData::Encoded(b) => b,
            ImageData::Rgb8 { pixels, .. } => pixels,
        }
    }
}

/// The input kinds an embedder supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Modalities {
    pub text: bool,
    pub image: bool,
    pub audio: bool,
    pub video: bool,
}

impl Modalities {
    pub const ALL: Modalities = Modalities {
        text: true,
        image: true,
        audio: true,
        video: true,
    };

    pub fn supports(&self, input: &EmbedInput) -> bool {
        match input {
            EmbedInput::Query(_) | EmbedInput::Document { .. } => self.text,
            EmbedInput::Image(_) => self.image,
            EmbedInput::Audio { .. } => self.audio,
            EmbedInput::VideoFrames(_) => self.video,
        }
    }
}

/// The text the model sees for a query.
pub fn query_prompt(query: &str) -> String {
    format!("task: search result | query: {query}")
}

/// The text the model sees for a document passage.
pub fn document_prompt(title: Option<&str>, text: &str) -> String {
    format!("title: {} | text: {text}", title.unwrap_or("none"))
}

/// Cuts a Matryoshka vector to `dim` and scales it back to unit length.
pub fn truncate_normalize(v: &mut Vec<f32>, dim: usize) {
    v.truncate(dim);
    normalize(v);
}

/// Scales a vector to unit length (a zero vector stays zero).
pub fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

/// Dot product of two equal-length vectors (cosine similarity for unit vectors).
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut acc = [0f32; 8];
    let (ca, ra) = a.as_chunks::<8>();
    let (cb, rb) = b.as_chunks::<8>();
    let tail: f32 = ra.iter().zip(rb).map(|(x, y)| x * y).sum();
    for (x, y) in ca.iter().zip(cb) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    acc.iter().sum::<f32>() + tail
}

/// A deterministic stand-in for tests and model-less runs.
///
/// Text becomes a normalised sum of one pseudo-random vector per lowercased word, so texts that
/// share words are close and a query equals a document with the same words (prefixes are
/// ignored). Images, audio and video hash their raw bytes.
#[derive(Debug, Clone)]
pub struct FakeEmbedder {
    dim: usize,
}

impl FakeEmbedder {
    pub fn new(dim: usize) -> Self {
        assert!(dim > 0, "dim must be positive");
        Self { dim }
    }

    fn text(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dim];
        let mut any = false;
        for word in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            any = true;
            let w = self.seeded(fnv1a(word.to_lowercase().as_bytes()));
            v.iter_mut().zip(w).for_each(|(a, b)| *a += b);
        }
        if !any {
            v = self.seeded(fnv1a(text.as_bytes()));
        }
        normalize(&mut v);
        v
    }

    fn bytes(&self, bytes: &[u8]) -> Vec<f32> {
        let mut v = self.seeded(fnv1a(bytes));
        normalize(&mut v);
        v
    }

    fn seeded(&self, seed: u64) -> Vec<f32> {
        let mut state = seed;
        (0..self.dim)
            .map(|_| {
                let x = splitmix64(&mut state);
                (x >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
            })
            .collect()
    }
}

impl Embedder for FakeEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, inputs: &[EmbedInput]) -> Result<Vec<Vec<f32>>> {
        inputs
            .iter()
            .map(|input| {
                Ok(match input {
                    EmbedInput::Query(q) => self.text(q),
                    EmbedInput::Document { text, .. } => self.text(text),
                    EmbedInput::Image(img) => self.bytes(img.bytes()),
                    EmbedInput::Audio { samples_16k_mono } => {
                        let raw: Vec<u8> = samples_16k_mono
                            .iter()
                            .flat_map(|s| s.to_le_bytes())
                            .collect();
                        self.bytes(&raw)
                    }
                    EmbedInput::VideoFrames(frames) => {
                        if frames.is_empty() {
                            bail!("video input has no frames");
                        }
                        let mut v = vec![0f32; self.dim];
                        for f in frames {
                            v.iter_mut()
                                .zip(self.bytes(f.bytes()))
                                .for_each(|(a, b)| *a += b);
                        }
                        normalize(&mut v);
                        v
                    }
                })
            })
            .collect()
    }

    fn modalities(&self) -> Modalities {
        Modalities::ALL
    }

    fn name(&self) -> String {
        format!("fake (hash, {} dims)", self.dim)
    }
}

/// 64-bit FNV-1a hash.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(e: &FakeEmbedder, input: EmbedInput) -> Vec<f32> {
        e.embed(&[input]).unwrap().pop().unwrap()
    }

    #[test]
    fn fake_is_deterministic_unit_and_sized() {
        let e = FakeEmbedder::new(256);
        let a = one(&e, EmbedInput::Query("red bicycle".into()));
        let b = one(&e, EmbedInput::Query("red bicycle".into()));
        assert_eq!(a, b);
        assert_eq!(a.len(), 256);
        assert!((dot(&a, &a) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn fake_query_matches_document_with_shared_words() {
        let e = FakeEmbedder::new(768);
        let q = one(&e, EmbedInput::Query("Red Bicycle".into()));
        let near = one(
            &e,
            EmbedInput::Document {
                title: None,
                text: "a red bicycle by the lake".into(),
            },
        );
        let far = one(
            &e,
            EmbedInput::Document {
                title: None,
                text: "quarterly tax invoice".into(),
            },
        );
        assert!(dot(&q, &near) > dot(&q, &far) + 0.3);
    }

    #[test]
    fn truncate_renormalises() {
        let e = FakeEmbedder::new(768);
        let mut v = one(&e, EmbedInput::Image(ImageData::Encoded(vec![1, 2, 3])));
        truncate_normalize(&mut v, 128);
        assert_eq!(v.len(), 128);
        assert!((dot(&v, &v) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn prompts_follow_the_model_card() {
        assert_eq!(query_prompt("cats"), "task: search result | query: cats");
        assert_eq!(document_prompt(None, "hi"), "title: none | text: hi");
        assert_eq!(document_prompt(Some("T"), "hi"), "title: T | text: hi");
    }
}
