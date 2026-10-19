//! Request and response types of the HTTP API (`/api/v1`), shared by the server and the CLI.
//! Times are Unix seconds; media offsets are seconds from the start.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::config::Config;
use crate::embed::Modalities;

/// What a file is.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    ToSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Image,
    Video,
    Audio,
    /// Plain text, markdown and code.
    #[default]
    Text,
    /// PDF and other paged documents.
    Doc,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Image, Kind::Video, Kind::Audio, Kind::Text, Kind::Doc];

    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Video => "video",
            Kind::Audio => "audio",
            Kind::Text => "text",
            Kind::Doc => "doc",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What part of a file a segment is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SegmentKind {
    /// A passage of text.
    #[default]
    Text,
    /// A whole image.
    Image,
    /// A document page (rendered as an image).
    Page,
    /// A window of audio.
    Audio,
    /// A window of video frames.
    Video,
}

impl SegmentKind {
    const ALL: [SegmentKind; 5] = [
        SegmentKind::Text,
        SegmentKind::Image,
        SegmentKind::Page,
        SegmentKind::Audio,
        SegmentKind::Video,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            SegmentKind::Text => "text",
            SegmentKind::Image => "image",
            SegmentKind::Page => "page",
            SegmentKind::Audio => "audio",
            SegmentKind::Video => "video",
        }
    }

    pub fn parse(s: &str) -> Option<SegmentKind> {
        SegmentKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// Indexing state of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Pending,
    Indexed,
    Error,
    Skipped,
}

impl FileStatus {
    pub fn parse(s: &str) -> FileStatus {
        match s {
            "indexed" => FileStatus::Indexed,
            "error" => FileStatus::Error,
            "skipped" => FileStatus::Skipped,
            _ => FileStatus::Pending,
        }
    }
}

/// An indexed file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Item {
    pub id: i64,
    /// Absolute path on the server.
    pub path: String,
    pub name: String,
    /// Id of the folder it belongs to.
    pub folder_id: String,
    pub kind: Kind,
    pub mime: Option<String>,
    pub size: u64,
    /// Modification time.
    pub mtime: i64,
    /// Best known date: when the photo was taken if known, else `mtime`.
    pub date: i64,
    pub status: FileStatus,
    /// Why indexing failed, when `status` is `error`.
    pub error: Option<String>,
    /// EXIF and other embedded metadata.
    #[schema(value_type = Option<Object>)]
    pub exif: Option<serde_json::Value>,
    pub urls: ItemUrls,
}

/// Links for showing an item. They include the server's base path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ItemUrls {
    pub thumb: String,
    pub preview: String,
    pub raw: String,
}

/// A matched part of a file: a passage, a page or a time window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Segment {
    pub id: i64,
    pub kind: SegmentKind,
    /// 1-based page for documents.
    pub page: Option<i64>,
    /// Start of the window in seconds, for audio and video.
    pub t_start: Option<f64>,
    pub t_end: Option<f64>,
    /// The passage text, or a short excerpt.
    pub text: Option<String>,
    /// Similarity to the query, when there is one.
    pub score: Option<f32>,
    /// In search results: `text` cut into pieces, with the pieces that match the query's
    /// words marked `hit`. Joined, the pieces equal `text`. Empty when there is no text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snippet: Vec<TextSpan>,
}

/// A piece of a snippet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct TextSpan {
    pub text: String,
    /// True when this piece matches a word of the query.
    #[serde(default)]
    pub hit: bool,
}

/// Filters applied to a search.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct Filters {
    /// Only these kinds. Empty means all.
    pub types: Vec<Kind>,
    /// Only these folder ids. Empty means all.
    pub folders: Vec<String>,
    /// Only items dated at or after this time.
    pub date_from: Option<i64>,
    /// Only items dated before this time.
    pub date_to: Option<i64>,
    /// Only paths matching this glob, e.g. `**/2024/**`.
    pub path_glob: Option<String>,
}

/// How results are ranked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    /// Meaning and exact words, fused.
    #[default]
    Hybrid,
    /// Meaning only.
    Vector,
    /// Exact words only (names, paths, text, EXIF).
    Keyword,
}

