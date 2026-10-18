//! What the API and the CLI do, independent of HTTP. The server wraps a [`Service`] in axum
//! handlers; the CLI calls it directly when no server is running.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use anyhow::Context;
use tokio::sync::broadcast;

use crate::config::{Config, ConfigFile, FolderConfig, expand_home, needs_restart};
use crate::embed::{EmbedInput, Embedder, ImageData, dot, fnv1a, normalize};
use crate::store::{FileRow, Scope, Store};
use crate::types::*;

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

/// All vectors, kept in memory for an exact scan.
struct VectorCache {
    fingerprint: (i64, i64),
    dim: usize,
    chunk_ids: Vec<i64>,
    file_ids: Vec<i64>,
    data: Vec<f32>,
}

pub struct Service {
    config: RwLock<ConfigFile>,
    started_config: Config,
    store: Store,
    embedder: Arc<dyn Embedder>,
    events: broadcast::Sender<IndexEvent>,
    vectors: RwLock<Option<Arc<VectorCache>>>,
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
            embedder,
            events,
            vectors: RwLock::new(None),
            started: Instant::now(),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn embedder(&self) -> &Arc<dyn Embedder> {
        &self.embedder
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
            name: self.embedder.name(),
            dim: self.embedder.dim() as u32,
            modalities: self.embedder.modalities(),
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
        self.folder(&target.id)
    }

    /// Stops watching a folder and drops its files from the index.
    pub fn remove_folder(&self, id: &str) -> Result<()> {
        let target = self.folder(id)?;
        self.edit_folders(|folders| {
            folders.retain(|f| expand_home(&f.path) != Path::new(&target.path));
            Ok(())
        })?;
        self.store.delete_folder(&target.path)?;
        self.publish_status();
        Ok(())
    }

    // ---- index ----

    pub fn index_status(&self) -> Result<IndexStatus> {
        let totals: IndexCounts = self.store.counts(None)?.into();
        Ok(IndexStatus {
            state: IndexState::Idle,
            queued: totals.pending,
            totals,
            by_kind: self
                .store
                .kind_counts()?
                .into_iter()
                .map(|(kind, files)| KindCount { kind, files })
                .collect(),
            folders: self
                .folders()?
                .into_iter()
                .map(|f| FolderProgress {
                    id: f.id,
                    path: f.path,
                    counts: f.counts,
                })
                .collect(),
            current: None,
            files_per_min: None,
            eta_secs: None,
        })
    }

    fn publish_status(&self) {
        if self.events.receiver_count() > 0
            && let Ok(s) = self.index_status()
        {
            self.publish(IndexEvent::Status(s));
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
        };
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
        cf.apply_patch(patch)
            .map_err(|e| Error::BadRequest(format!("{e:#}")))?;
        Ok(self.settings_view(&cf))
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
            })
            .collect();
        if let Some(q) = q.filter(|q| !q.trim().is_empty()) {
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

    /// Where a cached thumbnail or preview for `id` would be (`thumbs/<id>-<variant>.jpg`).
    pub fn cache_path(&self, dir: &str, id: i64, variant: &str) -> std::path::PathBuf {
        self.config()
            .data_dir()
            .join(dir)
            .join(format!("{id}-{variant}.jpg"))
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
        if !self.embedder.modalities().supports(&input) {
            return Err(Error::Unsupported(
                "the embedding model can't read this kind of input".into(),
            ));
        }
        let mut v = self
            .embedder
            .embed(std::slice::from_ref(&input))?
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
                "searching by {} files is not supported yet; use an image or text file",
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
        if let Some(q) = &req.query {
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
        let want = (req.offset + limit) as usize;
        let pool = (want * 4).max(200);
        let allowed = self.allowed_files(&req.filters)?;
        let ok = |file_id: i64| {
            Some(file_id) != exclude && allowed.as_ref().is_none_or(|a| a.contains(&file_id))
        };

        // Best chunk per file from the vector scan, ranked.
        let mut vector_hits: Vec<(i64, i64, f32)> = vec![];
        if let Some(qv) = &qv
            && req.mode != SearchMode::Keyword
        {
            let cache = self.vector_cache()?;
            if cache.dim > 0 && cache.dim <= qv.len() {
                let q = truncated(qv, cache.dim);
                let mut best: HashMap<i64, (i64, f32)> = HashMap::new();
                for (i, row) in cache.data.chunks_exact(cache.dim).enumerate() {
                    let file_id = cache.file_ids[i];
                    if !ok(file_id) {
                        continue;
                    }
                    let s = dot(row, &q);
                    let e = best.entry(file_id).or_insert((cache.chunk_ids[i], s));
                    if s > e.1 {
                        *e = (cache.chunk_ids[i], s);
                    }
                }
                vector_hits = best.into_iter().map(|(f, (c, s))| (f, c, s)).collect();
                vector_hits.sort_by(|a, b| b.2.total_cmp(&a.2));
                vector_hits.truncate(pool);
            }
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
        struct Fused {
            score: f32,
            chunk: i64,
            similarity: Option<f32>,
            keyword: bool,
        }
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
        let total = ranked.len() as u64;

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
        let mut hits = Vec::with_capacity(page.len());
        for (file_id, f) in page {
            let Some(row) = files.get(file_id) else {
                continue;
            };
            let segment = self.store.chunk(f.chunk)?.map(|c| Segment {
                id: c.id,
                kind: c.kind,
                page: c.page,
                t_start: c.t_start,
                t_end: c.t_end,
                text: c.text.map(|t| excerpt(&t, 300)),
                score: f.similarity,
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
        })
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
        Ok(Some(
            self.store
                .file_facets()?
                .into_iter()
                .filter(|f| {
                    (filters.types.is_empty() || filters.types.contains(&f.kind))
                        && (folders.is_empty() || folders.contains(&f.folder))
                        && filters.date_from.is_none_or(|d| f.date >= d)
                        && filters.date_to.is_none_or(|d| f.date < d)
                        && glob.as_ref().is_none_or(|g| g.is_match(&f.path))
                })
                .map(|f| f.id)
                .collect(),
        ))
    }

    fn vector_cache(&self) -> Result<Arc<VectorCache>> {
        let fp = self.store.vectors_fingerprint()?;
        if let Some(c) = self.vectors.read().unwrap().as_ref()
            && c.fingerprint == fp
        {
            return Ok(c.clone());
        }
        let mut cache = VectorCache {
            fingerprint: fp,
            dim: 0,
            chunk_ids: vec![],
            file_ids: vec![],
            data: vec![],
        };
        self.store.for_each_vector(|chunk, file, v| {
            if cache.dim == 0 {
                cache.dim = v.len();
            }
            if v.len() == cache.dim {
                cache.chunk_ids.push(chunk);
                cache.file_ids.push(file);
                cache.data.extend_from_slice(v);
            }
        })?;
        let cache = Arc::new(cache);
        *self.vectors.write().unwrap() = Some(cache.clone());
        Ok(cache)
    }
}

/// The first `dim` numbers of `v`, renormalised.
fn truncated(v: &[f32], dim: usize) -> Vec<f32> {
    let mut out = v[..dim.min(v.len())].to_vec();
    normalize(&mut out);
    out
}

fn excerpt(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

/// Parses `image,video` into kinds.
pub fn parse_kinds(s: &str) -> Result<Vec<Kind>> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|k| Kind::parse(k).ok_or_else(|| Error::BadRequest(format!("unknown type {k:?}"))))
        .collect()
}
