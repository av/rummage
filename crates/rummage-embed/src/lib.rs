//! EmbeddingGemma 2 on llama.cpp: the real [`Embedder`] for rummage.
//!
//! The text model (GGUF) and the vision+audio encoders (mmproj GGUF) are loaded once. Every input
//! becomes one sequence `<bos> ... <eos>`, exactly as the official processor builds it:
//!
//! - query: `task: search result | query: {q}`
//! - document: `title: {title or none} | text: {text}`
//! - image: `<|image>` + soft tokens + `<image|>` (280 tokens by default)
//! - audio: `<|audio>` + 25 soft tokens per second + `<audio|>`, one block per 30 s part
//! - video: one `<|image>` ... `<image|>` block per frame (140 tokens per frame by default)
//!
//! The sequences are mean-pooled by llama.cpp, normalised, and optionally cut to a shorter
//! Matryoshka size and normalised again. Media soft tokens come from libmtmd and are fed to the
//! text model next to the text tokens in one non-causal batch.

pub mod ffi;
pub mod media;
pub mod model_files;

use std::ffi::{CStr, CString};
use std::path::PathBuf;
use std::ptr;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail, ensure};
pub use rummage_core::embed::{EmbedInput, Embedder, ImageData, Modalities};
use rummage_core::embed::{document_prompt, normalize, query_prompt};

/// Full size of an EmbeddingGemma 2 vector.
pub const FULL_DIM: usize = 768;
/// Matryoshka sizes the model was trained for.
pub const MRL_DIMS: [usize; 4] = [768, 512, 256, 128];

/// Weight precision of the GGUF files. The model card forbids float16; both of these are safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    Q8_0,
    Bf16,
}

impl std::str::FromStr for Precision {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "q8_0" | "q8" => Ok(Precision::Q8_0),
            "bf16" => Ok(Precision::Bf16),
            "f16" | "fp16" | "float16" => {
                bail!(
                    "float16 is not allowed: EmbeddingGemma 2 gives NaN or degraded vectors in float16"
                )
            }
            _ => bail!("unknown precision {s:?}, expected q8_0 or bf16"),
        }
    }
}

/// How to load and run the model.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Text model GGUF. `None` = download the file for `precision` into `model_dir`.
    pub model: Option<PathBuf>,
    /// Vision+audio encoder GGUF (mmproj). `None` = download it into `model_dir`.
    pub mmproj: Option<PathBuf>,
    /// Where downloaded model files live.
    pub model_dir: PathBuf,
    pub precision: Precision,
    /// Load the vision and audio encoders. Text-only engines skip the 555 MB mmproj.
    pub media: bool,
    /// Output vector size: 768, or a Matryoshka cut (512, 256, 128).
    pub dim: usize,
    /// Layers to put on the GPU; -1 = all of them.
    pub gpu_layers: i32,
    /// CPU threads; 0 = one per physical core.
    pub threads: i32,
    /// Longest input in tokens; every input must fit. Also the llama.cpp micro-batch size.
    pub max_tokens: u32,
    /// Target tokens per batch when packing several inputs together. Attention in a batch spans
    /// all of its tokens, so packing far more than one input's worth costs more than it saves.
    pub batch_tokens: u32,
    /// Most sequences in one batch.
    pub max_seqs: u32,
    /// Vision tokens per image (70..=1120; the model card default is 280).
    pub image_tokens: u32,
    /// Vision tokens per video frame (the official video processor uses 140).
    pub video_frame_tokens: u32,
    /// Flash attention. Off by default: llama.cpp runs flash attention with float16 K/V when
    /// there is no KV cache, and the model card forbids float16.
    pub flash_attn: bool,
    /// llama.cpp log level: 0 debug, 2 info, 3 warn (default), 4 error, 5 silent.
    /// The `RUMMAGE_LLAMA_LOG` environment variable overrides it.
    pub log_level: i32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            model: None,
            mmproj: None,
            model_dir: default_model_dir(),
            precision: Precision::Q8_0,
            media: true,
            dim: FULL_DIM,
            gpu_layers: -1,
            threads: 0,
            max_tokens: 4096,
            batch_tokens: 512,
            max_seqs: 64,
            image_tokens: 280,
            video_frame_tokens: 140,
            flash_attn: false,
            log_level: 3,
        }
    }
}

