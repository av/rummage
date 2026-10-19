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
    /// False while the model loads in the background (or after it failed to load). Searches
    /// then match keywords only instead of waiting.
    fn ready(&self) -> bool {
        true
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
/// Uses AVX2 and FMA when the CPU has them.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    dot_fn()(a, b)
}

/// The fastest dot product this CPU supports, picked once; call it in hot loops.
pub fn dot_fn() -> fn(&[f32], &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        static AVX2: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *AVX2.get_or_init(|| {
            std::arch::is_x86_feature_detected!("avx2")
                && std::arch::is_x86_feature_detected!("fma")
        }) {
            return |a, b| {
                // SAFETY: the CPU supports AVX2 and FMA (checked above).
                unsafe { dot_avx2(a, b) }
            };
        }
    }
    dot_portable
}

fn dot_portable(a: &[f32], b: &[f32]) -> f32 {
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

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn dot_avx2(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;
    let n = a.len().min(b.len());
    let (pa, pb) = (a.as_ptr(), b.as_ptr());
    let mut acc = [_mm256_setzero_ps(); 4];
    let mut i = 0;
    // SAFETY: every load reads 8 floats at offsets below `n`.
    unsafe {
        while i + 32 <= n {
            for (k, acc) in acc.iter_mut().enumerate() {
                let x = _mm256_loadu_ps(pa.add(i + k * 8));
                let y = _mm256_loadu_ps(pb.add(i + k * 8));
                *acc = _mm256_fmadd_ps(x, y, *acc);
            }
            i += 32;
        }
        while i + 8 <= n {
            acc[0] = _mm256_fmadd_ps(
                _mm256_loadu_ps(pa.add(i)),
                _mm256_loadu_ps(pb.add(i)),
                acc[0],
            );
            i += 8;
        }
        let sum = _mm256_add_ps(_mm256_add_ps(acc[0], acc[1]), _mm256_add_ps(acc[2], acc[3]));
        let mut lanes = [0f32; 8];
        _mm256_storeu_ps(lanes.as_mut_ptr(), sum);
        let mut total: f32 = lanes.iter().sum();
        while i < n {
            total += *pa.add(i) * *pb.add(i);
            i += 1;
        }
        total
    }
}

/// Shares one embedder between searches and the indexer, searches first: a background
/// (indexing) call waits while any search is embedding, so a query waits at most for the
/// one indexing call already running.
pub struct EmbedQueue {
    inner: std::sync::Arc<dyn Embedder>,
    background: std::sync::Mutex<()>,
    queries: std::sync::Mutex<usize>,
    idle: std::sync::Condvar,
}

impl EmbedQueue {
    pub fn new(inner: std::sync::Arc<dyn Embedder>) -> Self {
        Self {
            inner,
            background: std::sync::Mutex::new(()),
            queries: std::sync::Mutex::new(0),
            idle: std::sync::Condvar::new(),
        }
    }

    pub fn embedder(&self) -> &std::sync::Arc<dyn Embedder> {
        &self.inner
    }

    /// Embeds for a search, ahead of any waiting indexing work.
    pub fn query(&self, inputs: &[EmbedInput]) -> Result<Vec<Vec<f32>>> {
        *self.queries.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        let out = self.inner.embed(inputs);
        let mut n = self.queries.lock().unwrap_or_else(|e| e.into_inner());
        *n -= 1;
        if *n == 0 {
            self.idle.notify_all();
        }
        out
    }

    /// Embeds for the indexer, one caller at a time, after any running searches.
    pub fn background(&self, inputs: &[EmbedInput]) -> Result<Vec<Vec<f32>>> {
        let _turn = self.background.lock().unwrap_or_else(|e| e.into_inner());
        let mut n = self.queries.lock().unwrap_or_else(|e| e.into_inner());
        while *n > 0 {
            n = self.idle.wait(n).unwrap_or_else(|e| e.into_inner());
        }
        drop(n);
        self.inner.embed(inputs)
    }
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
    fn simd_dot_matches_portable() {
        let e = FakeEmbedder::new(771);
        let a = one(&e, EmbedInput::Query("alpha".into()));
        let b = one(&e, EmbedInput::Query("beta gamma".into()));
        for n in [0, 1, 7, 8, 31, 32, 33, 768, 771] {
            let (x, y) = (&a[..n], &b[..n]);
            assert!((dot(x, y) - dot_portable(x, y)).abs() < 1e-5, "n={n}");
        }
    }

    #[test]
    fn embed_queue_serves_both_kinds() {
        let q = EmbedQueue::new(std::sync::Arc::new(FakeEmbedder::new(8)));
        let a = q.query(&[EmbedInput::Query("x".into())]).unwrap();
        let b = q.background(&[EmbedInput::Query("x".into())]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn prompts_follow_the_model_card() {
        assert_eq!(query_prompt("cats"), "task: search result | query: cats");
        assert_eq!(document_prompt(None, "hi"), "title: none | text: hi");
        assert_eq!(document_prompt(Some("T"), "hi"), "title: T | text: hi");
    }
}
