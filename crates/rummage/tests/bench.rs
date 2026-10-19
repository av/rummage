//! Speed checks, run by hand on the target machine (they are `#[ignore]`d in CI):
//!
//! ```sh
//! cargo test --release -p rummage --test bench -- --ignored --nocapture
//! RUMMAGE_BENCH_DIR=~/Pictures cargo test --release -p rummage --test bench first_pass -- --ignored --nocapture
//! ```

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rummage::indexer;
use rummage_core::service::Service;
use rummage_core::store::{ChunkRow, FileContent, NewFile};
use rummage_core::types::*;
use rummage_core::{ConfigFile, FakeEmbedder};

fn service(dir: &Path, folders: &[&Path]) -> Arc<Service> {
    let config = dir.join("config.toml");
    let mut text = format!("data_dir = \"{}\"\n", dir.join("data").display());
    for f in folders {
        text.push_str(&format!("[[folders]]\npath = \"{}\"\n", f.display()));
    }
    std::fs::write(&config, text).unwrap();
    let cf = ConfigFile::load_with_env(&config, |_| None).unwrap();
    let svc = Arc::new(Service::open(cf, Arc::new(FakeEmbedder::new(768))).unwrap());
    svc.set_decoder(Arc::new(rummage::extract::upload::Decoder));
    svc
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
    }
}