/// `$XDG_DATA_HOME/rummage/models`, or `~/.local/share/rummage/models`.
pub fn default_model_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("rummage").join("models")
}

/// A compute device llama.cpp found.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Device {
    pub name: String,
    pub description: String,
    pub gpu: bool,
    pub memory_total: usize,
    pub memory_free: usize,
}

/// What the engine runs on, for logs and `rummage doctor`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineInfo {
    pub backends: String,
    pub llama_cpp_commit: String,
    pub model: String,
    pub model_path: PathBuf,
    pub mmproj_path: Option<PathBuf>,
    pub precision: String,
    pub devices: Vec<Device>,
    pub dim: usize,
}

/// Lists the devices compiled into this build (CPU always, plus the enabled GPU backend).
pub fn devices() -> Vec<Device> {
    backend_init();
    let mut out = Vec::new();
    unsafe {
        for i in 0..ffi::ggml_backend_dev_count() {
            let dev = ffi::ggml_backend_dev_get(i);
            let (mut free, mut total) = (0usize, 0usize);
            ffi::ggml_backend_dev_memory(dev, &mut free, &mut total);
            out.push(Device {
                name: cstr(ffi::ggml_backend_dev_name(dev)),
                description: cstr(ffi::ggml_backend_dev_description(dev)),
                gpu: ffi::ggml_backend_dev_type(dev) != ffi::GGML_BACKEND_DEVICE_TYPE_CPU,
                memory_total: total,
                memory_free: free,
            });
        }
    }
    out
}

/// Physical CPU cores (hyper-threads share their core's vector units, so llama.cpp gains
/// nothing from them).
pub fn physical_cores() -> usize {
    let logical = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut cores = std::collections::BTreeSet::new();
    for cpu in 0..logical {
        let topo = format!("/sys/devices/system/cpu/cpu{cpu}/topology");
        let read = |f: &str| std::fs::read_to_string(format!("{topo}/{f}")).ok();
        match (read("physical_package_id"), read("core_id")) {
            (Some(p), Some(c)) => {
                cores.insert((p.trim().to_string(), c.trim().to_string()));
            }
            _ => return logical,
        }
    }
    cores.len().clamp(1, logical)
}

fn backend_init() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        // The model card forbids float16. Vulkan's vision/audio kernels use it by default
        // (half-precision shaders and accumulators); turn that off unless asked for with
        // RUMMAGE_VULKAN_F16=1. Measured on gfx1151 it costs no speed.
        if cfg!(feature = "vulkan")
            && std::env::var_os("GGML_VK_DISABLE_F16").is_none()
            && std::env::var_os("RUMMAGE_VULKAN_F16").is_none_or(|v| v != "1")
        {
            // SAFETY: runs once, before ggml starts any threads or reads the environment.
            unsafe { std::env::set_var("GGML_VK_DISABLE_F16", "1") };
        }
        unsafe { ffi::rmg_backend_init() }
    });
}

