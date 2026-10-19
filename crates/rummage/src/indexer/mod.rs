//! Scanning, watching, extraction and embedding of the watched folders.
//!
//! One coordinator thread walks the folders, reads file-change events (debounced) and runs a
//! periodic rescan; it queues files by priority (changes first, then backfill). Worker threads
//! take files off the queue, extract them, embed the pieces (searches get the embedder first)
//! and store the result. A failing file is recorded with its error and never stops the rest.

mod scan;
mod watch;

use std::collections::{HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use rummage_core::Service;
use rummage_core::service::IndexCommand;
use rummage_core::store::FileContent;
use rummage_core::types::{FileStatus, IndexEvent};

use crate::extract::{self, Ctx, Outcome, Tools, render};
use scan::{FolderRules, hash_file};
use watch::Watcher;

/// Quiet time after the last file event before a batch is handled.
const DEBOUNCE: Duration = Duration::from_millis(500);
/// Longest a batch waits while events keep coming.
const DEBOUNCE_MAX: Duration = Duration::from_secs(2);

/// Queue priority: lower runs first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// New and changed files seen while running.
    Live,
    /// The first pass over a folder, and re-indexing.
    Backfill,
}

pub enum Msg {
    Cmd(IndexCommand),
    Fs(Vec<PathBuf>),
    Stop,
}

#[derive(Default)]
struct QueueState {
    live: VecDeque<i64>,
    backfill: VecDeque<i64>,
    queued: HashSet<i64>,
    active: usize,
    paused: bool,
    stop: bool,
}

/// Files waiting for a worker.
#[derive(Default)]
struct Queue {
    state: Mutex<QueueState>,
    ready: Condvar,
}

impl Queue {
    fn lock(&self) -> std::sync::MutexGuard<'_, QueueState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push(&self, id: i64, prio: Priority) {
        let mut s = self.lock();
        if s.queued.insert(id) {
            match prio {
                Priority::Live => s.live.push_back(id),
                Priority::Backfill => s.backfill.push_back(id),
            }
            self.ready.notify_one();
        } else if prio == Priority::Live
            && let Some(pos) = s.backfill.iter().position(|x| *x == id)
        {
            s.backfill.remove(pos);
            s.live.push_back(id);
        }
    }

    fn remove(&self, id: i64) {
        let mut s = self.lock();
        if s.queued.remove(&id) {
            s.live.retain(|x| *x != id);
            s.backfill.retain(|x| *x != id);
        }
    }

    /// The next file, waiting while empty or paused; `None` when stopping.
    fn pop(&self) -> Option<i64> {
        let mut s = self.lock();
        loop {
            if s.stop {
                return None;
            }
            if !s.paused
                && let Some(id) = s.live.pop_front().or_else(|| s.backfill.pop_front())
            {
                s.queued.remove(&id);
                s.active += 1;
                return Some(id);
            }
            s = self.ready.wait(s).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// A worker finished a file. Returns true when nothing is queued or running.
    fn finish(&self) -> bool {
        let mut s = self.lock();
        s.active -= 1;
        s.queued.is_empty() && s.active == 0
    }

    fn idle(&self) -> bool {
        let s = self.lock();
        s.queued.is_empty() && s.active == 0
    }

    fn set_paused(&self, paused: bool) {
        self.lock().paused = paused;
        self.ready.notify_all();
    }

    fn stop(&self) {
        self.lock().stop = true;
        self.ready.notify_all();
    }

    fn stopping(&self) -> bool {
        self.lock().stop
    }
}

/// State shared by the coordinator and the workers.
pub struct Shared {
    svc: Arc<Service>,
    queue: Queue,
    tools: RwLock<Tools>,
    /// The coordinator is walking folders or handling events.
    coordinating: AtomicBool,
    /// Events received but not handled yet.
    batched: AtomicUsize,
    first_pass_done: AtomicBool,
}

impl Shared {
    fn tools(&self) -> Tools {
        self.tools.read().unwrap().clone()
    }

    fn enqueue(&self, id: i64, prio: Priority) {
        self.queue.push(id, prio);
        self.svc.live().busy = true;
    }

    fn dequeue(&self, id: i64) {
        self.queue.remove(id);
    }

    /// Queues every file the index has marked pending.
    fn enqueue_pending(&self) -> Result<()> {
        for id in self.svc.store().pending_files()? {
            self.enqueue(id, Priority::Backfill);
        }
        Ok(())
    }

    fn refresh_tools(&self) {
        let tools = Tools::detect();
        self.svc.live().messages = tools.messages();
        *self.tools.write().unwrap() = tools;
    }

    fn update_busy(&self) {
        let busy = !self.queue.idle();
        let changed = {
            let mut live = self.svc.live();
            let changed = live.busy != busy;
            live.busy = busy;
            if !busy {
                live.current = None;
            }
            changed
        };
        if changed {
            self.svc.publish_status();
        }
    }
}

/// How to run the indexer.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Watch the folders for changes (and rescan periodically) instead of one pass.
    pub watch: bool,
}

