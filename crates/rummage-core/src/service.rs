//! What the API and the CLI do, independent of HTTP. The server wraps a [`Service`] in axum
//! handlers; the CLI calls it directly when no server is running.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use anyhow::Context;
use tokio::sync::broadcast;

use crate::config::{Config, ConfigFile, FolderConfig, expand_home, needs_restart};
use crate::embed::{EmbedInput, EmbedQueue, Embedder, ImageData, dot, fnv1a, normalize};
use crate::store::{ChunkRow, FileContent, FileRow, Scope, Store};
use crate::types::*;
use crate::vindex::VectorIndex;

/// An error with an HTTP-friendly category.
#[derive(Debug)]
pub enum Error {
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    Unsupported(String),
    NotReady(String),
    Internal(anyhow::Error),
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Error::BadRequest(_) => "bad_request",
            Error::NotFound(_) => "not_found",
            Error::Conflict(_) => "conflict",
            Error::Unsupported(_) => "unsupported",
            Error::NotReady(_) => "not_ready",
            Error::Internal(_) => "internal",
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BadRequest(m)
            | Error::NotFound(m)
            | Error::Conflict(m)
            | Error::Unsupported(m)
            | Error::NotReady(m) => f.write_str(m),
            Error::Internal(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Error::Internal(e)
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 500;
/// Reciprocal-rank-fusion constant.
const RRF_K: f32 = 60.0;

/// Stable id of a folder, derived from its path.
pub fn folder_id(path: &str) -> String {
    format!("{:012x}", fnv1a(path.as_bytes()) >> 16)
}

/// A ranked candidate file.
struct Fused {
    score: f32,
    chunk: i64,
    similarity: Option<f32>,
    keyword: bool,
}

/// What the service asks of a running indexer.
#[derive(Debug, Clone, PartialEq)]
pub enum IndexCommand {
    /// Files were marked pending: pick them up.
    Wake,
    /// Walk one folder (by path) or all of them for changes.
    Scan(Option<String>),
    /// The folder list or the index settings changed.
    ConfigChanged,
    Pause,
    Resume,
}

/// Turns files into searchable content for search-by-file uploads (decoding images and audio
/// the same way the indexer does). Returns `None` for kinds it doesn't handle.
pub trait UploadDecoder: Send + Sync {
    fn decode(
        &self,
        name: Option<&str>,
        mime: &str,
        bytes: &[u8],
    ) -> Option<anyhow::Result<EmbedInput>>;
}

/// What a running indexer reports, for `/index/status`.
#[derive(Default)]
pub struct Live {
    /// True while an indexer runs in this process.
    pub attached: bool,
    /// Folders (paths) being walked.
    pub scanning: HashSet<String>,
    /// True while files are queued or being worked on.
    pub busy: bool,
    /// The file most recently started.
    pub current: Option<String>,
    /// Notes for the user, e.g. a missing ffmpeg.
    pub messages: Vec<String>,
    done: VecDeque<Instant>,
    last_status: Option<Instant>,
}

impl Live {
    /// Files per second over the last minute, once there is enough to measure.
    fn rate(&mut self) -> Option<f64> {
        let now = Instant::now();
        while self
            .done
            .front()
            .is_some_and(|t| now.duration_since(*t) > RATE_WINDOW)
        {
            self.done.pop_front();
        }
        let (first, last) = (self.done.front()?, self.done.back()?);
        let span = last.duration_since(*first).as_secs_f64();
        (self.done.len() >= 2 && span > 0.0).then(|| (self.done.len() - 1) as f64 / span)
    }
}

const RATE_WINDOW: Duration = Duration::from_secs(60);
const MODEL_LOADING: &str =
    "the embedding model is still loading (it downloads on first start); try again shortly";
/// Least time between two status events pushed while indexing.
const STATUS_EVERY: Duration = Duration::from_millis(500);

type IndexLink = Box<dyn Fn(IndexCommand) + Send + Sync>;

pub struct Service {
    config: RwLock<ConfigFile>,
    started_config: Config,
    store: Store,
    embeds: EmbedQueue,
    events: broadcast::Sender<IndexEvent>,
    /// Loaded on first search, then updated as files are indexed; with the store's
    /// `data_version` at load time (another process writing forces a reload).
    vectors: RwLock<Option<(VectorIndex, i64)>>,
    live: Mutex<Live>,
    paused: AtomicBool,
    link: RwLock<Option<IndexLink>>,
    decoder: RwLock<Option<Arc<dyn UploadDecoder>>>,
    started: Instant,
}

impl Service {
    /// Opens the store named by the config.
    pub fn open(config: ConfigFile, embedder: Arc<dyn Embedder>) -> anyhow::Result<Self> {
        let store = Store::open(&config.config().db_path())?;
        Ok(Self::new(config, store, embedder))
    }

    pub fn new(config: ConfigFile, store: Store, embedder: Arc<dyn Embedder>) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            started_config: config.config().clone(),
            config: RwLock::new(config),
            store,
            embeds: EmbedQueue::new(embedder),
            events,
            vectors: RwLock::new(None),
            live: Mutex::new(Live::default()),
            paused: AtomicBool::new(false),
            link: RwLock::new(None),
            decoder: RwLock::new(None),
            started: Instant::now(),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn embedder(&self) -> &Arc<dyn Embedder> {
        self.embeds.embedder()
    }

    /// Embeds for the indexer; searches go first.
    pub fn embed_background(&self, inputs: &[EmbedInput]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.embeds.background(inputs)
    }

    /// Sets how uploaded files are decoded for search-by-file.
    pub fn set_decoder(&self, decoder: Arc<dyn UploadDecoder>) {
        *self.decoder.write().unwrap() = Some(decoder);
    }

    // ---- the indexer's side ----

    /// Connects a running indexer: the service sends it commands through `link`.
    pub fn attach_indexer(&self, link: impl Fn(IndexCommand) + Send + Sync + 'static) {
        *self.link.write().unwrap() = Some(Box::new(link));
        self.live().attached = true;
    }

    pub fn detach_indexer(&self) {
        *self.link.write().unwrap() = None;
        let mut live = self.live();
        live.attached = false;
        live.busy = false;
        live.scanning.clear();
        live.current = None;
    }

    fn command(&self, cmd: IndexCommand) {
        if let Some(link) = self.link.read().unwrap().as_ref() {
            link(cmd);
        }
    }

    /// The indexer's live state.
    pub fn live(&self) -> MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Records a finished (or failed) file and tells subscribers.
    pub fn file_done(&self, event: IndexEvent) {
        self.live().done.push_back(Instant::now());
        self.publish(event);
        self.publish_status_soon();
    }

    /// Stores a file's extracted content and vectors, and makes them searchable.
    /// Returns the chunk ids, or `None` when the file left the index meanwhile.
    pub fn store_file(
        &self,
        file_id: i64,
        content: &FileContent,
        chunks: Vec<(ChunkRow, Vec<f32>)>,
    ) -> anyhow::Result<Option<Vec<i64>>> {
        let Some(ids) = self.store.replace_content(file_id, content, &chunks)? else {
            return Ok(None);
        };
        if let Some((ix, _)) = self.vectors.write().unwrap().as_mut() {
            let rows: Vec<(i64, SegmentKind, Vec<f32>)> = ids
                .iter()
                .zip(chunks)
                .map(|(id, (c, v))| (*id, c.kind, v))
                .collect();
            ix.set_file(file_id, &rows);
        }
        Ok(Some(ids))
    }

    /// Drops a file from the index, with its thumbnails and previews.
    pub fn remove_file(&self, file_id: i64) -> anyhow::Result<()> {
        self.store.delete_file(file_id)?;
        if let Some((ix, _)) = self.vectors.write().unwrap().as_mut() {
            ix.remove_file(file_id);
        }
        self.clear_renders(file_id);
        Ok(())
    }

    /// Deletes the cached thumbnails and previews of a file.
    pub fn clear_renders(&self, file_id: i64) {
        let data = self.config().data_dir();
        for dir in ["thumbs", "previews"] {
            let _ = std::fs::remove_dir_all(data.join(dir).join(file_id.to_string()));
        }
    }

    /// Pauses indexing (searches keep working).
    pub fn pause(&self) -> Result<IndexStatus> {
        self.paused.store(true, Ordering::Relaxed);
        self.command(IndexCommand::Pause);
        self.publish_status();
        self.index_status()
    }

    pub fn resume(&self) -> Result<IndexStatus> {
        self.paused.store(false, Ordering::Relaxed);
        self.command(IndexCommand::Resume);
        self.publish_status();
        self.index_status()
    }

    /// Asks the indexer to walk one folder (or all) for changes now.
    pub fn scan(&self, req: &ScanRequest) -> Result<()> {
        let folder = match req.folder.as_deref().filter(|f| !f.trim().is_empty()) {
            Some(id) => Some(self.folder(id)?.path),
            None => None,
        };
        if !self.live().attached {
            return Err(Error::NotReady(
                "no indexer is running here; start `rummage serve` or run `rummage index`".into(),
            ));
        }
        self.command(IndexCommand::Scan(folder));
        Ok(())
    }

    /// Files that failed to index or were skipped.
    pub fn problems(&self, params: &PageParams, prefix: &str) -> Result<ProblemList> {
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let (total, rows) = self
            .store
            .problem_files(limit as usize, params.offset as usize)?;
        Ok(ProblemList {
            total,
            items: rows.iter().map(|f| self.item_view(f, prefix)).collect(),
        })
    }

    /// A snapshot of the effective config.
    pub fn config(&self) -> Config {
        self.config.read().unwrap().config().clone()
    }

    /// Live index events (progress, finished files).
    pub fn subscribe(&self) -> broadcast::Receiver<IndexEvent> {
        self.events.subscribe()
    }

    /// Sends an event to every subscriber. The indexer calls this.
    pub fn publish(&self, event: IndexEvent) {
        let _ = self.events.send(event);
    }

    pub fn health(&self) -> Result<Health> {
        Ok(Health {
            status: "ok".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            schema_version: self.store.schema_version()?,
            embedder: self.embedder_info(),
            uptime_secs: self.started.elapsed().as_secs(),
            auth_required: !self.config().server.auth_token.is_empty(),
        })
    }

    pub fn embedder_info(&self) -> EmbedderInfo {
        EmbedderInfo {
            name: self.embedder().name(),
            dim: self.embedder().dim() as u32,
            modalities: self.embedder().modalities(),
        }
    }

    // ---- folders ----

    pub fn folders(&self) -> Result<Vec<Folder>> {
        let cf = self.config.read().unwrap();
        let from_env = cf.env_overrides().iter().any(|k| k == "folders");
        cf.config()
            .folders
            .iter()
            .map(|f| self.folder_view(f, from_env))
            .collect()
    }

    pub fn folder(&self, id: &str) -> Result<Folder> {
        self.folders()?
            .into_iter()
            .find(|f| f.id == id || f.path == id)
            .ok_or_else(|| Error::NotFound(format!("no folder {id}")))
    }

    fn folder_view(&self, f: &FolderConfig, from_env: bool) -> Result<Folder> {
        let path = expand_home(&f.path);
        let path_str = path.to_string_lossy().into_owned();
        Ok(Folder {
            id: folder_id(&path_str),
            available: std::fs::read_dir(&path).is_ok(),
            counts: self.store.counts(Some(&path_str))?.into(),
            path: path_str,
            include: f.include.clone(),
            exclude: f.exclude.clone(),
            enabled: f.enabled,
            from_env,
        })
    }

    /// Edits the folder list in the config file.
    fn edit_folders(&self, edit: impl FnOnce(&mut Vec<FolderConfig>) -> Result<()>) -> Result<()> {
        let mut cf = self.config.write().unwrap();
        if cf.env_overrides().iter().any(|k| k == "folders") {
            return Err(Error::Conflict(
                "folders are set by RUMMAGE_FOLDERS; change that variable instead".into(),
            ));
        }
        let mut folders = cf.file_config()?.folders;
        edit(&mut folders)?;
        cf.set_folders(&folders)
            .map_err(|e| Error::BadRequest(format!("{e:#}")))
    }

    pub fn add_folder(&self, new: NewFolder) -> Result<Folder> {
        let path = expand_home(new.path.trim());
        if !path.is_absolute() {
            return Err(Error::BadRequest(format!(
                "folder path must be absolute: {}",
                new.path
            )));
        }
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if !path.is_dir() {
            return Err(Error::BadRequest(format!(
                "not a directory: {}",
                path.display()
            )));
        }
        let path_str = path.to_string_lossy().into_owned();
        self.edit_folders(|folders| {
            if folders
                .iter()
                .any(|f| expand_home(&f.path) == Path::new(&path_str))
            {
                return Err(Error::Conflict(format!("already watched: {path_str}")));
            }
            folders.push(FolderConfig {
                path: path_str.clone(),
                include: new.include,
                exclude: new.exclude,
                enabled: new.enabled,
            });
            Ok(())
        })?;
        self.command(IndexCommand::ConfigChanged);
        self.folder(&folder_id(&path_str))
    }

    pub fn update_folder(&self, id: &str, patch: FolderPatch) -> Result<Folder> {
        let target = self.folder(id)?;
        self.edit_folders(|folders| {
            let f = folders
                .iter_mut()
                .find(|f| expand_home(&f.path) == Path::new(&target.path))
                .ok_or_else(|| Error::NotFound(format!("no folder {id}")))?;
            if let Some(v) = patch.include {
                f.include = v;
            }
            if let Some(v) = patch.exclude {
                f.exclude = v;
            }
            if let Some(v) = patch.enabled {
                f.enabled = v;
            }
            Ok(())
        })?;
        self.command(IndexCommand::ConfigChanged);
        self.folder(&target.id)
    }

    /// Stops watching a folder and drops its files from the index.
    pub fn remove_folder(&self, id: &str) -> Result<()> {
        let target = self.folder(id)?;
        self.edit_folders(|folders| {
            folders.retain(|f| expand_home(&f.path) != Path::new(&target.path));
            Ok(())
        })?;
        self.command(IndexCommand::ConfigChanged);
        let ids = self.store.folder_file_ids(&target.path)?;
        self.store.delete_folder(&target.path)?;
        if let Some((ix, _)) = self.vectors.write().unwrap().as_mut() {
            ids.iter().for_each(|id| ix.remove_file(*id));
        }
        ids.iter().for_each(|id| self.clear_renders(*id));
        self.publish_status();
        Ok(())
    }

    /// Sub-directories of `path` (default: the home directory), for picking a folder.
    pub fn list_dirs(&self, path: Option<&str>, hidden: bool) -> Result<DirListing> {
        let raw = path.map(str::trim).filter(|p| !p.is_empty()).unwrap_or("~");
        let path = expand_home(raw);
        if !path.is_absolute() {
            return Err(Error::BadRequest(format!("path must be absolute: {raw}")));
        }
        let path = std::fs::canonicalize(&path)
            .map_err(|e| Error::NotFound(format!("{}: {e}", path.display())))?;
        let entries = std::fs::read_dir(&path)
            .map_err(|e| Error::BadRequest(format!("can't read {}: {e}", path.display())))?;
        let watched: HashSet<String> = self.folders()?.into_iter().map(|f| f.path).collect();
        let mut dirs: Vec<DirEntry> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if !hidden && name.starts_with('.') {
                    return None;
                }
                let p = e.path().to_string_lossy().into_owned();
                Some(DirEntry {
                    watched: watched.contains(&p),
                    name,
                    path: p,
                })
            })
            .collect();
        dirs.sort_by_key(|d| d.name.to_lowercase());
        Ok(DirListing {
            path: path.to_string_lossy().into_owned(),
            parent: path.parent().map(|p| p.to_string_lossy().into_owned()),
            dirs,
        })
    }