fn cstr(p: *const std::ffi::c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// One piece of a sequence: text tokens, or the soft tokens of one media chunk.
enum Piece {
    Tokens(Vec<ffi::llama_token>),
    /// One image or audio chunk from libmtmd; `embd` gets its encoder output, row-major
    /// `n_tokens x n_embd_inp` floats.
    Media {
        chunk: *const ffi::mtmd_input_chunk,
        n_tokens: usize,
        embd: Vec<f32>,
    },
}

struct Seq {
    pieces: Vec<Piece>,
    n_tokens: usize,
    /// Owns the libmtmd chunks the media pieces point into.
    _chunks: Option<Chunks>,
}

struct Inner {
    model: *mut ffi::llama_model,
    ctx: *mut ffi::llama_context,
    mtmd: *mut ffi::mtmd_context,
    batch: *mut ffi::llama_batch_ext,
    vocab: *const ffi::llama_vocab,
    bos: ffi::llama_token,
    eos: ffi::llama_token,
    n_embd_inp: usize,
    n_embd_out: usize,
    max_tokens: usize,
    max_seqs: usize,
    marker: String,
}

// The raw pointers are only used while holding the engine's mutex.
unsafe impl Send for Inner {}

impl Drop for Inner {
    fn drop(&mut self) {
        unsafe {
            if !self.batch.is_null() {
                ffi::llama_batch_ext_free(self.batch);
            }
            if !self.mtmd.is_null() {
                ffi::mtmd_free(self.mtmd);
            }
            if !self.ctx.is_null() {
                ffi::llama_free(self.ctx);
            }
            if !self.model.is_null() {
                ffi::llama_model_free(self.model);
            }
        }
    }
}

/// The EmbeddingGemma 2 engine. Thread-safe; calls are serialised internally.
pub struct Engine {
    inner: Mutex<Inner>,
    cfg: EngineConfig,
    info: EngineInfo,
    modalities: Modalities,
}

impl Engine {
    pub fn new(cfg: EngineConfig) -> Result<Self> {
        ensure!(
            MRL_DIMS.contains(&cfg.dim),
            "dim must be one of {MRL_DIMS:?}, got {}",
            cfg.dim
        );
        ensure!(
            (70..=1120).contains(&cfg.image_tokens),
            "image_tokens must be 70..=1120"
        );
        ensure!(
            (70..=1120).contains(&cfg.video_frame_tokens),
            "video_frame_tokens must be 70..=1120"
        );
        ensure!(cfg.max_tokens >= 512, "max_tokens must be at least 512");
        backend_init();
        // RUMMAGE_LLAMA_LOG overrides the log level for debugging (0 = everything).
        let log_level = std::env::var("RUMMAGE_LLAMA_LOG")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(cfg.log_level);
        unsafe { ffi::rmg_log_set(log_level) };

        let (text_file, mmproj_file) = model_files::files(cfg.precision);
        let model_path = match &cfg.model {
            Some(p) => p.clone(),
            None => model_files::ensure(&cfg.model_dir, &text_file)?,
        };
        let mmproj_path = if cfg.media {
            Some(match &cfg.mmproj {
                Some(p) => p.clone(),
                None => model_files::ensure(&cfg.model_dir, &mmproj_file)?,
            })
        } else {
            None
        };

        let c_model = CString::new(model_path.to_string_lossy().as_bytes())?;
        let model = unsafe { ffi::rmg_model_load(c_model.as_ptr(), cfg.gpu_layers) };
        if model.is_null() {
            bail!("llama.cpp could not load {}", model_path.display());
        }
        let mut inner = Inner {
            model,
            ctx: ptr::null_mut(),
            mtmd: ptr::null_mut(),
            batch: ptr::null_mut(),
            vocab: unsafe { ffi::llama_model_get_vocab(model) },
            bos: 0,
            eos: 0,
            n_embd_inp: unsafe { ffi::llama_model_n_embd_inp(model) } as usize,
            n_embd_out: unsafe { ffi::llama_model_n_embd_out(model) } as usize,
            max_tokens: cfg.max_tokens as usize,
            max_seqs: cfg.max_seqs as usize,
            marker: cstr(unsafe { ffi::mtmd_default_marker() }),
        };
        inner.bos = unsafe { ffi::llama_vocab_bos(inner.vocab) };
        inner.eos = unsafe { ffi::llama_vocab_eos(inner.vocab) };
        ensure!(
            inner.n_embd_out == FULL_DIM,
            "{} is not EmbeddingGemma 2: output size {} != {FULL_DIM}",
            model_path.display(),
            inner.n_embd_out
        );
        let model_desc = {
            let mut buf = vec![0u8; 256];
            let n = unsafe { ffi::llama_model_desc(model, buf.as_mut_ptr().cast(), buf.len()) };
            String::from_utf8_lossy(&buf[..n.max(0) as usize]).into_owned()
        };

        let flash = if cfg.flash_attn { 1 } else { 0 };
        let threads = if cfg.threads > 0 {
            cfg.threads
        } else {
            physical_cores() as i32
        };
        inner.ctx = unsafe {
            ffi::rmg_context_new(
                model,
                cfg.max_tokens,
                cfg.max_seqs,
                threads,
                flash,
                cfg.gpu_layers != 0,
            )
        };
        if inner.ctx.is_null() {
            bail!(
                "llama.cpp could not create a context (max_tokens {})",
                cfg.max_tokens
            );
        }
        ensure!(
            unsafe { ffi::llama_pooling_type(inner.ctx) } == ffi::LLAMA_POOLING_TYPE_MEAN,
            "context is not mean-pooled"
        );
        inner.max_seqs = unsafe { ffi::llama_n_seq_max(inner.ctx) } as usize;
        inner.batch = unsafe { ffi::llama_batch_ext_init(inner.ctx) };
        ensure!(!inner.batch.is_null(), "llama_batch_ext_init failed");

        let mut modalities = Modalities {
            text: true,
            image: false,
            audio: false,
            video: false,
        };
        if let Some(mmproj) = &mmproj_path {
            let c_mmproj = CString::new(mmproj.to_string_lossy().as_bytes())?;
            let max_img = cfg.image_tokens.max(cfg.video_frame_tokens) as i32;
            inner.mtmd = unsafe {
                ffi::rmg_mtmd_init(
                    c_mmproj.as_ptr(),
                    model,
                    cfg.gpu_layers != 0,
                    threads,
                    // The engine resizes images itself (see media::prepare_image); a minimum of
                    // one token keeps llama.cpp from upscaling them again.
                    1,
                    max_img,
                    flash,
                    // Soft tokens per vision batch: about 3 images or 7 video frames. Bigger
                    // batches are no faster (the encoder is compute-bound) and each image adds
                    // ~300 MB of attention scores.
                    1024,
                    false,
                )
            };
            if inner.mtmd.is_null() {
                bail!("libmtmd could not load {}", mmproj.display());
            }
            let vision = unsafe { ffi::mtmd_support_vision(inner.mtmd) };
            let audio = unsafe { ffi::mtmd_support_audio(inner.mtmd) };
            if audio {
                let rate = unsafe { ffi::mtmd_get_audio_sample_rate(inner.mtmd) };
                ensure!(
                    rate as usize == media::AUDIO_RATE,
                    "unexpected audio sample rate {rate}"
                );
            }
            modalities = Modalities {
                text: true,
                image: vision,
                audio,
                video: vision,
            };
        }

        let info = EngineInfo {
            backends: env!("RUMMAGE_EMBED_BACKENDS").to_string(),
            llama_cpp_commit: env!("RUMMAGE_LLAMA_CPP_COMMIT").to_string(),
            model: model_desc,
            model_path,
            mmproj_path,
            precision: format!("{:?}", cfg.precision),
            devices: devices(),
            dim: cfg.dim,
        };
        Ok(Self {
            inner: Mutex::new(inner),
            cfg,
            info,
            modalities,
        })
    }

    pub fn info(&self) -> &EngineInfo {
        &self.info
    }

    pub fn config(&self) -> &EngineConfig {
        &self.cfg
    }

    /// Number of model tokens an input turns into (including `<bos>`/`<eos>` and media markers).
    pub fn count_tokens(&self, input: &EmbedInput) -> Result<usize> {
        let prepared = self.prepare(input)?;
        let inner = self.lock()?;
        Ok(build_seq(&inner, prepared)?.n_tokens)
    }

    /// Embeds inputs and returns full 768-d unit vectors (before any Matryoshka cut).
    pub fn embed_full(&self, inputs: &[EmbedInput]) -> Result<Vec<Vec<f32>>> {
        for input in inputs {
            ensure!(
                self.modalities.supports(input),
                "this engine does not accept {}",
                kind(input)
            );
        }
        let t0 = Instant::now();
        let prepared = self.prepare_all(inputs)?;
        let t_prep = t0.elapsed();

        let inner = self.lock()?;
        let t1 = Instant::now();
        let mut seqs = Vec::with_capacity(prepared.len());
        for (input, p) in inputs.iter().zip(prepared) {
            let seq = build_seq(&inner, p)?;
            ensure!(
                seq.n_tokens <= inner.max_tokens,
                "{} input needs {} tokens, more than max_tokens ({})",
                kind(input),
                seq.n_tokens,
                inner.max_tokens
            );
            seqs.push(seq);
        }
        encode_media(&inner, &mut seqs)?;
        let t_media = t1.elapsed();

        let t2 = Instant::now();
        let mut out = Vec::with_capacity(seqs.len());
        let mut start = 0;
        let budget = self.cfg.batch_tokens.min(self.cfg.max_tokens) as usize;
        while start < seqs.len() {
            // Greedy packing of whole sequences. Attention in a batch is computed across all of
            // its tokens (masked between sequences), so batches stay near `batch_tokens`; one
            // longer sequence still goes alone, up to `max_tokens`.
            let mut end = start + 1;
            let mut tokens = seqs[start].n_tokens;
            while end < seqs.len()
                && end - start < inner.max_seqs
                && tokens + seqs[end].n_tokens <= budget
            {
                tokens += seqs[end].n_tokens;
                end += 1;
            }
            out.extend(decode(&inner, &seqs[start..end])?);
            start = end;
        }
        if timing_enabled() {
            eprintln!(
                "rummage-embed: {} inputs: prepare {:.1} ms, tokenize+encode media {:.1} ms, text model {:.1} ms",
                inputs.len(),
                t_prep.as_secs_f64() * 1e3,
                t_media.as_secs_f64() * 1e3,
                t2.elapsed().as_secs_f64() * 1e3
            );
        }
        Ok(out)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| anyhow!("engine mutex poisoned"))
    }

    /// Decodes and resizes images on all cores, before taking the engine lock.
    fn prepare_all(&self, inputs: &[EmbedInput]) -> Result<Vec<Prepared>> {
        let heavy = inputs
            .iter()
            .filter(|i| matches!(i, EmbedInput::Image(_) | EmbedInput::VideoFrames(_)))
            .count();
        if heavy < 2 {
            return inputs.iter().map(|i| self.prepare(i)).collect();
        }
        let n = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(inputs.len());
        let per = inputs.len().div_ceil(n);
        std::thread::scope(|s| {
            let handles: Vec<_> = inputs
                .chunks(per)
                .map(|chunk| {
                    s.spawn(move || {
                        chunk
                            .iter()
                            .map(|i| self.prepare(i))
                            .collect::<Result<Vec<_>>>()
                    })
                })
                .collect();
            let mut out = Vec::with_capacity(inputs.len());
            for h in handles {
                out.extend(
                    h.join()
                        .map_err(|_| anyhow!("image preparation panicked"))??,
                );
            }
            Ok(out)
        })
    }

    fn prepare(&self, input: &EmbedInput) -> Result<Prepared> {
        Ok(match input {
            EmbedInput::Query(q) => Prepared::Text(query_prompt(q)),
            EmbedInput::Document { title, text } => {
                Prepared::Text(document_prompt(title.as_deref(), text))
            }
            EmbedInput::Image(img) => {
                Prepared::Images(vec![media::prepare_image(img, self.cfg.image_tokens)?])
            }
            EmbedInput::VideoFrames(frames) => {
                ensure!(!frames.is_empty(), "video input has no frames");
                Prepared::Images(
                    frames
                        .iter()
                        .map(|f| media::prepare_image(f, self.cfg.video_frame_tokens))
                        .collect::<Result<_>>()?,
                )
            }
            EmbedInput::Audio { samples_16k_mono } => {
                ensure!(!samples_16k_mono.is_empty(), "audio input is empty");
                let mut parts = Vec::new();
                for (i, part) in samples_16k_mono
                    .chunks(media::AUDIO_PART_SAMPLES)
                    .enumerate()
                {
                    // A tail shorter than one encoder step adds no tokens; drop it.
                    if i > 0 && part.len() < media::AUDIO_RATE / 10 {
                        break;
                    }
                    parts.push(part.to_vec());
                }
                Prepared::Audio(parts)
            }
        })
    }
}