/// A search. Give exactly one of `query`, `item_id` or `file`
/// (or send `multipart/form-data` with a `file` part and an optional `request` JSON part).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct SearchRequest {
    /// Text to search for.
    pub query: Option<String>,
    /// Find items similar to this one.
    pub item_id: Option<i64>,
    /// Find items similar to this uploaded file.
    pub file: Option<UploadedFile>,
    pub filters: Filters,
    pub mode: SearchMode,
    /// Default 50, at most 500.
    pub limit: Option<u32>,
    pub offset: u32,
}

/// A file sent inline with a search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct UploadedFile {
    pub name: Option<String>,
    /// e.g. `image/jpeg`; guessed from `name` when missing.
    pub mime: Option<String>,
    /// File bytes, base64.
    pub data_base64: String,
}

/// Search results, best first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    /// Number of matching items before `offset`/`limit` (capped at the candidate pool).
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
    pub took_ms: f64,
    /// Matching items per kind, ignoring the `types` filter (for filter chips; capped like
    /// `total`).
    #[serde(default)]
    pub kinds: Vec<KindCount>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SearchHit {
    pub item: Item,
    /// Fused rank score (higher is better).
    pub score: f32,
    /// Cosine similarity of the best-matching segment, when it matched by meaning.
    pub similarity: Option<f32>,
    /// True when the item matched exact words.
    pub keyword_match: bool,
    /// The best-matching segment: where to open the preview.
    pub segment: Option<Segment>,
}

/// An item with its segments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ItemDetail {
    pub item: Item,
    /// Segments, best match to `q` first when `q` is given, else in file order.
    pub segments: Vec<Segment>,
}

/// A watched folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Folder {
    /// Stable id derived from the path.
    pub id: String,
    pub path: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub enabled: bool,
    /// Whether the path exists and is a readable directory.
    pub available: bool,
    /// True when the folder list comes from `RUMMAGE_FOLDERS` and can't be edited.
    pub from_env: bool,
    pub counts: IndexCounts,
}

/// A folder to add.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct NewFolder {
    pub path: String,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// Changes to a folder; missing fields stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct FolderPatch {
    pub include: Option<Vec<String>>,
    pub exclude: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct IndexCounts {
    pub files: u64,
    pub indexed: u64,
    pub pending: u64,
    pub errors: u64,
    /// Files that can't be indexed here (e.g. videos without ffmpeg).
    #[serde(default)]
    pub skipped: u64,
    pub bytes: u64,
    pub chunks: u64,
    pub vectors: u64,
}

impl From<crate::store::Counts> for IndexCounts {
    fn from(c: crate::store::Counts) -> Self {
        Self {
            files: c.files,
            indexed: c.indexed,
            pending: c.pending,
            errors: c.errors,
            skipped: c.skipped,
            bytes: c.bytes,
            chunks: c.chunks,
            vectors: c.vectors,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum IndexState {
    #[default]
    Idle,
    Scanning,
    Indexing,
    Paused,
}

/// Where indexing stands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct IndexStatus {
    pub state: IndexState,
    pub totals: IndexCounts,
    pub by_kind: Vec<KindCount>,
    pub folders: Vec<FolderProgress>,
    /// File being worked on.
    pub current: Option<String>,
    /// Files waiting to be indexed.
    pub queued: u64,
    /// Recent indexing rate.
    pub files_per_min: Option<f64>,
    /// Estimated seconds until the queue is empty.
    pub eta_secs: Option<u64>,
    /// Things the user should know, e.g. that ffmpeg is missing so videos are skipped.
    #[serde(default)]
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct KindCount {
    pub kind: Kind,
    pub files: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FolderProgress {
    pub id: String,
    pub path: String,
    pub counts: IndexCounts,
    /// True while the folder is being walked for new and changed files.
    #[serde(default)]
    pub scanning: bool,
    /// Estimated seconds until this folder's waiting files are indexed, from the measured rate.
    #[serde(default)]
    pub eta_secs: Option<u64>,
}

/// One message on the `/index/events` stream (the SSE `event:` name is the `type`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IndexEvent {
    /// A full status snapshot; sent on connect and whenever totals change.
    Status(IndexStatus),
    /// A file finished (or failed).
    File {
        id: i64,
        path: String,
        kind: Kind,
        status: FileStatus,
        error: Option<String>,
    },
}

impl IndexEvent {
    pub fn name(&self) -> &'static str {
        match self {
            IndexEvent::Status(_) => "status",
            IndexEvent::File { .. } => "file",
        }
    }
}

/// What to re-index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "scope", rename_all = "lowercase")]
pub enum ReindexRequest {
    All,
    Folder {
        id: String,
    },
    Item {
        id: i64,
    },
    /// Every file that failed or was skipped.
    Problems,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ReindexResponse {
    /// Files queued.
    pub queued: u64,
}

/// What to re-scan for new, changed, moved and deleted files.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ScanRequest {
    /// A folder id or path; empty scans every enabled folder.
    pub folder: Option<String>,
}

/// Files that failed to index or were skipped, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ProblemList {
    pub total: u64,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, IntoParams)]