fn cpu() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|c| {
            c.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "unknown CPU".into())
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// Search latency over 300k random 768-dim vectors (100k files x 3 chunks), with keyword text.
#[test]
#[ignore]
fn search_latency_300k() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path(), &[]);
    let store = svc.store();
    let words: Vec<String> = (0..5000).map(|i| format!("w{i:04}")).collect();
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let started = Instant::now();
    let (files, per_file, dim) = (100_000, 3, 768);
    for f in 0..files {
        let id = store
            .upsert_file(&NewFile {
                path: format!("/bench/{}/file{f}.txt", f % 100),
                folder: "/bench".into(),
                size: 1000,
                mtime: 1_700_000_000 + f as i64,
                kind: Kind::ALL[f % 5],
                ..Default::default()
            })
            .unwrap();
        let chunks: Vec<(ChunkRow, Vec<f32>)> = (0..per_file)
            .map(|_| {
                let text: Vec<&str> = (0..40)
                    .map(|_| words[(rng.next() % words.len() as u64) as usize].as_str())
                    .collect();
                let mut v: Vec<f32> = (0..dim).map(|_| rng.unit()).collect();
                rummage_core::embed::normalize(&mut v);
                (
                    ChunkRow {
                        kind: SegmentKind::Text,
                        text: Some(text.join(" ")),
                        ..Default::default()
                    },
                    v,
                )
            })
            .collect();
        svc.store_file(
            id,
            &FileContent {
                size: 1000,
                mtime: 1_700_000_000 + f as i64,
                status: "indexed".into(),
                ..Default::default()
            },
            chunks,
        )
        .unwrap();
    }
    eprintln!(
        "built {} vectors in {:.1} s",
        files * per_file,
        started.elapsed().as_secs_f64()
    );

    let run = |mode: SearchMode, filters: Filters, n: usize| -> Vec<f64> {
        let mut rng = Rng(42);
        let mut times: Vec<f64> = (0..n)
            .map(|_| {
                let q = format!(
                    "{} {}",
                    words[(rng.next() % 5000) as usize],
                    words[(rng.next() % 5000) as usize]
                );
                let t = Instant::now();
                let r = svc
                    .search(
                        SearchRequest {
                            query: Some(q),
                            mode,
                            filters: filters.clone(),
                            limit: Some(50),
                            ..Default::default()
                        },
                        "",
                    )
                    .unwrap();
                assert!(!r.hits.is_empty());
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(f64::total_cmp);
        times
    };
    let t = Instant::now();
    run(SearchMode::Vector, Filters::default(), 1);
    eprintln!(
        "first search (loads the vectors): {:.0} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    eprintln!("CPU: {}", cpu());
    let mut p50s = vec![];
    for (name, mode, filters) in [
        ("hybrid", SearchMode::Hybrid, Filters::default()),
        ("vector", SearchMode::Vector, Filters::default()),
        ("keyword", SearchMode::Keyword, Filters::default()),
        (
            "hybrid + type filter",
            SearchMode::Hybrid,
            Filters {
                types: vec![Kind::Image],
                ..Default::default()
            },
        ),
    ] {
        let times = run(mode, filters, 200);
        eprintln!(
            "{name:<22} p50 {:6.1} ms   p95 {:6.1} ms   max {:6.1} ms",
            percentile(&times, 0.5),
            percentile(&times, 0.95),
            times[times.len() - 1]
        );
        p50s.push(percentile(&times, 0.5));
    }
    assert!(p50s[0] < 50.0, "hybrid p50 {} ms", p50s[0]);
}

/// First-pass throughput with the fake embedder: how fast scanning, extraction, hashing,
/// thumbnails and storage go when embedding costs nothing. Indexes `RUMMAGE_BENCH_DIR`
/// (read-only) if set, else a generated folder.
#[test]
#[ignore]
fn first_pass_throughput() {
    let dir = tempfile::tempdir().unwrap();
    let folder = match std::env::var("RUMMAGE_BENCH_DIR") {
        Ok(d) => std::fs::canonicalize(d).unwrap(),
        Err(_) => {
            let root = dir.path().join("files");
            generate(&root);
            root
        }
    };
    let svc = service(dir.path(), &[&folder]);
    let started = Instant::now();
    let handle = indexer::start(svc.clone(), indexer::Options::default()).unwrap();
    assert!(handle.wait_idle(Some(Duration::from_secs(3600))));
    handle.stop();
    let secs = started.elapsed().as_secs_f64();
    let s = svc.index_status().unwrap();
    eprintln!("CPU: {}", cpu());
    eprintln!(
        "{} files ({:.1} MB, {} chunks) in {secs:.1} s: {:.0} files/s, {:.1} MB/s; {} errors, {} skipped",
        s.totals.files,
        s.totals.bytes as f64 / 1e6,
        s.totals.chunks,
        s.totals.files as f64 / secs,
        s.totals.bytes as f64 / 1e6 / secs,
        s.totals.errors,
        s.totals.skipped
    );
    for k in &s.by_kind {
        eprintln!("  {} {}", k.files, k.kind);
    }
}

/// 2000 text files, 300 photos (1600x1200 JPEG) and 50 one-minute WAVs.
fn generate(root: &Path) {
    let mut rng = Rng(7);
    for i in 0..2000 {
        let p = root.join(format!("notes/{}/note{i}.md", i % 40));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let body: Vec<String> = (0..700)
            .map(|_| format!("w{}", rng.next() % 5000))
            .collect();
        std::fs::write(p, format!("# Note {i}\n\n{}", body.join(" "))).unwrap();
    }
    for i in 0..300 {
        let p = root.join(format!("photos/{}/img{i}.jpg", i % 10));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let seed = rng.next();
        let img = image::RgbImage::from_fn(1600, 1200, |x, y| {
            image::Rgb([
                (x as u64 ^ seed) as u8,
                (y as u64 + seed) as u8,
                ((x + y) as u64 * seed) as u8,
            ])
        });
        img.save(p).unwrap();
    }
    for i in 0..50 {
        let p = root.join(format!("audio/clip{i}.wav"));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let rate = 44100u32;
        let samples: Vec<i16> = (0..rate * 60)
            .map(|n| {
                ((n as f32 * (200.0 + i as f32) / rate as f32 * std::f32::consts::TAU).sin()
                    * 9000.0) as i16
            })
            .collect();
        let mut wav = vec![];
        let data = samples.len() as u32 * 2;
        wav.extend(b"RIFF");
        wav.extend((36 + data).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(rate.to_le_bytes());
        wav.extend((rate * 2).to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(data.to_le_bytes());
        samples.iter().for_each(|s| wav.extend(s.to_le_bytes()));
        std::fs::write(p, wav).unwrap();
    }
}