/// An input after the CPU-side work that needs no model: prompt text, resized images, audio parts.
enum Prepared {
    Text(String),
    Images(Vec<image::RgbImage>),
    Audio(Vec<Vec<f32>>),
}

/// Tokenises one input. Media chunks are not encoded yet (see [`encode_media`]).
fn build_seq(inner: &Inner, input: Prepared) -> Result<Seq> {
    let (pieces, chunks) = match input {
        Prepared::Text(prompt) => (vec![Piece::Tokens(tokenize_text(inner, &prompt)?)], None),
        Prepared::Images(images) => {
            let bitmaps = images
                .iter()
                .map(Bitmap::image)
                .collect::<Result<Vec<_>>>()?;
            media_pieces(inner, &bitmaps)?
        }
        Prepared::Audio(parts) => {
            let bitmaps = parts
                .iter()
                .map(|p| Bitmap::audio(p))
                .collect::<Result<Vec<_>>>()?;
            media_pieces(inner, &bitmaps)?
        }
    };
    let n_tokens = pieces
        .iter()
        .map(|p| match p {
            Piece::Tokens(t) => t.len(),
            Piece::Media { n_tokens, .. } => *n_tokens,
        })
        .sum::<usize>();
    Ok(Seq {
        pieces,
        n_tokens,
        _chunks: chunks,
    })
}

