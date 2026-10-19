//! Playable copies of videos browsers often can't play (HEVC phone videos in Chrome on Linux):
//! H.264 + AAC in a fast-start MP4, at most 1280 px on the long edge, made with ffmpeg when a
//! video is first opened or ahead of time in the background, and kept under
//! `<data_dir>/playable/<id>/` within `playback.cache_mb` (the least recently played go first).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, anyhow, bail};
use rummage_core::service::{Error, Result, Service, needs_playable_copy};
use rummage_core::store::FileRow;
use rummage_core::types::{Kind, PlayStatus};

use super::Tools;

/// What `/items/{id}/play` sends.
pub enum Play {
    /// The original plays in browsers, or conversion is off: send the file itself.
    Original,
    /// The copy, ready to stream.
    Ready(PathBuf),
    /// The copy is being made.
    Pending(PlayStatus),
}

/// On-demand conversions run at once; more wait for a turn.
const ON_DEMAND: usize = 2;
/// A failed conversion is tried again after this long.
const RETRY_AFTER: Duration = Duration::from_secs(600);
/// The background stops making copies when the cache is this full (it never evicts).
const PREPARE_UP_TO: f64 = 0.75;
/// Encoder threads for background conversions (they also run at the lowest priority).
const BACKGROUND_THREADS: &str = "4";

#[derive(Default)]
struct Progress {
    started: AtomicBool,
    /// 0 to 1000.
    permille: AtomicU32,
}

impl Progress {
    fn status(&self) -> PlayStatus {
        let started = self.started.load(Ordering::Relaxed);
        PlayStatus {
            state: if started { "converting" } else { "waiting" }.into(),
            progress: started.then(|| self.permille.load(Ordering::Relaxed) as f32 / 1000.0),
        }
    }
}

#[derive(Default)]
struct Jobs {
    running: HashMap<i64, Arc<Progress>>,
    failed: HashMap<i64, (Instant, String)>,
    /// On-demand conversions running now.
    active: usize,
}

static JOBS: Mutex<Option<Jobs>> = Mutex::new(None);
static TURN: Condvar = Condvar::new();

fn jobs() -> MutexGuard<'static, Option<Jobs>> {
    let mut g = JOBS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(Jobs::default);
    g
}

/// The copies' folder.
fn cache_dir(svc: &Service) -> PathBuf {
    svc.config().data_dir().join("playable")
}

/// Where the copy of `id` is kept.
pub fn copy_path(svc: &Service, id: i64) -> PathBuf {
    cache_dir(svc).join(id.to_string()).join("h264.mp4")
}

/// The playable form of video `id`: the original, the copy, or how far making it has got
/// (starting it if needed).
pub fn play(svc: &Arc<Service>, id: i64) -> Result<Play> {
    let f = svc
        .store()
        .file(id)?
        .ok_or_else(|| Error::NotFound(format!("no item {id}")))?;
    if f.kind != Kind::Video {
        return Err(Error::Unsupported(format!("item {id} is not a video")));
    }
    if !svc.config().playback.convert || !needs_playable_copy(&f) {
        return Ok(Play::Original);
    }
    let out = copy_path(svc, id);
    let mut g = jobs();
    let jobs = g.as_mut().expect("initialised");
    if let Some(p) = jobs.running.get(&id) {
        return Ok(Play::Pending(p.status()));
    }
    if out.exists() {
        drop(g);
        touch(&out);
        return Ok(Play::Ready(out));
    }
    if let Some((at, why)) = jobs.failed.get(&id)
        && at.elapsed() < RETRY_AFTER
    {
        return Err(Error::Unsupported(format!(
            "can't make a playable copy of item {id}: {why}"
        )));
    }
    if !Tools::cached().ffmpeg {
        return Err(Error::Unsupported(
            "playable copies need ffmpeg, which was not found".into(),
        ));
    }
    let progress = Arc::new(Progress::default());
    jobs.running.insert(id, progress.clone());
    drop(g);
    let status = progress.status();
    let svc = svc.clone();
    std::thread::spawn(move || run_job(&svc, &f, &progress, false));
    Ok(Play::Pending(status))
}