    // ---- index ----

    pub fn index_status(&self) -> Result<IndexStatus> {
        let totals: IndexCounts = self.store.counts(None)?.into();
        let folders = self.folders()?;
        let by_kind = self.store.kind_counts()?;
        let mut live = self.live();
        let rate = live.rate();
        let eta = |pending: u64| {
            rate.filter(|_| pending > 0)
                .map(|r| (pending as f64 / r).ceil() as u64)
        };
        let state = if self.is_paused() {
            IndexState::Paused
        } else if !live.scanning.is_empty() {
            IndexState::Scanning
        } else if live.busy {
            IndexState::Indexing
        } else {
            IndexState::Idle
        };
        Ok(IndexStatus {
            state,
            queued: totals.pending,
            files_per_min: rate.map(|r| r * 60.0),
            eta_secs: eta(totals.pending),
            totals,
            by_kind: by_kind
                .into_iter()
                .map(|(kind, files)| KindCount { kind, files })
                .collect(),
            folders: folders
                .into_iter()
                .map(|f| FolderProgress {
                    scanning: live.scanning.contains(&f.path),
                    eta_secs: eta(f.counts.pending),
                    id: f.id,
                    path: f.path,
                    counts: f.counts,
                })
                .collect(),
            current: live
                .current
                .clone()
                .filter(|_| state == IndexState::Indexing),
            messages: {
                let mut m = live.messages.clone();
                if !self.embedder().ready() {
                    m.push(
                        "The embedding model is not ready: it is loading (it downloads on \
                         first start) or failed to load (see the log); until then searches match \
                         words only and indexing waits."
                            .into(),
                    );
                }
                m
            },
        })
    }

