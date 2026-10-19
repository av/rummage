//! Throughput and latency of the EmbeddingGemma 2 engine on one backend.
//!
//! Inputs are synthetic but sized like real work: 512-token text chunks, 280-token images,
//! 30 s audio windows, 140-token video frames in 10-frame (10 s) windows, and short queries.

use std::time::Instant;

use anyhow::{Result, bail};
use clap::Parser;
use rummage_embed::{EmbedInput, Embedder, Engine, EngineConfig, ImageData, Precision};

#[derive(Parser, Debug)]
#[command(about = "Benchmark the EmbeddingGemma 2 engine")]
struct Args {
    /// Backend to run on: cpu, or the GPU backend this binary was built with (vulkan, rocm, cuda).
    #[arg(long, default_value = "auto")]
    backend: String,
    /// Weights: q8_0 or bf16.
    #[arg(long, default_value = "q8_0")]
    precision: Precision,
    /// Directory with (or for) the GGUF files.
    #[arg(long, env = "RUMMAGE_EMBED_MODEL_DIR")]
    model_dir: Option<std::path::PathBuf>,
    /// Tokens per batch (and longest input).
    #[arg(long, default_value_t = 4096)]
    max_tokens: u32,
    /// Target tokens per batch when packing several inputs.
    #[arg(long, default_value_t = 512)]
    batch_tokens: u32,
    /// CPU threads (0 = one per physical core).
    #[arg(long, default_value_t = 0)]
    threads: i32,
    /// Enable flash attention (runs K/V in float16; off by default because of the model card).
    #[arg(long)]
    flash_attn: bool,
    /// Number of 512-token text chunks.
    #[arg(long, default_value_t = 256)]
    texts: usize,
    /// Text chunks per embed() call.
    #[arg(long, default_value_t = 32)]
    text_batch: usize,
    /// Number of images.
    #[arg(long, default_value_t = 48)]
    images: usize,
    /// Number of 30 s audio windows.
    #[arg(long, default_value_t = 12)]
    audio: usize,
    /// Number of 10-frame video windows.
    #[arg(long, default_value_t = 6)]
    videos: usize,
    /// Number of single queries for the latency numbers.
    #[arg(long, default_value_t = 200)]
    queries: usize,
    /// Print the result as JSON too.
    #[arg(long)]
    json: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let built = env!("RUMMAGE_EMBED_BACKENDS");
    let gpu_backend = built.split(',').find(|b| *b != "cpu");
    let backend = match args.backend.as_str() {
        "auto" => gpu_backend.unwrap_or("cpu").to_string(),
        "cpu" => "cpu".to_string(),
        b if Some(b) == gpu_backend => b.to_string(),
        b => bail!(
            "this binary was built with [{built}]; it cannot run on {b}. Rebuild with --features {b}."
        ),
    };
    let mut cfg = EngineConfig {
        precision: args.precision,
        max_tokens: args.max_tokens,
        batch_tokens: args.batch_tokens,
        threads: args.threads,
        flash_attn: args.flash_attn,
        gpu_layers: if backend == "cpu" { 0 } else { -1 },
        ..Default::default()
    };
    if let Some(dir) = &args.model_dir {
        cfg.model_dir = dir.clone();
    }
    let gpu_free_before = gpu_free();
    let t = Instant::now();
    let engine = Engine::new(cfg)?;
    let load = t.elapsed();
    eprintln!(
        "engine: {} (loaded in {:.2}s)",
        engine.name(),
        load.as_secs_f64()
    );

    let mut gpu_low = gpu_free_before;
    let mut timed = |items: &[EmbedInput], per_call: usize| -> Result<f64> {
        // The first call of each shape compiles kernels and sizes buffers; keep it out.
        engine.embed(&items[..per_call.min(items.len())])?;
        let t = Instant::now();
        for chunk in items.chunks(per_call) {
            engine.embed(chunk)?;
        }
        let secs = t.elapsed().as_secs_f64();
        if let (Some(low), Some(now)) = (gpu_low, gpu_free()) {
            gpu_low = Some(low.min(now));
        }
        Ok(secs)
    };

    // Text: 512-token chunks.
    let doc = text_of_tokens(&engine, 512)?;
    let docs: Vec<EmbedInput> = (0..args.texts)
        .map(|i| EmbedInput::Document {
            title: None,
            text: format!("{i} {doc}"),
        })
        .collect();
    let text_s = timed(&docs, args.text_batch)?;

    // Images: 1600x1200 photos, resized to the 280-token budget.
    let imgs: Vec<EmbedInput> = (0..args.images)
        .map(|i| EmbedInput::Image(synthetic_image(1600, 1200, i as u64)))
        .collect();
    let image_s = timed(&imgs, 8)?;

    // Audio: 30 s windows.
    let auds: Vec<EmbedInput> = (0..args.audio)
        .map(|i| EmbedInput::Audio {
            samples_16k_mono: synthetic_audio(16_000 * 30, i as u64),
        })
        .collect();
    let audio_s = timed(&auds, 4)?;