/// Makes the copy of `f` (registered as running), then records the outcome.
fn run_job(svc: &Service, f: &FileRow, progress: &Progress, background: bool) {
    if !background {
        let mut g = jobs();
        while g.as_ref().expect("initialised").active >= ON_DEMAND {
            g = TURN.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        g.as_mut().expect("initialised").active += 1;
    }
    progress.started.store(true, Ordering::Relaxed);
    let started = Instant::now();
    let out = copy_path(svc, f.id);
    let tmp = cache_dir(svc).join(format!(".tmp-{}-{}.mp4", f.id, std::process::id()));
    let result = std::fs::create_dir_all(cache_dir(svc))
        .map_err(anyhow::Error::from)
        .and_then(|_| convert(Path::new(&f.path), &tmp, duration(f), progress, background))
        .and_then(|_| {
            std::fs::create_dir_all(out.parent().expect("has a parent"))?;
            std::fs::rename(&tmp, &out)?;
            Ok(())
        });
    let _ = std::fs::remove_file(&tmp);
    let mut g = jobs();
    let jobs = g.as_mut().expect("initialised");
    jobs.running.remove(&f.id);
    if !background {
        jobs.active -= 1;
        TURN.notify_all();
    }
    match result {
        Ok(()) => {
            jobs.failed.remove(&f.id);
            drop(g);
            tracing::info!(
                "made a playable copy of {} in {:.1} s{}",
                f.path,
                started.elapsed().as_secs_f64(),
                if background {
                    " (in the background)"
                } else {
                    ""
                }
            );
            // The background stops short of the limit instead.
            if !background {
                let cap = svc.config().playback.cache_mb * 1024 * 1024;
                evict(&cache_dir(svc), cap, &out);
            }
        }
        Err(e) => {
            tracing::warn!("can't make a playable copy of {}: {e:#}", f.path);
            jobs.failed.insert(f.id, (Instant::now(), format!("{e:#}")));
        }
    }
}

/// The duration the indexer probed, in seconds.
fn duration(f: &FileRow) -> Option<f64> {
    let exif: serde_json::Value = serde_json::from_str(f.exif.as_deref()?).ok()?;
    exif["duration"].as_f64().filter(|d| *d > 0.0)
}

/// The H.264 encoder arguments for this ffmpeg: x264 if it has it, else OpenH264.
fn encoder() -> Option<&'static [&'static str]> {
    static ENCODER: OnceLock<Option<&'static [&'static str]>> = OnceLock::new();
    *ENCODER.get_or_init(|| {
        let list = super::run(super::ffmpeg().args(["-hide_banner", "-encoders"])).ok()?;
        let list = String::from_utf8_lossy(&list);
        let has = |name: &str| list.split_whitespace().any(|w| w == name);
        if has("libx264") {
            Some(&["-c:v", "libx264", "-preset", "veryfast", "-crf", "23"])
        } else if has("libopenh264") {
            Some(&["-c:v", "libopenh264", "-b:v", "4M"])
        } else {
            None
        }
    })
}

/// Converts `src` to a fast-start H.264/AAC MP4 at `out`, reporting progress.
fn convert(
    src: &Path,
    out: &Path,
    duration: Option<f64>,
    progress: &Progress,
    background: bool,
) -> anyhow::Result<()> {
    let encoder = encoder().ok_or_else(|| anyhow!("this ffmpeg has no H.264 encoder"))?;
    let mut cmd = if background {
        super::ffmpeg_niced()
    } else {
        super::ffmpeg()
    };
    cmd.args(["-v", "error", "-nostdin", "-y"]);
    if background {
        cmd.args(["-threads", BACKGROUND_THREADS]);
    }
    cmd.arg("-i")
        .arg(src)
        .args(["-map", "0:v:0", "-map", "0:a:0?", "-sn", "-dn"])
        // Fit 1280 px without upscaling; 8-bit 4:2:0 in BT.709, which every browser decodes
        // (HDR phone video is HLG, which looks right shown as plain BT.709).
        .args([
            "-vf",
            "scale='min(iw,1280)':'min(ih,1280)':force_original_aspect_ratio=decrease\
             :force_divisible_by=2:out_color_matrix=bt709:out_range=tv,format=yuv420p",
        ])
        .args(["-fpsmax", "60"])
        .args(encoder)
        // A keyframe every 2 s or so, for quick seeking.
        .args(["-g", "60"])
        .args(["-colorspace", "bt709", "-color_primaries", "bt709"])
        .args(["-color_trc", "bt709"])
        .args(["-c:a", "aac", "-b:a", "128k", "-ac", "2"])
        .args(["-movflags", "+faststart", "-progress", "pipe:1", "-nostats"]);
    if background {
        cmd.args(["-threads", BACKGROUND_THREADS]);
    }
    let mut child = cmd
        .args(["-f", "mp4"])
        .arg(out)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting ffmpeg")?;
    let stderr = child.stderr.take().expect("piped");
    let errors = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut s);
        s
    });
    for line in BufReader::new(child.stdout.take().expect("piped"))
        .lines()
        .map_while(|l| l.ok())
    {
        if let (Some(us), Some(d)) = (line.strip_prefix("out_time_us="), duration)
            && let Ok(us) = us.trim().parse::<f64>()
        {
            let done = (us / 1e6 / d * 1000.0).clamp(0.0, 999.0);
            progress.permille.store(done as u32, Ordering::Relaxed);
        }
    }
    let status = child.wait()?;
    let err = errors.join().unwrap_or_default();
    if !status.success() {
        bail!(
            "ffmpeg failed: {}",
            err.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("no output")
        );
    }
    Ok(())
}

