//! Parity with the official model: cosine similarity against reference vectors made by
//! `tests/reference/make_reference.py` (sentence-transformers, float32).
//!
//! Needs the GGUF files: set `RUMMAGE_EMBED_MODEL` + `RUMMAGE_EMBED_MMPROJ`, or
//! `RUMMAGE_EMBED_MODEL_DIR` (files are downloaded there if missing). `RUMMAGE_EMBED_PRECISION`
//! picks q8_0 (default, cosine >= 0.98) or bf16 (cosine >= 0.995). The video item needs
//! `ffmpeg` on PATH. Set `RUMMAGE_SKIP_PARITY=1` to skip the test explicitly; it never skips
//! silently. `RUMMAGE_PARITY_OUT=file.json` writes the per-item numbers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use rummage_embed::{
    EmbedInput, Embedder, Engine, EngineConfig, ImageData, Precision, cosine, truncate,
};
use serde_json::Value;

fn reference_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference")
}

fn engine() -> (Engine, Precision) {
    let precision: Precision = std::env::var("RUMMAGE_EMBED_PRECISION")
        .unwrap_or_else(|_| "q8_0".into())
        .parse()
        .unwrap();
    let mut cfg = EngineConfig {
        precision,
        ..Default::default()
    };
    if let Some(dir) = std::env::var_os("RUMMAGE_EMBED_MODEL_DIR") {
        cfg.model_dir = dir.into();
    }
    cfg.model = std::env::var_os("RUMMAGE_EMBED_MODEL").map(PathBuf::from);
    cfg.mmproj = std::env::var_os("RUMMAGE_EMBED_MMPROJ").map(PathBuf::from);
    if let Ok(v) = std::env::var("RUMMAGE_EMBED_GPU_LAYERS") {
        cfg.gpu_layers = v.parse().unwrap();
    }
    cfg.flash_attn = std::env::var("RUMMAGE_EMBED_FLASH_ATTN").is_ok_and(|v| v == "1");
    (Engine::new(cfg).expect("loading the engine"), precision)
}

fn read_wav(path: &Path) -> Vec<f32> {
    let mut r = hound::WavReader::open(path).unwrap();
    let spec = r.spec();
    assert_eq!(
        (spec.channels, spec.sample_rate, spec.bits_per_sample),
        (1, 16000, 16)
    );
    // Same scaling as soundfile's float32 reads: int16 / 32768.
    r.samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect()
}

/// Decodes the given frames as RGB with the same ffmpeg command the reference script used.
fn decode_frames(path: &Path, indices: &[u64], width: u32, height: u32) -> Vec<ImageData> {
    let select: Vec<String> = indices.iter().map(|i| format!("eq(n\\,{i})")).collect();
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-vf",
            &format!("select='{}'", select.join("+")),
            "-fps_mode",
            "passthrough",
        ])
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("the video parity item needs ffmpeg on PATH");
    assert!(
        out.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let frame = (width * height * 3) as usize;
    assert_eq!(out.stdout.len(), frame * indices.len());
    out.stdout
        .chunks_exact(frame)
        .map(|px| ImageData::Rgb8 {
            width,
            height,
            pixels: px.to_vec(),
        })
        .collect()
}

fn input_for(item: &Value) -> EmbedInput {
    let fix = reference_dir().join("fixtures");
    let source = item["source"].as_str().unwrap();
    match item["kind"].as_str().unwrap() {
        "query" => EmbedInput::Query(
            source
                .strip_prefix("task: search result | query: ")
                .unwrap()
                .into(),
        ),
        "document" => {
            let rest = source.strip_prefix("title: ").unwrap();
            let (title, text) = rest.split_once(" | text: ").unwrap();
            EmbedInput::Document {
                title: (title != "none").then(|| title.to_string()),
                text: text.into(),
            }
        }
        "image" => EmbedInput::Image(ImageData::Encoded(std::fs::read(fix.join(source)).unwrap())),
        "audio" => EmbedInput::Audio {
            samples_16k_mono: read_wav(&fix.join(source)),
        },
        "video" => {
            let idx: Vec<u64> = item["frame_indices"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap())
                .collect();
            let (w, h) = (
                item["width"].as_u64().unwrap() as u32,
                item["height"].as_u64().unwrap() as u32,
            );
            EmbedInput::VideoFrames(decode_frames(&fix.join(source), &idx, w, h))
        }
        k => panic!("unknown kind {k}"),
    }
}