struct MtmdBatch(*mut ffi::mtmd_batch);

impl Drop for MtmdBatch {
    fn drop(&mut self) {
        unsafe { ffi::mtmd_batch_free(self.0) }
    }
}

/// Runs the vision/audio encoders on every media chunk of `seqs`. Consecutive chunks that
/// libmtmd can batch (images of the same size, e.g. the frames of one video) go through the
/// encoder together.
fn encode_media(inner: &Inner, seqs: &mut [Seq]) -> Result<()> {
    let mut slots: Vec<(*const ffi::mtmd_input_chunk, usize, &mut Vec<f32>)> = seqs
        .iter_mut()
        .flat_map(|s| s.pieces.iter_mut())
        .filter_map(|p| match p {
            Piece::Media {
                chunk,
                n_tokens,
                embd,
            } => Some((*chunk, *n_tokens, embd)),
            Piece::Tokens(_) => None,
        })
        .collect();
    let mut i = 0;
    while i < slots.len() {
        let batch = MtmdBatch(unsafe { ffi::mtmd_batch_init(inner.mtmd) });
        ensure!(!batch.0.is_null(), "mtmd_batch_init failed");
        let mut j = i;
        while j < slots.len() {
            match unsafe { ffi::mtmd_batch_add_chunk(batch.0, slots[j].0) } {
                0 => j += 1,
                // 2: batch is full, 3: this chunk cannot join the batch.
                2 | 3 if j > i => break,
                rc => bail!("mtmd_batch_add_chunk failed ({rc})"),
            }
        }
        let rc = unsafe { ffi::mtmd_batch_encode(batch.0) };
        ensure!(rc == 0, "mtmd_batch_encode failed ({rc})");
        for (chunk, n_tokens, embd) in &mut slots[i..j] {
            let out = unsafe { ffi::mtmd_batch_get_output_embd(batch.0, *chunk) };
            ensure!(!out.is_null(), "no media embeddings");
            **embd =
                unsafe { std::slice::from_raw_parts(out, *n_tokens * inner.n_embd_inp) }.to_vec();
        }
        i = j;
    }
    Ok(())
}