    /// Pushes a status snapshot to subscribers.
    pub fn publish_status(&self) {
        if self.events.receiver_count() > 0
            && let Ok(s) = self.index_status()
        {
            self.live().last_status = Some(Instant::now());
            self.publish(IndexEvent::Status(s));
        }
    }

    /// Like [`Service::publish_status`], at most every [`STATUS_EVERY`].
    pub fn publish_status_soon(&self) {
        let due = self
            .live()
            .last_status
            .is_none_or(|t| t.elapsed() >= STATUS_EVERY);
        if due {
            self.publish_status();
        }
    }

    /// Queues files for re-indexing.
    pub fn reindex(&self, req: &ReindexRequest) -> Result<ReindexResponse> {
        let queued = match req {
            ReindexRequest::All => self.store.mark_pending(Scope::All)?,
            ReindexRequest::Folder { id } => {
                let f = self.folder(id)?;
                self.store.mark_pending(Scope::Folder(&f.path))?
            }
            ReindexRequest::Item { id } => {
                self.file_row(*id)?;
                self.store.mark_pending(Scope::File(*id))?
            }
            ReindexRequest::Problems => self.store.mark_pending(Scope::Problems)?,
        };
        self.command(IndexCommand::Wake);
        self.publish_status();
        Ok(ReindexResponse {
            queued: queued as u64,
        })
    }