#[serde(default)]
#[into_params(parameter_in = Query)]
pub struct PageParams {
    /// Default 50, at most 500.
    pub limit: Option<u32>,
    pub offset: u32,
}

/// The settings (same keys as the config file), where they come from, and what needs a restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SettingsView {
    /// Effective settings. A set auth token shows as `********`.
    pub settings: Config,
    /// The config file on the server.
    pub config_path: String,
    /// Keys overridden by `RUMMAGE_*` variables; edits to them are saved but don't apply.
    pub env_overrides: Vec<String>,
    /// Keys changed since the server started that need a restart to apply.
    pub restart_required: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct EmbedderInfo {
    pub name: String,
    pub dim: u32,
    pub modalities: Modalities,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Health {
    /// Always `ok` when the server answers.
    pub status: String,
    pub version: String,
    pub schema_version: u32,
    pub embedder: EmbedderInfo,
    pub uptime_secs: u64,
    /// Whether API calls need a bearer token.
    pub auth_required: bool,
}

/// The loudness outline of an audio or video file, for drawing a waveform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Waveform {
    /// Length of the file in seconds.
    pub duration: f64,
    /// Peak levels from 0 to 1, evenly spaced over `duration`.
    pub peaks: Vec<f32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, IntoParams)]
#[serde(default)]
#[into_params(parameter_in = Query)]
pub struct WaveformParams {
    /// Number of peaks, default 200, at most 2000.
    pub points: Option<u32>,
}

/// Sub-folders of a directory on the server, for picking a folder to index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DirListing {
    /// The directory listed (absolute).
    pub path: String,
    /// Its parent, if it has one.
    pub parent: Option<String>,
    /// Sub-directories, by name.
    pub dirs: Vec<DirEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    /// Whether this directory is already an indexed folder.
    pub watched: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, IntoParams)]
#[serde(default)]
#[into_params(parameter_in = Query)]
pub struct DirParams {
    /// Absolute directory to list (`~` is expanded); default is the server user's home.
    pub path: Option<String>,
    /// Include hidden (dot) directories.
    pub hidden: bool,
}

/// Every error response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ApiError {
    /// Machine-readable code: `bad_request`, `unauthorized`, `not_found`, `conflict`,
    /// `unsupported`, `not_ready` or `internal`.
    pub error: String,
    pub message: String,
}

/// Query parameters for listing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, IntoParams, ToSchema)]
#[serde(default)]
#[into_params(parameter_in = Query)]
pub struct SimilarParams {
    /// Default 50, at most 500.
    pub limit: Option<u32>,
    pub offset: u32,
    /// Comma-separated kinds, e.g. `image,video`.
    #[serde(rename = "type")]
    #[param(rename = "type")]
    pub types: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, IntoParams)]
#[serde(default)]
#[into_params(parameter_in = Query)]
pub struct ItemParams {
    /// Rank the segments by this query.
    pub q: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, IntoParams)]
#[serde(default)]
#[into_params(parameter_in = Query)]
pub struct ThumbParams {
    /// Longest edge in pixels; defaults to `index.thumbnail_size`.
    pub size: Option<u32>,
    /// For video: the poster of the window that holds this second.
    pub t: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, IntoParams)]
#[serde(default)]
#[into_params(parameter_in = Query)]
pub struct PreviewParams {
    /// 1-based page, for documents.
    pub page: Option<u32>,
    /// Seconds from the start, for video.
    pub t: Option<f64>,
}