fn timing_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RUMMAGE_EMBED_TIMING").is_some_and(|v| v != "0"))
}

fn kind(input: &EmbedInput) -> &'static str {
    match input {
        EmbedInput::Query(_) => "query",
        EmbedInput::Document { .. } => "document",
        EmbedInput::Image(_) => "image",
        EmbedInput::Audio { .. } => "audio",
        EmbedInput::VideoFrames(_) => "video",
    }
}

/// Tokenises a prompt as `<bos> text <eos>`, cutting overlong text but keeping `<eos>`.
fn tokenize_text(inner: &Inner, text: &str) -> Result<Vec<ffi::llama_token>> {
    let mut tokens = tokenize(inner, text, true, false)?;
    if tokens.first() != Some(&inner.bos) {
        tokens.insert(0, inner.bos);
    }
    if tokens.last() != Some(&inner.eos) {
        tokens.push(inner.eos);
    }
    if tokens.len() > inner.max_tokens {
        tokens.truncate(inner.max_tokens - 1);
        tokens.push(inner.eos);
    }
    Ok(tokens)
}

fn tokenize(
    inner: &Inner,
    text: &str,
    add_special: bool,
    parse_special: bool,
) -> Result<Vec<ffi::llama_token>> {
    let len = i32::try_from(text.len()).context("text too long")?;
    let mut tokens = vec![0; text.len() + 8];
    let mut n = unsafe {
        ffi::llama_tokenize(
            inner.vocab,
            text.as_ptr().cast(),
            len,
            tokens.as_mut_ptr(),
            tokens.len() as i32,
            add_special,
            parse_special,
        )
    };
    if n < 0 {
        tokens.resize((-n) as usize, 0);
        n = unsafe {
            ffi::llama_tokenize(
                inner.vocab,
                text.as_ptr().cast(),
                len,
                tokens.as_mut_ptr(),
                tokens.len() as i32,
                add_special,
                parse_special,
            )
        };
    }
    ensure!(n >= 0, "tokenization failed");
    tokens.truncate(n as usize);
    Ok(tokens)
}