    // ---- settings ----

    pub fn settings(&self) -> SettingsView {
        let cf = self.config.read().unwrap();
        self.settings_view(&cf)
    }

    fn settings_view(&self, cf: &ConfigFile) -> SettingsView {
        let now = serde_json::to_value(cf.config()).unwrap_or_default();
        let then = serde_json::to_value(&self.started_config).unwrap_or_default();
        let mut restart_required: Vec<String> = ["data_dir", "server", "model", "engine", "index"]
            .iter()
            .flat_map(|section| {
                let (a, b) = (&now[section], &then[section]);
                match (a, b) {
                    (serde_json::Value::Object(x), serde_json::Value::Object(_)) => x
                        .keys()
                        .filter(|k| a[k.as_str()] != b[k.as_str()])
                        .map(|k| format!("{section}.{k}"))
                        .collect::<Vec<_>>(),
                    _ if a != b => vec![section.to_string()],
                    _ => vec![],
                }
            })
            .filter(|k| needs_restart(k))
            .collect();
        restart_required.sort();
        SettingsView {
            settings: cf.config().redacted(),
            config_path: cf.path().to_string_lossy().into_owned(),
            env_overrides: cf.env_overrides().to_vec(),
            restart_required,
        }
    }

    /// Applies a JSON merge patch to the config file.
    pub fn patch_settings(&self, patch: &serde_json::Value) -> Result<SettingsView> {
        let mut cf = self.config.write().unwrap();
        let changed = cf
            .apply_patch(patch)
            .map_err(|e| Error::BadRequest(format!("{e:#}")))?;
        let view = self.settings_view(&cf);
        drop(cf);
        if changed
            .iter()
            .any(|k| k.starts_with("folders") || k.starts_with("index."))
        {
            self.command(IndexCommand::ConfigChanged);
        }
        Ok(view)
    }