    // Video: 10-frame (10 s at 1 fps) windows of 1280x720 frames, 140 tokens per frame.
    let vids: Vec<EmbedInput> = (0..args.videos)
        .map(|v| {
            EmbedInput::VideoFrames(
                (0..10)
                    .map(|f| synthetic_image(1280, 720, (v * 100 + f) as u64))
                    .collect(),
            )
        })
        .collect();
    let video_s = timed(&vids, 1)?;

    // Query latency, one query per call.
    let mut lat = Vec::with_capacity(args.queries);
    for i in 0..args.queries {
        let q = EmbedInput::Query(format!("photos of my cat sleeping on the porch {i}"));
        let t = Instant::now();
        engine.embed(std::slice::from_ref(&q))?;
        lat.push(t.elapsed());
    }
    lat.sort();
    let pct = |p: f64| lat[((lat.len() as f64 - 1.0) * p).round() as usize].as_secs_f64() * 1000.0;

    let gpu_used = match (gpu_free_before, gpu_low) {
        (Some(before), Some(low)) => Some(before.saturating_sub(low)),
        _ => None,
    };
    let result = serde_json::json!({
        "backend": backend,
        "engine": engine.name(),
        "devices": engine.info().devices,
        "precision": format!("{:?}", args.precision),
        "max_tokens": args.max_tokens,
        "batch_tokens": args.batch_tokens,
        "flash_attn": args.flash_attn,
        "load_seconds": load.as_secs_f64(),
        "text_tokens_per_chunk": engine.count_tokens(&docs[0])?,
        "texts_per_s": args.texts as f64 / text_s,
        "images_per_s": args.images as f64 / image_s,
        "audio_seconds_per_s": (args.audio * 30) as f64 / audio_s,
        "video_frames_per_s": (args.videos * 10) as f64 / video_s,
        "query_p50_ms": pct(0.5),
        "query_p95_ms": pct(0.95),
        "peak_rss_mb": peak_rss_mb(),
        "gpu_mem_used_mb": gpu_used.map(|b| b as f64 / 1e6),
    });
    let f = |k: &str| result[k].as_f64().unwrap_or(f64::NAN);
    println!(
        "backend             {backend} ({:?}, batch_tokens {})",
        args.precision, args.batch_tokens
    );
    println!("texts/s (512 tok)   {:.1}", f("texts_per_s"));
    println!("images/s            {:.1}", f("images_per_s"));
    println!("audio seconds/s     {:.1}", f("audio_seconds_per_s"));
    println!("video frames/s      {:.1}", f("video_frames_per_s"));
    println!(
        "query p50 / p95 ms  {:.1} / {:.1}",
        f("query_p50_ms"),
        f("query_p95_ms")
    );
    println!("peak RSS MB         {:.0}", f("peak_rss_mb"));
    if gpu_used.is_some() {
        println!("GPU memory used MB  {:.0}", f("gpu_mem_used_mb"));
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

/// A document body that makes a document of at least `n` tokens (prefix and markers included).
fn text_of_tokens(engine: &Engine, n: usize) -> Result<String> {
    const WORDS: &str = "the river carried autumn leaves past the old mill while children counted boats \
        and a baker opened the shop window to let warm bread cool in the morning air near the station";
    let words: Vec<&str> = WORDS.split_whitespace().collect();
    let mut text = String::new();
    let mut i = 0;
    loop {
        let doc = EmbedInput::Document {
            title: None,
            text: format!("0 {text}"),
        };
        if engine.count_tokens(&doc)? >= n {
            return Ok(text);
        }
        text.push_str(words[i % words.len()]);
        text.push(' ');
        i += 1;
    }
}

fn synthetic_image(w: u32, h: u32, seed: u64) -> ImageData {
    let mut s = seed.wrapping_mul(0x9e3779b97f4a7c15) | 1;
    let mut pixels = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let n = (s & 31) as u32;
            pixels.push(((x * 255 / w) + n).min(255) as u8);
            pixels.push(((y * 255 / h) + n).min(255) as u8);
            pixels.push((((x + y) * 128 / (w + h)) + n) as u8);
        }
    }
    ImageData::Rgb8 {
        width: w,
        height: h,
        pixels,
    }
}

fn synthetic_audio(n: usize, seed: u64) -> Vec<f32> {
    let f = 220.0 + 40.0 * seed as f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / 16_000.0;
            0.15 * (2.0 * std::f32::consts::PI * f * t).sin() * (1.0 + (3.0 * t).sin())
        })
        .collect()
}

/// Peak resident memory of this process (VmHWM).
fn peak_rss_mb() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

fn gpu_free() -> Option<usize> {
    rummage_embed::devices()
        .into_iter()
        .find(|d| d.gpu)
        .map(|d| d.memory_free)
}