/// An owned libmtmd bitmap (an RGB image or 16 kHz PCM samples).
struct Bitmap(*mut ffi::mtmd_bitmap);

impl Bitmap {
    fn image(rgb: &image::RgbImage) -> Result<Self> {
        let p = unsafe { ffi::mtmd_bitmap_init(rgb.width(), rgb.height(), rgb.as_raw().as_ptr()) };
        ensure!(!p.is_null(), "mtmd_bitmap_init failed");
        Ok(Self(p))
    }
    fn audio(samples: &[f32]) -> Result<Self> {
        let p = unsafe { ffi::mtmd_bitmap_init_from_audio(samples.len(), samples.as_ptr()) };
        ensure!(!p.is_null(), "mtmd_bitmap_init_from_audio failed");
        Ok(Self(p))
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe { ffi::mtmd_bitmap_free(self.0) }
    }
}

struct Chunks(*mut ffi::mtmd_input_chunks);

impl Drop for Chunks {
    fn drop(&mut self) {
        unsafe { ffi::mtmd_input_chunks_free(self.0) }
    }
}

/// Runs libmtmd's tokenizer on one marker per bitmap and returns `<bos>`, the chunks, and
/// `<eos>`. libmtmd adds the model's begin/end markers (`<|image>`/`<image|>`,
/// `<|audio>`/`<audio|>`) around each media chunk.
fn media_pieces(inner: &Inner, bitmaps: &[Bitmap]) -> Result<(Vec<Piece>, Option<Chunks>)> {
    ensure!(!inner.mtmd.is_null(), "media input needs the mmproj model");
    let prompt = CString::new(inner.marker.repeat(bitmaps.len()))?;
    let text = ffi::mtmd_input_text {
        text: prompt.as_ptr(),
        text_len: prompt.as_bytes().len(),
        add_special: false,
        parse_special: true,
    };
    let ptrs: Vec<*const ffi::mtmd_bitmap> = bitmaps.iter().map(|b| b.0 as *const _).collect();
    let chunks = Chunks(unsafe { ffi::mtmd_input_chunks_init() });
    let rc = unsafe { ffi::mtmd_tokenize(inner.mtmd, chunks.0, &text, ptrs.as_ptr(), ptrs.len()) };
    ensure!(rc == 0, "mtmd_tokenize failed ({rc})");

    let mut pieces = vec![Piece::Tokens(vec![inner.bos])];
    let n = unsafe { ffi::mtmd_input_chunks_size(chunks.0) };
    for i in 0..n {
        let chunk = unsafe { ffi::mtmd_input_chunks_get(chunks.0, i) };
        if unsafe { ffi::mtmd_input_chunk_get_type(chunk) } == ffi::MTMD_INPUT_CHUNK_TYPE_TEXT {
            let mut len = 0usize;
            let p = unsafe { ffi::mtmd_input_chunk_get_tokens_text(chunk, &mut len) };
            let toks = unsafe { std::slice::from_raw_parts(p, len) }.to_vec();
            match pieces.last_mut() {
                Some(Piece::Tokens(t)) => t.extend(toks),
                _ => pieces.push(Piece::Tokens(toks)),
            }
        } else {
            pieces.push(Piece::Media {
                chunk,
                n_tokens: unsafe { ffi::mtmd_input_chunk_get_n_tokens(chunk) },
                embd: Vec::new(),
            });
        }
    }
    match pieces.last_mut() {
        Some(Piece::Tokens(t)) => t.push(inner.eos),
        _ => pieces.push(Piece::Tokens(vec![inner.eos])),
    }
    Ok((pieces, Some(chunks)))
}