    // ---- items ----

    fn file_row(&self, id: i64) -> Result<FileRow> {
        self.store
            .file(id)?
            .ok_or_else(|| Error::NotFound(format!("no item {id}")))
    }

    /// Builds the API view of a file; `prefix` is the URL base path.
    pub fn item_view(&self, f: &FileRow, prefix: &str) -> Item {
        let base = format!("{prefix}/api/v1/items/{}", f.id);
        Item {
            id: f.id,
            path: f.path.clone(),
            name: f.name.clone(),
            folder_id: folder_id(&f.folder),
            kind: f.kind,
            mime: f.mime.clone(),
            size: f.size.max(0) as u64,
            mtime: f.mtime,
            date: f.date.unwrap_or(f.mtime),
            status: FileStatus::parse(&f.status),
            error: f.error.clone(),
            exif: f.exif.as_deref().and_then(|e| serde_json::from_str(e).ok()),
            urls: ItemUrls {
                thumb: format!("{base}/thumb"),
                preview: format!("{base}/preview"),
                raw: format!("{base}/raw"),
            },
        }
    }

    pub fn item(&self, id: i64, q: Option<&str>, prefix: &str) -> Result<ItemDetail> {
        let f = self.file_row(id)?;
        let mut segments: Vec<Segment> = self
            .store
            .chunks(id)?
            .into_iter()
            .map(|c| Segment {
                id: c.id,
                kind: c.kind,
                page: c.page,
                t_start: c.t_start,
                t_end: c.t_end,
                text: c.text,
                score: None,
                snippet: vec![],
            })
            .collect();
        if let Some(q) = q.filter(|q| !q.trim().is_empty())
            && self.embedder().ready()
        {
            let qv = self.embed_one(EmbedInput::Query(q.into()))?;
            let scores: HashMap<i64, f32> = self
                .store
                .vectors_for_file(id)?
                .into_iter()
                .filter(|(_, v)| v.len() <= qv.len())
                .map(|(cid, v)| (cid, dot(&v, &truncated(&qv, v.len()))))
                .collect();
            for s in &mut segments {
                s.score = scores.get(&s.id).copied();
            }
            segments.sort_by(|a, b| b.score.unwrap_or(-2.0).total_cmp(&a.score.unwrap_or(-2.0)));
        }
        Ok(ItemDetail {
            item: self.item_view(&f, prefix),
            segments,
        })
    }

    /// Path of the item's file on disk.
    pub fn item_path(&self, id: i64) -> Result<(String, Option<String>)> {
        let f = self.file_row(id)?;
        Ok((f.path, f.mime))
    }

    /// Where a cached thumbnail or preview of `id` is kept (`<data_dir>/<dir>/<id>/<variant>.webp`).
    pub fn cache_path(&self, dir: &str, id: i64, variant: &str) -> PathBuf {
        self.config()
            .data_dir()
            .join(dir)
            .join(id.to_string())
            .join(format!("{variant}.webp"))
    }

    pub fn item_kind(&self, id: i64) -> Result<Kind> {
        Ok(self.file_row(id)?.kind)
    }

    pub fn similar(&self, id: i64, params: &SimilarParams, prefix: &str) -> Result<SearchResponse> {
        let types = params
            .types
            .as_deref()
            .map(parse_kinds)
            .transpose()?
            .unwrap_or_default();
        self.search(
            SearchRequest {
                item_id: Some(id),
                limit: params.limit,
                offset: params.offset,
                mode: SearchMode::Vector,
                filters: Filters {
                    types,
                    ..Default::default()
                },
                ..Default::default()
            },
            prefix,
        )
    }

    // ---- search ----

    fn embed_one(&self, input: EmbedInput) -> Result<Vec<f32>> {
        if !self.embedder().modalities().supports(&input) {
            return Err(Error::Unsupported(
                "the embedding model can't read this kind of input".into(),
            ));
        }
        let mut v = self
            .embeds
            .query(std::slice::from_ref(&input))?
            .pop()
            .context("embedder returned nothing")?;
        normalize(&mut v);
        Ok(v)
    }