/// Marks a copy as just played (its modification time orders eviction).
fn touch(p: &Path) {
    let fresh = std::fs::metadata(p)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().unwrap_or_default() < Duration::from_secs(60));
    if !fresh && let Ok(f) = std::fs::File::options().write(true).open(p) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// The copies in `dir`: `(last played, bytes, path)`.
fn copies(dir: &Path) -> Vec<(SystemTime, u64, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("h264.mp4"))
        .filter_map(|p| {
            let m = std::fs::metadata(&p).ok()?;
            Some((m.modified().ok()?, m.len(), p))
        })
        .collect()
}

/// Removes the least recently played copies until the rest fit in `cap` bytes; `keep` stays.
fn evict(dir: &Path, cap: u64, keep: &Path) {
    let mut all = copies(dir);
    let mut total: u64 = all.iter().map(|c| c.1).sum();
    all.sort();
    for (_, bytes, p) in all {
        if total <= cap {
            break;
        }
        if p == keep {
            continue;
        }
        if let Some(d) = p.parent() {
            let _ = std::fs::remove_dir_all(d);
            total -= bytes;
            tracing::debug!("dropped the playable copy {}", p.display());
        }
    }
}

/// Makes copies of the videos that need one in the background, newest first, one at a time
/// at the lowest priority, while the indexer is idle and the cache has room. Never returns.
pub fn prepare(svc: Arc<Service>) {
    // Leftovers of conversions cut short by a restart.
    if let Ok(entries) = std::fs::read_dir(cache_dir(&svc)) {
        for e in entries.filter_map(|e| e.ok()) {
            if e.file_name().to_string_lossy().starts_with(".tmp-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    std::thread::sleep(Duration::from_secs(60));
    loop {
        let made = prepare_one(&svc).unwrap_or_else(|e| {
            tracing::warn!("preparing playable copies: {e:#}");
            false
        });
        std::thread::sleep(Duration::from_secs(if made { 5 } else { 600 }));
    }
}

/// Makes one missing copy; false when there was nothing to do (or no room, or not now).
fn prepare_one(svc: &Arc<Service>) -> anyhow::Result<bool> {
    let cfg = svc.config().playback;
    if !(cfg.convert && cfg.prepare) || !Tools::cached().ffmpeg || svc.live().busy {
        return Ok(false);
    }
    let used: u64 = copies(&cache_dir(svc)).iter().map(|c| c.1).sum();
    if used as f64 >= cfg.cache_mb as f64 * 1024.0 * 1024.0 * PREPARE_UP_TO {
        return Ok(false);
    }
    let store = svc.store();
    let ids = store.filtered_files(&[Kind::Video], &[], None, None, None)?;
    let mut todo: Vec<FileRow> = store
        .files(&ids)?
        .into_iter()
        .filter(|f| f.status == "indexed" && needs_playable_copy(f))
        .collect();
    todo.sort_by_key(|f| std::cmp::Reverse(f.date.unwrap_or(f.mtime)));
    let progress = Arc::new(Progress::default());
    let next = {
        let mut g = jobs();
        let jobs = g.as_mut().expect("initialised");
        let next = todo.into_iter().find(|f| {
            !jobs.running.contains_key(&f.id)
                && !jobs.failed.contains_key(&f.id)
                && !copy_path(svc, f.id).exists()
        });
        if let Some(f) = &next {
            jobs.running.insert(f.id, progress.clone());
        }
        next
    };
    let Some(f) = next else {
        return Ok(false);
    };
    run_job(svc, &f, &progress, true);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_the_least_recently_played() {
        let dir = tempfile::tempdir().unwrap();
        let make = |id: u32, age_secs: u64| {
            let p = dir.path().join(id.to_string()).join("h264.mp4");
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let f = std::fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(SystemTime::now() - Duration::from_secs(age_secs))
                .unwrap();
            p
        };
        let old = make(1, 300);
        let mid = make(2, 200);
        let new = make(3, 100);
        let keep = make(4, 400);
        evict(dir.path(), 250, &keep);
        assert!(!old.exists() && !mid.exists());
        assert!(new.exists() && keep.exists());
        assert_eq!(copies(dir.path()).len(), 2);
    }
}