/// Runs the text model on whole sequences (one batch) and returns their pooled unit vectors.
fn decode(inner: &Inner, seqs: &[Seq]) -> Result<Vec<Vec<f32>>> {
    let b = inner.batch;
    unsafe { ffi::llama_batch_ext_clear(b) };
    for (s, seq) in seqs.iter().enumerate() {
        let sid = s as ffi::llama_seq_id;
        let mut pos: ffi::llama_pos = 0;
        for piece in &seq.pieces {
            match piece {
                Piece::Tokens(toks) => {
                    for &t in toks {
                        let idx = unsafe { ffi::llama_batch_ext_add_token(b, sid, t) };
                        add_common(b, idx, pos)?;
                        pos += 1;
                    }
                }
                Piece::Media { embd, .. } => {
                    for row in embd.chunks_exact(inner.n_embd_inp) {
                        let e = ffi::llama_embd {
                            data: row.as_ptr(),
                            n_rows: 1,
                            n_embd: inner.n_embd_inp,
                        };
                        let idx = unsafe { ffi::llama_batch_ext_add_embd(b, sid, e) };
                        add_common(b, idx, pos)?;
                        pos += 1;
                    }
                }
            }
        }
    }
    let rc = unsafe { ffi::llama_process(inner.ctx, ffi::LLAMA_PROCESS_TYPE_DECODE, b) };
    ensure!(rc == 0, "llama_process failed ({rc})");
    unsafe { ffi::llama_synchronize(inner.ctx) };
    let mut out = Vec::with_capacity(seqs.len());
    for s in 0..seqs.len() {
        let p = unsafe { ffi::llama_get_embeddings_seq(inner.ctx, s as ffi::llama_seq_id) };
        ensure!(!p.is_null(), "no pooled embedding for sequence {s}");
        let mut v = unsafe { std::slice::from_raw_parts(p, inner.n_embd_out) }.to_vec();
        ensure!(v.iter().all(|x| x.is_finite()), "model returned NaN/inf");
        normalize(&mut v);
        out.push(v);
    }
    Ok(out)
}

fn add_common(b: *mut ffi::llama_batch_ext, idx: i32, pos: ffi::llama_pos) -> Result<()> {
    ensure!(idx >= 0, "llama batch add failed ({idx})");
    ensure!(
        unsafe { ffi::llama_batch_ext_set_pos(b, idx, &pos) },
        "set_pos failed"
    );
    ensure!(
        unsafe { ffi::llama_batch_ext_set_output_embd(b, idx, true) },
        "set_output failed"
    );
    Ok(())
}

/// Cuts a unit vector to `dim` (Matryoshka) and renormalises it.
pub fn truncate(v: &[f32], dim: usize) -> Vec<f32> {
    let mut out = v[..dim.min(v.len())].to_vec();
    normalize(&mut out);
    out
}

impl Embedder for Engine {
    fn dim(&self) -> usize {
        self.cfg.dim
    }

    fn embed(&self, inputs: &[EmbedInput]) -> Result<Vec<Vec<f32>>> {
        let full = self.embed_full(inputs)?;
        if self.cfg.dim == FULL_DIM {
            return Ok(full);
        }
        Ok(full.iter().map(|v| truncate(v, self.cfg.dim)).collect())
    }

    fn modalities(&self) -> Modalities {
        self.modalities
    }

    fn name(&self) -> String {
        let gpu: Vec<_> = self
            .info
            .devices
            .iter()
            .filter(|d| d.gpu)
            .map(|d| d.description.as_str())
            .collect();
        format!(
            "EmbeddingGemma 2 ({:?}, {} dims) on llama.cpp {} [{}{}]",
            self.cfg.precision,
            self.cfg.dim,
            self.info.llama_cpp_commit,
            self.info.backends,
            if gpu.is_empty() {
                String::new()
            } else {
                format!(": {}", gpu.join(", "))
            }
        )
    }
}

/// Cosine similarity of two vectors of equal length.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut ab, mut aa, mut bb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        ab += (*x as f64) * (*y as f64);
        aa += (*x as f64) * (*x as f64);
        bb += (*y as f64) * (*y as f64);
    }
    (ab / (aa.sqrt() * bb.sqrt())) as f32
}