    /// Embeds an uploaded file for search-by-example.
    fn embed_upload(
        &self,
        name: Option<&str>,
        mime: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<Vec<f32>> {
        let mime = mime
            .map(str::to_string)
            .or_else(|| name.and_then(|n| mime_guess::from_path(n).first_raw().map(String::from)))
            .unwrap_or_default();
        let decoder = self.decoder.read().unwrap().clone();
        if let Some(input) = decoder.and_then(|d| d.decode(name, &mime, &bytes)) {
            let input = input
                .map_err(|e| Error::BadRequest(format!("can't read the uploaded file: {e:#}")))?;
            return self.embed_one(input);
        }
        if mime.starts_with("image/") {
            self.embed_one(EmbedInput::Image(ImageData::Encoded(bytes)))
        } else if mime.starts_with("text/")
            || mime.is_empty() && std::str::from_utf8(&bytes).is_ok()
        {
            let text = String::from_utf8(bytes)
                .map_err(|_| Error::BadRequest("text upload is not UTF-8".into()))?;
            self.embed_one(EmbedInput::Document {
                title: name.map(String::from),
                text,
            })
        } else {
            Err(Error::Unsupported(format!(
                "searching by {} files is not supported; use an image, audio or text file",
                if mime.is_empty() {
                    "this kind of"
                } else {
                    &mime
                }
            )))
        }
    }

    fn query_vector(&self, req: &SearchRequest) -> Result<Option<Vec<f32>>> {
        let given = req.query.as_deref().is_some_and(|q| !q.trim().is_empty()) as u8
            + req.item_id.is_some() as u8
            + req.file.is_some() as u8;
        if given != 1 {
            return Err(Error::BadRequest(
                "give exactly one of query, item_id or file".into(),
            ));
        }
        if req.mode == SearchMode::Keyword {
            return Ok(None);
        }
        let ready = self.embedder().ready();
        if let Some(q) = &req.query {
            if !ready {
                // Don't make a search wait for the model to load: match words for now.
                if req.mode == SearchMode::Hybrid {
                    return Ok(None);
                }
                return Err(Error::NotReady(MODEL_LOADING.into()));
            }
            return self.embed_one(EmbedInput::Query(q.clone())).map(Some);
        }
        if let Some(id) = req.item_id {
            self.file_row(id)?;
            let vs = self.store.vectors_for_file(id)?;
            if vs.is_empty() {
                return Err(Error::NotReady(format!("item {id} is not indexed yet")));
            }
            let dim = vs[0].1.len();
            let mut mean = vec![0f32; dim];
            for (_, v) in vs.iter().filter(|(_, v)| v.len() == dim) {
                mean.iter_mut().zip(v).for_each(|(a, b)| *a += b);
            }
            normalize(&mut mean);
            return Ok(Some(mean));
        }
        let file = req.file.as_ref().expect("checked above");
        if !ready {
            return Err(Error::NotReady(MODEL_LOADING.into()));
        }
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(file.data_base64.trim())
            .map_err(|e| Error::BadRequest(format!("file.data_base64: {e}")))?;
        self.embed_upload(file.name.as_deref(), file.mime.as_deref(), bytes)
            .map(Some)
    }

    /// Searches with an uploaded file given as raw bytes (the multipart form of `POST /search`).
    pub fn search_upload(
        &self,
        mut req: SearchRequest,
        name: Option<String>,
        mime: Option<String>,
        bytes: Vec<u8>,
        prefix: &str,
    ) -> Result<SearchResponse> {
        if !self.embedder().ready() {
            return Err(Error::NotReady(MODEL_LOADING.into()));
        }
        let qv = self.embed_upload(name.as_deref(), mime.as_deref(), bytes)?;
        req.query = None;
        req.item_id = None;
        req.file = None;
        if req.mode == SearchMode::Keyword {
            req.mode = SearchMode::Vector;
        }
        self.run_search(&req, Some(qv), None, prefix)
    }

    pub fn search(&self, req: SearchRequest, prefix: &str) -> Result<SearchResponse> {
        let qv = self.query_vector(&req)?;
        let exclude = req.item_id;
        if req.query.is_none() && req.mode == SearchMode::Keyword {
            return Err(Error::BadRequest("keyword mode needs a text query".into()));
        }
        self.run_search(&req, qv, exclude, prefix)
    }

    fn run_search(
        &self,
        req: &SearchRequest,
        qv: Option<Vec<f32>>,
        exclude: Option<i64>,
        prefix: &str,
    ) -> Result<SearchResponse> {
        let started = Instant::now();
        let limit = req.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let ranked = self.rank(req, &req.filters, qv.as_deref(), exclude, limit)?;
        let total = ranked.len() as u64;
        let kinds = if req.filters.types.is_empty() {
            self.kind_counts(&ranked)?
        } else {
            let all_kinds = Filters {
                types: vec![],
                ..req.filters.clone()
            };
            self.kind_counts(&self.rank(req, &all_kinds, qv.as_deref(), exclude, limit)?)?
        };

        let page: Vec<&(i64, Fused)> = ranked
            .iter()
            .skip(req.offset as usize)
            .take(limit as usize)
            .collect();
        let files: HashMap<i64, FileRow> = self
            .store
            .files(&page.iter().map(|(id, _)| *id).collect::<Vec<_>>())?
            .into_iter()
            .map(|f| (f.id, f))
            .collect();
        let words = req.query.as_deref().map(query_words).unwrap_or_default();
        let mut hits = Vec::with_capacity(page.len());
        for (file_id, f) in page {
            let Some(row) = files.get(file_id) else {
                continue;
            };
            let segment = self.store.chunk(f.chunk)?.map(|c| {
                let snippet = c
                    .text
                    .as_deref()
                    .map(|t| snippet(t, &words, SNIPPET_CHARS))
                    .unwrap_or_default();
                Segment {
                    id: c.id,
                    kind: c.kind,
                    page: c.page,
                    t_start: c.t_start,
                    t_end: c.t_end,
                    text: c
                        .text
                        .as_ref()
                        .map(|_| snippet.iter().map(|s| s.text.as_str()).collect()),
                    score: f.similarity,
                    snippet,
                }
            });
            hits.push(SearchHit {
                item: self.item_view(row, prefix),
                score: f.score,
                similarity: f.similarity,
                keyword_match: f.keyword,
                segment,
            });
        }
        Ok(SearchResponse {
            hits,
            total,
            offset: req.offset,
            limit,
            took_ms: started.elapsed().as_secs_f64() * 1000.0,
            kinds,
        })
    }

    /// Every candidate file passing `filters`, best first.
    fn rank(
        &self,
        req: &SearchRequest,
        filters: &Filters,
        qv: Option<&[f32]>,
        exclude: Option<i64>,
        limit: u32,
    ) -> Result<Vec<(i64, Fused)>> {
        let want = (req.offset + limit) as usize;
        let pool = (want * 4).max(200);
        let allowed = self.allowed_files(filters)?;
        let ok = |file_id: i64| {
            Some(file_id) != exclude && allowed.as_ref().is_none_or(|a| a.contains(&file_id))
        };

        // Best chunk per file from the vector scan, ranked.
        let mut vector_hits: Vec<(i64, i64, f32)> = vec![];
        if let Some(qv) = &qv
            && req.mode != SearchMode::Keyword
        {
            vector_hits = self
                .with_vectors(|ix| ix.search(qv, pool, ok))?
                .into_iter()
                .map(|h| (h.file_id, h.chunk_id, h.score))
                .collect();
        }

        // Best chunk per file from the keyword index, ranked.
        let mut keyword_hits: Vec<(i64, i64)> = vec![];
        if let Some(q) = req.query.as_deref()
            && req.mode != SearchMode::Vector
        {
            let mut seen = HashSet::new();
            for (chunk, file) in self.store.keyword_search(q, pool * 4)? {
                if ok(file) && seen.insert(file) {
                    keyword_hits.push((file, chunk));
                }
            }
            keyword_hits.truncate(pool);
        }

        // Reciprocal-rank fusion.
        let mut fused: HashMap<i64, Fused> = HashMap::new();
        for (rank, (file, chunk, sim)) in vector_hits.iter().enumerate() {
            fused.insert(
                *file,
                Fused {
                    score: 1.0 / (RRF_K + rank as f32 + 1.0),
                    chunk: *chunk,
                    similarity: Some(*sim),
                    keyword: false,
                },
            );
        }
        for (rank, (file, chunk)) in keyword_hits.iter().enumerate() {
            let s = 1.0 / (RRF_K + rank as f32 + 1.0);
            fused
                .entry(*file)
                .and_modify(|f| {
                    f.score += s;
                    f.keyword = true;
                })
                .or_insert(Fused {
                    score: s,
                    chunk: *chunk,
                    similarity: None,
                    keyword: true,
                });
        }
        let mut ranked: Vec<(i64, Fused)> = fused.into_iter().collect();
        ranked.sort_by(|a, b| b.1.score.total_cmp(&a.1.score).then(a.0.cmp(&b.0)));
        Ok(ranked)
    }

    /// How many of the ranked files are of each kind.
    fn kind_counts(&self, ranked: &[(i64, Fused)]) -> Result<Vec<KindCount>> {
        let ids: Vec<i64> = ranked.iter().map(|(id, _)| *id).collect();
        let mut counts: HashMap<Kind, u64> = HashMap::new();
        for f in self.store.files(&ids)? {
            *counts.entry(f.kind).or_default() += 1;
        }
        Ok(Kind::ALL
            .into_iter()
            .filter_map(|kind| counts.get(&kind).map(|&files| KindCount { kind, files }))
            .collect())
    }

    /// The file ids that pass `filters`, or `None` when nothing is filtered.
    fn allowed_files(&self, filters: &Filters) -> Result<Option<HashSet<i64>>> {
        if filters == &Filters::default() {
            return Ok(None);
        }
        let glob = filters
            .path_glob
            .as_deref()
            .map(|g| {
                globset::GlobBuilder::new(g)
                    .literal_separator(false)
                    .build()
                    .map(|g| g.compile_matcher())
                    .map_err(|e| Error::BadRequest(format!("path_glob: {e}")))
            })
            .transpose()?;
        let folders: Vec<String> = filters
            .folders
            .iter()
            .map(|id| self.folder(id).map(|f| f.path))
            .collect::<Result<_>>()?;
        let path_ok = glob.map(|g| move |p: &str| g.is_match(p));
        Ok(Some(
            self.store
                .filtered_files(
                    &filters.types,
                    &folders,
                    filters.date_from,
                    filters.date_to,
                    path_ok.as_ref().map(|f| f as &dyn Fn(&str) -> bool),
                )?
                .into_iter()
                .collect(),
        ))
    }

    /// Runs `f` on the in-memory vectors, loading them first if needed (or if another process
    /// changed the database since).
    fn with_vectors<R>(&self, f: impl FnOnce(&VectorIndex) -> R) -> Result<R> {
        let version = self.store.data_version()?;
        {
            let guard = self.vectors.read().unwrap();
            if let Some((ix, v)) = guard.as_ref()
                && *v == version
            {
                return Ok(f(ix));
            }
        }
        let mut guard = self.vectors.write().unwrap();
        let version = self.store.data_version()?;
        if guard.as_ref().is_none_or(|(_, v)| *v != version) {
            let started = Instant::now();
            let mut ix = VectorIndex::new(self.embedder().dim());
            self.store.for_each_vector(|chunk, file, kind, v| {
                ix.push(file, chunk, kind, v);
            })?;
            tracing::debug!(
                "loaded {} vectors in {:.0} ms",
                ix.len(),
                started.elapsed().as_secs_f64() * 1000.0
            );
            *guard = Some((ix, version));
        }
        Ok(f(&guard.as_ref().expect("loaded above").0))
    }
}

/// Longest snippet returned with a search hit, in characters.
const SNIPPET_CHARS: usize = 300;

/// The words of a query, lowercased, for highlighting.
fn query_words(q: &str) -> Vec<String> {
    q.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// A window of about `max_chars` characters of `text` around its densest run of query words,
/// cut into spans; words that start with a query word are marked.
fn snippet(text: &str, words: &[String], max_chars: usize) -> Vec<TextSpan> {
    // Byte ranges of the matching words.
    let mut matches: Vec<(usize, usize)> = vec![];
    if !words.is_empty() {
        let mut start = None;
        for (i, c) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ' ')))
        {
            match (c.is_alphanumeric(), start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    let word = text[s..i].to_lowercase();
                    if words.iter().any(|w| word.starts_with(w.as_str())) {
                        matches.push((s, i));
                    }
                    start = None;
                }
                _ => {}
            }
        }
    }
    // Start a little before the match with the most matches after it inside the window.
    let window = max_chars * 3 / 4;
    let best = (0..matches.len()).max_by_key(|&i| {
        let end = text[matches[i].0..]
            .char_indices()
            .nth(window)
            .map(|(b, _)| matches[i].0 + b)
            .unwrap_or(text.len());
        let n = matches[i..].iter().take_while(|m| m.1 <= end).count();
        (n, std::cmp::Reverse(i))
    });
    let from = match best {
        Some(i) => {
            let at = matches[i].0;
            let lead = text[..at]
                .char_indices()
                .rev()
                .nth(max_chars / 5)
                .map(|(b, _)| b)
                .unwrap_or(0);
            if lead == 0 {
                0
            } else {
                // Begin at a word boundary.
                text[lead..at]
                    .find(char::is_whitespace)
                    .map(|w| lead + w + 1)
                    .unwrap_or(at)
            }
        }
        None => 0,
    };
    let to = text[from..]
        .char_indices()
        .nth(max_chars)
        .map(|(b, _)| from + b)
        .unwrap_or(text.len());
    let mut spans = vec![];
    let mut push = |s: &str, hit: bool| {
        if !s.is_empty() {
            spans.push(TextSpan {
                text: s.to_string(),
                hit,
            });
        }
    };
    if from > 0 {
        push("…", false);
    }
    let mut pos = from;
    for (s, e) in matches.into_iter().filter(|(s, e)| *s >= from && *e <= to) {
        push(&text[pos..s], false);
        push(&text[s..e], true);
        pos = e;
    }
    push(&text[pos..to], false);
    if to < text.len() {
        push("…", false);
    }
    spans
}