/// A running indexer.
pub struct Handle {
    shared: Arc<Shared>,
    tx: Sender<Msg>,
    threads: Vec<JoinHandle<()>>,
}

/// Starts indexing the configured folders in the background.
pub fn start(svc: Arc<Service>, opts: Options) -> Result<Handle> {
    let cfg = svc.config();
    let workers = match cfg.index.concurrency {
        0 => std::thread::available_parallelism()
            .map(|n| n.get() / 4)
            .unwrap_or(2)
            .clamp(2, 8),
        n => n as usize,
    };
    let (tx, rx) = mpsc::channel();
    let shared = Arc::new(Shared {
        svc: svc.clone(),
        queue: Queue::default(),
        tools: RwLock::new(Tools::detect()),
        coordinating: AtomicBool::new(true),
        batched: AtomicUsize::new(0),
        first_pass_done: AtomicBool::new(false),
    });
    shared.svc.live().messages = shared.tools().messages();
    if svc.is_paused() {
        shared.queue.set_paused(true);
    }
    let link = Mutex::new(tx.clone());
    svc.attach_indexer(move |cmd| {
        let _ = link.lock().unwrap().send(Msg::Cmd(cmd));
    });
    let mut threads = vec![];
    for i in 0..workers {
        let sh = shared.clone();
        threads.push(
            std::thread::Builder::new()
                .name(format!("rummage-worker-{i}"))
                .spawn(move || worker(sh))?,
        );
    }
    let watcher = if opts.watch {
        match Watcher::new(tx.clone()) {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!("file watching is unavailable ({e:#}); relying on rescans");
                None
            }
        }
    } else {
        None
    };
    let sh = shared.clone();
    let events = opts.watch.then(|| tx.clone());
    threads.push(
        std::thread::Builder::new()
            .name("rummage-indexer".into())
            .spawn(move || coordinator(sh, rx, events, watcher))?,
    );
    tracing::info!("indexer started with {workers} workers");
    Ok(Handle {
        shared,
        tx,
        threads,
    })
}

impl Handle {
    /// True once the first pass is done and nothing is queued, running or waiting to be handled.
    pub fn is_idle(&self) -> bool {
        let sh = &self.shared;
        sh.first_pass_done.load(Ordering::SeqCst)
            && !sh.coordinating.load(Ordering::SeqCst)
            && sh.batched.load(Ordering::SeqCst) == 0
            && sh.queue.idle()
    }