fn as_vec(v: &Value) -> Vec<f32> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap() as f32)
        .collect()
}

#[test]
fn parity_with_reference_vectors() {
    if std::env::var("RUMMAGE_SKIP_PARITY").is_ok_and(|v| v == "1") {
        eprintln!(
            "SKIPPED: RUMMAGE_SKIP_PARITY=1, parity with the reference model was not checked"
        );
        return;
    }
    let reference: Value = serde_json::from_str(
        &std::fs::read_to_string(reference_dir().join("reference.json")).unwrap(),
    )
    .unwrap();
    let items = reference["items"].as_array().unwrap();

    let t0 = Instant::now();
    let (engine, precision) = engine();
    eprintln!(
        "engine: {} (loaded in {:.1}s)",
        engine.name(),
        t0.elapsed().as_secs_f32()
    );
    let threshold = match precision {
        Precision::Q8_0 => 0.98,
        Precision::Bf16 => 0.995,
    };

    let inputs: Vec<EmbedInput> = items.iter().map(input_for).collect();

    // Token layout must match the official processor (<bos> ... <eos>, media markers).
    for (item, input) in items.iter().zip(&inputs) {
        if let Some(want) = item["n_tokens"].as_u64() {
            let got = engine.count_tokens(input).unwrap();
            assert_eq!(got as u64, want, "{}: token count", item["id"]);
        }
    }

    // Each item on its own, then all of them in one call (batched) to check batching changes nothing.
    let t1 = Instant::now();
    let single: Vec<Vec<f32>> = inputs
        .iter()
        .map(|i| engine.embed(std::slice::from_ref(i)).unwrap().remove(0))
        .collect();
    let single_secs = t1.elapsed().as_secs_f32();
    let batched = engine.embed(&inputs).unwrap();

    let mut results = Vec::new();
    let mut failures = Vec::new();
    eprintln!(
        "{:<16} {:>9} {:>9} {:>9}",
        "item", "cos", "cos@256", "batched"
    );
    let mut got = BTreeMap::new();
    for ((item, s), b) in items.iter().zip(&single).zip(&batched) {
        let id = item["id"].as_str().unwrap();
        let want = as_vec(&item["vector"]);
        assert_eq!(s.len(), 768);
        let cos = cosine(s, &want);
        let cos256 = cosine(&truncate(s, 256), &truncate(&want, 256));
        let cos_batch = cosine(s, b);
        eprintln!("{id:<16} {cos:>9.5} {cos256:>9.5} {cos_batch:>9.6}");
        if cos < threshold || cos256 < threshold {
            failures.push(format!(
                "{id}: cosine {cos:.5} (at 256 dims {cos256:.5}) < {threshold}"
            ));
        }
        if cos_batch < 0.999 {
            failures.push(format!(
                "{id}: batched and single results differ (cosine {cos_batch:.6})"
            ));
        }
        results.push(serde_json::json!({"id": id, "kind": item["kind"], "cosine": cos, "cosine_256": cos256, "cosine_batched": cos_batch}));
        got.insert(id.to_string(), s.clone());
    }

    // Retrieval sanity: each query is nearest its matching item in every group.
    for (q, groups) in reference["retrieval"].as_object().unwrap() {
        for (kind, want) in groups.as_object().unwrap() {
            let best = items
                .iter()
                .filter(|it| it["kind"] == kind.as_str())
                .map(|it| it["id"].as_str().unwrap())
                .max_by(|a, b| cosine(&got[q], &got[*a]).total_cmp(&cosine(&got[q], &got[*b])))
                .unwrap();
            eprintln!(
                "retrieval {q} -> {kind}: {best} (want {})",
                want.as_str().unwrap()
            );
            if best != want.as_str().unwrap() {
                failures.push(format!("retrieval: {q} is nearest {best}, not {want}"));
            }
        }
    }

    if let Some(path) = std::env::var_os("RUMMAGE_PARITY_OUT") {
        let out = serde_json::json!({
            "engine": engine.name(),
            "info": engine.info(),
            "precision": format!("{precision:?}"),
            "threshold": threshold,
            "seconds_one_by_one": single_secs,
            "items": results,
        });
        std::fs::write(path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
    }
    assert!(
        failures.is_empty(),
        "parity failures:\n{}",
        failures.join("\n")
    );
}