/// The first `dim` numbers of `v`, renormalised.
fn truncated(v: &[f32], dim: usize) -> Vec<f32> {
    let mut out = v[..dim.min(v.len())].to_vec();
    normalize(&mut out);
    out
}

/// Parses `image,video` into kinds.
pub fn parse_kinds(s: &str) -> Result<Vec<Kind>> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|k| Kind::parse(k).ok_or_else(|| Error::BadRequest(format!("unknown type {k:?}"))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(spans: &[TextSpan]) -> String {
        spans
            .iter()
            .map(|s| {
                if s.hit {
                    format!("[{}]", s.text)
                } else {
                    s.text.clone()
                }
            })
            .collect()
    }

    #[test]
    fn snippets_mark_query_words() {
        let words = query_words("Red bicy");
        assert_eq!(
            render(&snippet("A red bicycle, reddish.", &words, 100)),
            "A [red] [bicycle], [reddish]."
        );
        let long = format!("{} the red bicycle {}", "x ".repeat(500), "y ".repeat(500));
        let out = render(&snippet(&long, &words, 60));
        assert!(out.starts_with('…') && out.ends_with('…'), "{out}");
        assert!(out.contains("[red] [bicycle]"), "{out}");
        assert!(out.chars().count() < 70, "{out}");
        assert_eq!(render(&snippet("no match here", &words, 5)), "no ma…");
        assert!(snippet("", &words, 5).is_empty());
        assert_eq!(
            render(&snippet("Zürich straße", &query_words("STRASSE zür"), 50)),
            "[Zürich] straße"
        );
    }
}
