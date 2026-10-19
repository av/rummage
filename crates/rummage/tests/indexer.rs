//! The indexer end to end over a folder with one file of each kind: index, search each kind
//! (by text, by keyword, by uploaded file), then edit, delete, move and add files and check
//! the index follows within 10 s. Uses the deterministic fake embedder.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use rummage::extract::Tools;
use rummage::{indexer, server};
use rummage_core::service::Service;
use rummage_core::types::*;
use rummage_core::{ConfigFile, EmbedInput, Embedder, FakeEmbedder, Modalities};
use tower::ServiceExt;

/// The fake embedder, counting how many inputs it embedded.
struct Counting {
    inner: FakeEmbedder,
    inputs: AtomicUsize,
}

impl Embedder for Counting {
    fn dim(&self) -> usize {
        self.inner.dim()
    }
    fn embed(&self, inputs: &[EmbedInput]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.inputs.fetch_add(inputs.len(), Ordering::SeqCst);
        self.inner.embed(inputs)
    }
    fn modalities(&self) -> Modalities {
        Modalities::ALL
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// 16-bit mono WAV of a rising tone.
fn wav(seconds: f32, rate: u32) -> Vec<u8> {
    let n = (seconds * rate as f32) as usize;
    let samples: Vec<i16> = (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            ((t * (300.0 + 200.0 * t) * std::f32::consts::TAU).sin() * 12000.0) as i16
        })
        .collect();
    let data = samples.len() as u32 * 2;
    let mut out = vec![];
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend((rate * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    samples.iter().for_each(|s| out.extend(s.to_le_bytes()));
    out
}

fn png() -> Vec<u8> {
    let img = image::RgbImage::from_fn(64, 48, |x, y| image::Rgb([x as u8 * 4, y as u8 * 5, 90]));
    let mut out = vec![];
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
    svc: Arc<Service>,
    embedder: Arc<Counting>,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("files");
    let w = |rel: &str, bytes: &[u8]| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    };
    for f in [
        "beach.jpg",
        "lighthouse-report.pdf",
        "chime.mp3",
        "clip.mp4",
    ] {
        w(f, &std::fs::read(fixture(f)).unwrap());
    }
    w("sunset.png", &png());
    w(
        "notes/garden.md",
        b"# Garden plans\n\nPlant the tomato seedlings after the last frost.\n\nWater daily.",
    );
    w(
        "todo.txt",
        b"Call the dentist about the appointment on Friday.",
    );
    w(
        "src/parser.rs",
        b"/// Splits input into tokens.\npub fn tokenizer(input: &str) -> Vec<String> {\n    input.split_whitespace().map(String::from).collect()\n}\n",
    );
    w("birdsong.wav", &wav(3.0, 16000));
    // Never indexed: a skipped directory, an excluded glob, an unknown type.
    w("node_modules/pkg/index.js", b"module.exports = 'tomato';");
    w("scratch/draft.tmp.md", b"tomato");
    w("fedora.iso", b"\0\0\0binary");

    let root = std::fs::canonicalize(&root).unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        format!(
            "data_dir = \"{}\"\n[index]\nconcurrency = 3\n[[folders]]\npath = \"{}\"\nexclude = [\"**/*.tmp.md\"]\n",
            dir.path().join("data").display(),
            root.display()
        ),
    )
    .unwrap();
    let cf = ConfigFile::load_with_env(&config, |_| None).unwrap();
    let embedder = Arc::new(Counting {
        inner: FakeEmbedder::new(768),
        inputs: AtomicUsize::new(0),
    });
    let svc = Arc::new(Service::open(cf, embedder.clone()).unwrap());
    svc.set_decoder(Arc::new(rummage::extract::upload::Decoder));
    Setup {
        _dir: dir,
        root,
        svc,
        embedder,
    }
}

fn search_mode(svc: &Service, query: &str, types: &[Kind], mode: SearchMode) -> SearchResponse {
    svc.search(
        SearchRequest {
            query: Some(query.into()),
            filters: Filters {
                types: types.to_vec(),
                ..Default::default()
            },
            mode,
            limit: Some(20),
            ..Default::default()
        },
        "",
    )
    .unwrap()
}

/// Hybrid search (meaning and words).
fn search(svc: &Service, query: &str, types: &[Kind]) -> SearchResponse {
    search_mode(svc, query, types, SearchMode::Hybrid)
}

/// Names of the files that contain the words (vector search ranks every file, so presence
/// and absence are checked by keyword).
fn word_hits(svc: &Service, query: &str) -> Vec<String> {
    search_mode(svc, query, &[], SearchMode::Keyword)
        .hits
        .into_iter()
        .map(|h| h.item.name)
        .collect()
}

fn upload(svc: &Service, path: &Path, mime: &str) -> SearchResponse {
    svc.search_upload(
        SearchRequest::default(),
        path.file_name().map(|n| n.to_string_lossy().into_owned()),
        Some(mime.into()),
        std::fs::read(path).unwrap(),
        "",
    )
    .unwrap()
}

fn top(r: &SearchResponse) -> &str {
    r.hits.first().map(|h| h.item.name.as_str()).unwrap_or("")
}

/// Polls `check` until it holds; returns how long that took.
fn within(limit: Duration, what: &str, check: impl Fn() -> bool) -> Duration {
    let started = Instant::now();
    while !check() {
        assert!(started.elapsed() < limit, "{what}: not within {limit:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
    let took = started.elapsed();
    eprintln!("{what}: {:.2} s", took.as_secs_f64());
    took
}

async fn get(app: &server::App, uri: &str) -> (StatusCode, String, Vec<u8>) {
    let resp = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let ctype = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, ctype, body)
}

#[tokio::test(flavor = "multi_thread")]
async fn indexes_every_kind_and_follows_changes() {
    let s = setup();
    let svc = s.svc.clone();
    let tools = Tools::detect();
    eprintln!("tools: {tools:?}");

    let started = Instant::now();
    let handle = indexer::start(svc.clone(), indexer::Options { watch: true }).unwrap();
    assert!(
        handle.wait_idle(Some(Duration::from_secs(60))),
        "first pass"
    );
    eprintln!("first pass: {:.2} s", started.elapsed().as_secs_f64());

    let status = svc.index_status().unwrap();
    assert_eq!(status.state, IndexState::Idle);
    assert_eq!(status.totals.files, 9, "{status:#?}");
    assert_eq!(
        status.totals.errors,
        0,
        "{:#?}",
        svc.problems(&Default::default(), "")
    );
    let kinds: Vec<(Kind, u64)> = status.by_kind.iter().map(|k| (k.kind, k.files)).collect();
    assert_eq!(
        kinds,
        vec![
            (Kind::Audio, 2),
            (Kind::Doc, 1),
            (Kind::Image, 2),
            (Kind::Text, 3),
            (Kind::Video, 1)
        ]
    );

    // Text, markdown and code, by meaning and words.
    let r = search(&svc, "tomato seedlings", &[Kind::Text]);
    assert_eq!(top(&r), "garden.md");
    let seg = r.hits[0].segment.as_ref().unwrap();
    assert!(
        seg.snippet.iter().any(|p| p.hit && p.text == "tomato"),
        "{seg:?}"
    );
    assert_eq!(
        word_hits(&svc, "tomato"),
        vec!["garden.md"],
        "skipped and excluded files are not indexed"
    );
    assert_eq!(top(&search(&svc, "dentist appointment", &[])), "todo.txt");
    assert_eq!(top(&search(&svc, "tokenizer", &[Kind::Text])), "parser.rs");

    // PDF: page text with its page number.
    let r = search(&svc, "foghorn", &[Kind::Doc]);
    assert_eq!(top(&r), "lighthouse-report.pdf");
    assert_eq!(r.hits[0].segment.as_ref().unwrap().page, Some(2));
    let pdf_id = r.hits[0].item.id;
    let pdf = svc.item(pdf_id, None, "").unwrap();
    let page_images = pdf
        .segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Page)
        .count();
    assert_eq!(page_images, if tools.pdftoppm { 2 } else { 0 });

    // Images: EXIF in the keyword index, date taken, GPS; search by upload.
    let r = search(&svc, "TestCam", &[Kind::Image]);
    assert_eq!(top(&r), "beach.jpg");
    let beach = r.hits[0].item.clone();
    assert_eq!(beach.date, 1626258030, "2021-07-14 10:20:30");
    let exif = beach.exif.clone().unwrap();
    assert!(
        (exif["GPSLatitude"].as_f64().unwrap() - 52.225).abs() < 1e-3,
        "{exif}"
    );
    assert_eq!(exif["width"], 320);
    assert_eq!(exif["Make"], "TestCam");
    assert_eq!(exif["DateTimeOriginal"], "2021-07-14 10:20:30");
    let r = upload(&svc, &s.root.join("beach.jpg"), "image/jpeg");
    assert_eq!(top(&r), "beach.jpg");
    assert!(r.hits[0].similarity.unwrap() > 0.99);
    let r = upload(&svc, &s.root.join("sunset.png"), "image/png");
    assert_eq!(top(&r), "sunset.png");
    assert!(r.hits[0].similarity.unwrap() > 0.99);

    // Audio: wav by upload, mp3 by name, with time windows.
    let r = upload(&svc, &s.root.join("birdsong.wav"), "audio/wav");
    assert_eq!(top(&r), "birdsong.wav");
    assert!(r.hits[0].similarity.unwrap() > 0.99);
    let r = search(&svc, "chime", &[Kind::Audio]);
    assert_eq!(top(&r), "chime.mp3");
    let seg = r.hits[0].segment.as_ref().unwrap();
    assert_eq!(seg.kind, SegmentKind::Audio);
    assert_eq!(seg.t_start, Some(0.0));
    assert!((seg.t_end.unwrap() - 4.0).abs() < 0.2, "{seg:?}");

    // Video: 1 fps frames in 10 s windows plus the audio track; skipped without ffmpeg.
    let r = search(&svc, "clip", &[Kind::Video]);
    assert_eq!(top(&r), "clip.mp4");
    let video = r.hits[0].item.clone();
    if tools.ffmpeg {
        assert_eq!(video.status, FileStatus::Indexed);
        assert_eq!(video.date, 1646370367, "creation_time");
        let d = svc.item(video.id, None, "").unwrap();
        let windows: Vec<(SegmentKind, f64, f64)> = d
            .segments
            .iter()
            .map(|s| (s.kind, s.t_start.unwrap(), s.t_end.unwrap().round()))
            .collect();
        assert_eq!(
            windows,
            vec![
                (SegmentKind::Video, 0.0, 10.0),
                (SegmentKind::Video, 10.0, 12.0),
                (SegmentKind::Audio, 0.0, 12.0)
            ]
        );
        let similar = svc.similar(video.id, &Default::default(), "").unwrap();
        assert!(similar.hits.iter().all(|h| h.item.id != video.id));
    } else {
        assert_eq!(video.status, FileStatus::Skipped);
        assert!(video.error.unwrap().contains("ffmpeg"));
        assert!(status.messages.iter().any(|m| m.contains("ffmpeg")));
    }

    // Thumbnails, previews and ranges over HTTP.
    let app = server::app(svc.clone());
    let (code, ctype, body) = get(&app, &format!("/api/v1/items/{}/thumb", beach.id)).await;
    assert_eq!((code, ctype.as_str()), (StatusCode::OK, "image/webp"));
    assert_eq!(&body[8..12], b"WEBP");
    let wav_id = top_id(&search(&svc, "birdsong", &[]));
    let (code, ctype, _) = get(&app, &format!("/api/v1/items/{wav_id}/thumb")).await;
    assert_eq!(
        (code, ctype.as_str()),
        (StatusCode::OK, "image/webp"),
        "waveform"
    );
    let (code, _, body) = get(&app, &format!("/api/v1/items/{wav_id}/waveform?points=50")).await;
    assert_eq!(code, StatusCode::OK);
    let w: Waveform = serde_json::from_slice(&body).unwrap();
    assert_eq!(w.peaks.len(), 50);
    assert!((w.duration - 3.0).abs() < 0.01, "{}", w.duration);
    assert!(w.peaks.iter().all(|p| (0.0..=1.0).contains(p)) && w.peaks[10] > 0.2);
    let (code, _, _) = get(&app, &format!("/api/v1/items/{pdf_id}/preview?page=2")).await;
    assert_eq!(code == StatusCode::OK, tools.pdftoppm);
    if tools.ffmpeg {
        let (code, ctype, _) = get(&app, &format!("/api/v1/items/{}/thumb?t=11", video.id)).await;
        assert_eq!(
            (code, ctype.as_str()),
            (StatusCode::OK, "image/webp"),
            "poster"
        );
        let (code, _, _) = get(&app, &format!("/api/v1/items/{}/preview?t=5", video.id)).await;
        assert_eq!(code, StatusCode::OK, "frame");
    }
    let (code, _, body) = get(&app, "/api/v1/index/problems").await;
    assert_eq!(code, StatusCode::OK);
    let problems: ProblemList = serde_json::from_slice(&body).unwrap();
    assert_eq!(problems.total, if tools.ffmpeg { 0 } else { 1 });

    // Now change things and watch the index follow.
    let calls_before = s.embedder.inputs.load(Ordering::SeqCst);
    std::fs::write(
        s.root.join("notes/garden.md"),
        "# Garden plans\n\nPlant the tomato seedlings after the last frost.\n\nAlso sow zucchini.",
    )
    .unwrap();
    let edit = within(Duration::from_secs(10), "edited file searchable", || {
        top(&search(&svc, "zucchini", &[])) == "garden.md"
    });

    std::fs::remove_file(s.root.join("todo.txt")).unwrap();
    let delete = within(Duration::from_secs(10), "deleted file gone", || {
        svc.index_status().unwrap().totals.files == 8 && word_hits(&svc, "dentist").is_empty()
    });

    let embedded_before_move = s.embedder.inputs.load(Ordering::SeqCst);
    let chunks_before = svc.item(beach.id, None, "").unwrap().segments;
    std::fs::create_dir_all(s.root.join("photos/2021")).unwrap();
    std::fs::rename(
        s.root.join("beach.jpg"),
        s.root.join("photos/2021/beach-moved.jpg"),
    )
    .unwrap();
    let moved = within(Duration::from_secs(10), "moved file followed", || {
        svc.item(beach.id, None, "")
            .is_ok_and(|d| d.item.path.ends_with("photos/2021/beach-moved.jpg"))
    });
    assert_eq!(
        s.embedder.inputs.load(Ordering::SeqCst),
        embedded_before_move,
        "a move re-embeds nothing"
    );
    assert_eq!(
        svc.item(beach.id, None, "").unwrap().segments,
        chunks_before
    );
    assert_eq!(
        top(&search(&svc, "beach moved", &[Kind::Image])),
        "beach-moved.jpg"
    );

    std::fs::write(
        s.root.join("notes/new.md"),
        "A quokka smiled at the camera.",
    )
    .unwrap();
    let added = within(Duration::from_secs(10), "new file searchable", || {
        top(&search(&svc, "quokka", &[])) == "new.md"
    });
    assert!(s.embedder.inputs.load(Ordering::SeqCst) > calls_before);

    // Pause holds new work; resume picks it up.
    svc.pause().unwrap();
    std::fs::write(s.root.join("notes/paused.md"), "wombat burrow").unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert!(word_hits(&svc, "wombat").is_empty());
    assert_eq!(svc.index_status().unwrap().state, IndexState::Paused);
    svc.resume().unwrap();
    within(Duration::from_secs(10), "resumed", || {
        top(&search(&svc, "wombat", &[])) == "paused.md"
    });

    let status = svc.index_status().unwrap();
    assert_eq!(status.totals.files, 10, "{status:#?}");
    eprintln!(
        "follow-up times: edit {:.2} s, delete {:.2} s, move {:.2} s, add {:.2} s",
        edit.as_secs_f64(),
        delete.as_secs_f64(),
        moved.as_secs_f64(),
        added.as_secs_f64()
    );
    handle.stop();
    assert!(!svc.live().attached);
}

fn top_id(r: &SearchResponse) -> i64 {
    r.hits[0].item.id
}

/// A one-shot pass (`rummage index --once`) over the same folder, then a rescan that finds
/// nothing to do, and a restart that picks up changes made while stopped.
#[test]
fn run_once_is_incremental() {
    let s = setup();
    indexer::run_once(s.svc.clone()).unwrap();
    let first = s.embedder.inputs.load(Ordering::SeqCst);
    assert!(first > 0);
    assert_eq!(s.svc.index_status().unwrap().totals.pending, 0);

    indexer::run_once(s.svc.clone()).unwrap();
    assert_eq!(
        s.embedder.inputs.load(Ordering::SeqCst),
        first,
        "nothing changed"
    );

    std::fs::rename(s.root.join("todo.txt"), s.root.join("renamed.txt")).unwrap();
    std::fs::write(s.root.join("src/parser.rs"), "fn lexer() {}").unwrap();
    std::fs::remove_file(s.root.join("chime.mp3")).unwrap();
    indexer::run_once(s.svc.clone()).unwrap();
    assert_eq!(
        s.embedder.inputs.load(Ordering::SeqCst),
        first + 1,
        "only the edited file is embedded again"
    );
    assert_eq!(top(&search(&s.svc, "dentist", &[])), "renamed.txt");
    assert_eq!(top(&search(&s.svc, "lexer", &[])), "parser.rs");
    assert!(word_hits(&s.svc, "tokenizer").is_empty());
    assert!(word_hits(&s.svc, "chime").is_empty());
}

/// An embedder whose model is still loading.
struct Loading(FakeEmbedder);

impl Embedder for Loading {
    fn dim(&self) -> usize {
        self.0.dim()
    }
    fn embed(&self, _: &[EmbedInput]) -> anyhow::Result<Vec<Vec<f32>>> {
        anyhow::bail!("must not be called while loading")
    }
    fn modalities(&self) -> Modalities {
        Modalities::ALL
    }
    fn ready(&self) -> bool {
        false
    }
}

/// While the model loads, hybrid search answers from keywords instead of waiting, and the
/// status says why.
#[test]
fn searches_fall_back_to_words_while_the_model_loads() {
    let s = setup();
    indexer::run_once(s.svc.clone()).unwrap();
    let cf = ConfigFile::load_with_env(&s._dir.path().join("config.toml"), |_| None).unwrap();
    let svc = Service::open(cf, Arc::new(Loading(FakeEmbedder::new(768)))).unwrap();
    let r = search(&svc, "dentist", &[]);
    assert_eq!(top(&r), "todo.txt");
    assert!(r.hits[0].similarity.is_none());
    let err = svc
        .search(
            SearchRequest {
                query: Some("dentist".into()),
                mode: SearchMode::Vector,
                ..Default::default()
            },
            "",
        )
        .unwrap_err();
    assert_eq!(err.code(), "not_ready");
    let status = svc.index_status().unwrap();
    assert!(
        status.messages.iter().any(|m| m.contains("not ready")),
        "{status:?}"
    );
}