    /// Waits until [`Handle::is_idle`] (or the timeout passes). Returns whether it is idle.
    pub fn wait_idle(&self, timeout: Option<Duration>) -> bool {
        let started = Instant::now();
        loop {
            if self.is_idle() {
                return true;
            }
            if timeout.is_some_and(|t| started.elapsed() > t) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Stops the indexer; files in progress are left pending for next time.
    pub fn stop(self) {
        let _ = self.tx.send(Msg::Stop);
        self.shared.queue.stop();
        for t in self.threads {
            let _ = t.join();
        }
        self.shared.svc.detach_indexer();
    }
}

/// `events` is where a (re)created watcher sends file events; `None` for a single pass.
fn coordinator(
    sh: Arc<Shared>,
    rx: Receiver<Msg>,
    events: Option<Sender<Msg>>,
    mut watcher: Option<Watcher>,
) {
    let watch = events.is_some();
    let mut folders = FolderRules::all(&sh.svc.config());
    if let Err(e) = sh.enqueue_pending() {
        tracing::error!("reading pending files: {e:#}");
    }
    if let Err(e) = scan::scan(&sh, &folders, None, Priority::Backfill, watcher.as_mut()) {
        tracing::error!("scan failed: {e:#}");
    }
    if let Some(w) = &watcher {
        tracing::info!("watching {} directories", w.len());
    }
    sh.first_pass_done.store(true, Ordering::SeqCst);
    sh.coordinating.store(false, Ordering::SeqCst);
    sh.update_busy();
    sh.svc.publish_status();

    let rescan_every = |sh: &Shared| {
        let secs = sh.svc.config().index.scan_interval_secs;
        (watch && secs > 0).then(|| Duration::from_secs(secs))
    };
    let mut next_rescan = rescan_every(&sh).map(|d| Instant::now() + d);
    let mut batch: HashSet<PathBuf> = HashSet::new();
    let (mut first_event, mut last_event) = (None::<Instant>, Instant::now());
    loop {
        let now = Instant::now();
        let mut deadline = next_rescan;
        if let Some(first) = first_event {
            let due = (last_event + DEBOUNCE).min(first + DEBOUNCE_MAX);
            deadline = Some(deadline.map_or(due, |d| d.min(due)));
        }
        let msg = match deadline {
            Some(d) => rx.recv_timeout(d.saturating_duration_since(now)),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match msg {
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Fs(paths)) => {
                batch.extend(paths);
                sh.batched.store(batch.len(), Ordering::SeqCst);
                last_event = Instant::now();
                first_event.get_or_insert(last_event);
            }
            Ok(Msg::Cmd(cmd)) => {
                sh.coordinating.store(true, Ordering::SeqCst);
                let result = match cmd {
                    IndexCommand::Wake => sh.enqueue_pending(),
                    IndexCommand::Pause => {
                        sh.queue.set_paused(true);
                        Ok(())
                    }
                    IndexCommand::Resume => {
                        sh.queue.set_paused(false);
                        Ok(())
                    }
                    IndexCommand::Scan(only) => {
                        sh.refresh_tools();
                        scan::scan(
                            &sh,
                            &folders,
                            only.as_deref(),
                            Priority::Live,
                            watcher.as_mut(),
                        )
                    }
                    IndexCommand::ConfigChanged => {
                        folders = FolderRules::all(&sh.svc.config());
                        // Start over with fresh watches so removed folders stop being watched.
                        if let (Some(_), Some(tx)) = (&watcher, &events) {
                            watcher = Watcher::new(tx.clone()).ok();
                        }
                        next_rescan = rescan_every(&sh).map(|d| Instant::now() + d);
                        scan::scan(&sh, &folders, None, Priority::Backfill, watcher.as_mut())
                    }
                };
                if let Err(e) = result {
                    tracing::error!("indexer: {e:#}");
                }
                sh.coordinating.store(false, Ordering::SeqCst);
                sh.update_busy();
                sh.svc.publish_status();
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        let now = Instant::now();
        if let Some(first) = first_event
            && (now >= last_event + DEBOUNCE || now >= first + DEBOUNCE_MAX)
        {
            sh.coordinating.store(true, Ordering::SeqCst);
            let paths = std::mem::take(&mut batch);
            first_event = None;
            if let Err(e) = scan::apply_events(&sh, &folders, paths, watcher.as_mut()) {
                tracing::error!("handling file changes: {e:#}");
            }
            if let Some(w) = watcher.as_mut() {
                w.prune();
            }
            sh.batched.store(0, Ordering::SeqCst);
            sh.coordinating.store(false, Ordering::SeqCst);
            sh.update_busy();
        }
        if let Some(at) = next_rescan
            && now >= at
        {
            sh.coordinating.store(true, Ordering::SeqCst);
            sh.refresh_tools();
            if let Err(e) = scan::scan(&sh, &folders, None, Priority::Live, watcher.as_mut()) {
                tracing::error!("rescan failed: {e:#}");
            }
            sh.coordinating.store(false, Ordering::SeqCst);
            sh.update_busy();
            next_rescan = rescan_every(&sh).map(|d| Instant::now() + d);
        }
    }
}

fn worker(sh: Arc<Shared>) {
    while let Some(id) = sh.queue.pop() {
        if let Err(e) = process(&sh, id) {
            tracing::error!("indexing file {id}: {e:#}");
        }
        if sh.queue.finish() {
            sh.update_busy();
        }
    }
}

/// Indexes one file: extract, embed, store, render thumbnails, report.
fn process(sh: &Shared, id: i64) -> Result<()> {
    let svc = &sh.svc;
    let store = svc.store();
    let Some(row) = store.file(id)? else {
        return Ok(());
    };
    if row.status != "pending" {
        return Ok(());
    }
    let path = PathBuf::from(&row.path);
    let meta = match std::fs::metadata(&path) {
        Ok(m) if m.is_file() => m,
        _ => {
            svc.remove_file(id)?;
            return Ok(());
        }
    };
    svc.live().current = Some(row.path.clone());
    let cfg = svc.config();
    let tools = sh.tools();
    let ctx = Ctx {
        tools: &tools,
        thumb_size: cfg.index.thumbnail_size,
    };
    let modalities = svc.embedder().modalities();
    let mut chunks = vec![];
    let mut unreadable = 0usize;
    let mut stopped = false;
    let extracted = std::panic::catch_unwind(AssertUnwindSafe(|| {
        extract::extract(&path, row.kind, &ctx, &mut |piece| {
            if sh.queue.stopping() {
                stopped = true;
                bail!("stopping");
            }
            if !modalities.supports(&piece.input) {
                unreadable += 1;
                return Ok(());
            }
            let v = svc
                .embed_background(std::slice::from_ref(&piece.input))?
                .pop()
                .context("the embedder returned no vector")?;
            if v.iter().any(|x| !x.is_finite()) {
                bail!("the embedder returned NaN");
            }
            let mut chunk = piece.chunk;
            chunk.file_id = id;
            chunks.push((chunk, v));
            Ok(())
        })
    }))
    .unwrap_or_else(|panic| {
        let msg = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown".into());
        Err(anyhow!("the extractor crashed: {msg}"))
    });
    if stopped {
        return Ok(()); // stays pending for next time
    }
    let mut content = FileContent {
        size: meta.len() as i64,
        mtime: scan::mtime(&meta),
        hash: hash_file(&path).ok(),
        mime: row.mime.clone(),
        ..Default::default()
    };
    let mut renders = None;
    match extracted {
        Ok(Outcome::Done(m)) if chunks.is_empty() && unreadable > 0 => {
            content.status = "skipped".into();
            content.error = Some("the embedding model can't read this kind of content".into());
            content.date = m.date;
        }
        Ok(Outcome::Done(m)) => {
            content.status = "indexed".into();
            content.date = m.date;
            content.exif = m.exif.as_ref().map(|e| e.to_string());
            renders = Some(m);
        }
        Ok(Outcome::Skipped(why)) => {
            chunks.clear();
            content.status = "skipped".into();
            content.error = Some(why);
        }
        Err(e) => {
            chunks.clear();
            content.status = "error".into();
            content.error = Some(format!("{e:#}"));
        }
    }
    let status = FileStatus::parse(&content.status);
    let error = content.error.clone();
    if svc.store_file(id, &content, chunks)?.is_none() {
        return Ok(()); // removed meanwhile
    }
    svc.clear_renders(id);
    if let Some(m) = renders {
        let write = |dir: &str, variant: &str, bytes: &[u8]| {
            let p = svc.cache_path(dir, id, variant);
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&p, bytes) {
                tracing::warn!("writing {}: {e}", p.display());
            }
        };
        if let Some(t) = &m.thumb {
            write("thumbs", &cfg.index.thumbnail_size.to_string(), t);
        }
        for (variant, bytes) in &m.previews {
            write("previews", variant, bytes);
        }
        if let Some((duration, peaks)) = &m.peaks {
            let json = serde_json::json!({ "duration": duration, "peaks": peaks });
            let p = render::peaks_path(svc, id);
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&p, json.to_string()) {
                tracing::warn!("writing {}: {e}", p.display());
            }
        }
    }
    if let Some(e) = &error {
        tracing::debug!("{}: {e}", row.path);
    }
    svc.file_done(IndexEvent::File {
        id,
        path: row.path,
        kind: row.kind,
        status,
        error,
    });
    Ok(())
}

/// Indexes new and changed files in every enabled folder once, then returns.
pub fn run_once(svc: Arc<Service>) -> Result<()> {
    let started = Instant::now();
    let handle = start(svc.clone(), Options::default())?;
    let mut last = String::new();
    while !handle.wait_idle(Some(Duration::from_secs(2))) {
        let s = svc.index_status()?;
        let line = format!(
            "{} of {} files indexed, {} waiting{}",
            s.totals.indexed,
            s.totals.files,
            s.queued,
            s.eta_secs
                .map(|e| format!(", about {} left", human_secs(e)))
                .unwrap_or_default()
        );
        if line != last {
            eprintln!("{line}");
            last = line;
        }
    }
    handle.stop();
    let s = svc.index_status()?;
    eprintln!(
        "done in {}: {} files indexed, {} errors, {} skipped",
        human_secs(started.elapsed().as_secs()),
        s.totals.indexed,
        s.totals.errors,
        s.totals.skipped
    );
    for m in &s.messages {
        eprintln!("note: {m}");
    }
    Ok(())
}

/// Indexes, then keeps watching the folders for changes until Ctrl-C.
pub async fn watch(svc: Arc<Service>) -> Result<()> {
    let handle = start(svc, Options { watch: true })?;
    let _ = tokio::signal::ctrl_c().await;
    tokio::task::spawn_blocking(move || handle.stop()).await?;
    Ok(())
}

pub fn human_secs(s: u64) -> String {
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", (s + 30) / 60),
        _ => format!("{}h {:02}m", s / 3600, (s % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_orders_by_priority_and_dedupes() {
        let q = Queue::default();
        q.push(1, Priority::Backfill);
        q.push(2, Priority::Backfill);
        q.push(3, Priority::Live);
        q.push(2, Priority::Live); // promoted
        q.push(3, Priority::Live); // already queued
        assert_eq!(q.pop(), Some(3));
        assert_eq!(q.pop(), Some(2));
        q.remove(1);
        assert!(!q.finish());
        assert!(q.finish());
        assert!(q.idle());
        q.push(4, Priority::Live);
        q.stop();
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn human_durations() {
        assert_eq!(human_secs(5), "5 s");
        assert_eq!(human_secs(61), "1 min");
        assert_eq!(human_secs(100), "2 min");
        assert_eq!(human_secs(3700), "1h 01m");
    }
}
