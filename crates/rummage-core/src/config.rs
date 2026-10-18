//! The TOML config file, its `RUMMAGE_*` environment overrides, and dotted-key edits.
//!
//! The file is the source of truth and is edited in place (comments survive). Environment
//! overrides only change the effective config; they are never written back.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use toml_edit::DocumentMut;
use utoipa::ToSchema;

/// Everything configurable. The same keys appear in the config file, in
/// `rummage config get|set`, and in `GET|PATCH /api/v1/settings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Where the index, thumbnails and (by default) model files live. `~` is expanded.
    pub data_dir: String,
    pub server: ServerConfig,
    /// The folders to index.
    pub folders: Vec<FolderConfig>,
    pub model: ModelConfig,
    pub engine: EngineConfig,
    pub index: IndexConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// Address `rummage serve` listens on.
    pub bind: String,
    /// URL prefix when served behind a proxy, e.g. `/search`. Empty means the root.
    pub base_path: String,
    /// If set, API calls need `Authorization: Bearer <token>` (or `?access_token=`).
    pub auth_token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct FolderConfig {
    /// Folder to index. `~` is expanded.
    pub path: String,
    /// Only index files matching one of these globs (relative to the folder). Empty means all.
    pub include: Vec<String>,
    /// Skip files matching any of these globs (relative to the folder).
    pub exclude: Vec<String>,
    pub enabled: bool,
}

impl Default for FolderConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            include: vec![],
            exclude: vec![],
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ModelConfig {
    /// Hugging Face repo holding the GGUF files.
    pub repo: String,
    /// Weight format. Never float16: EmbeddingGemma 2 gives NaN or degraded vectors in it.
    pub quant: Quant,
    /// Text model file name. Empty picks the file for `quant`.
    pub file: String,
    /// Vision and audio projector file name. Empty picks the file for `quant`.
    pub mmproj: String,
    /// Where model files are stored. Empty means `<data_dir>/models`.
    pub dir: String,
    /// Vector length kept in the index: 768, 512, 256 or 128.
    pub dim: u32,
    /// Tokens spent per image, 70 to 1120.
    pub image_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum Quant {
    #[serde(rename = "Q8_0")]
    Q8_0,
    #[serde(rename = "BF16")]
    Bf16,
    #[serde(rename = "F32")]
    F32,
}

impl Quant {
    pub fn as_str(&self) -> &'static str {
        match self {
            Quant::Q8_0 => "Q8_0",
            Quant::Bf16 => "BF16",
            Quant::F32 => "F32",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct EngineConfig {
    /// Compute backend. `auto` picks the best one compiled in.
    pub backend: Backend,
    /// GPU to use: `auto` or a device index as listed by `rummage doctor`.
    pub device: String,
    /// CPU threads for the engine. 0 means automatic.
    pub threads: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Auto,
    Cpu,
    Vulkan,
    Rocm,
    Cuda,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct IndexConfig {
    /// Files extracted in parallel. 0 means automatic.
    pub concurrency: u32,
    /// Seconds between full re-scans (file watching covers changes in between). 0 disables.
    pub scan_interval_secs: u64,
    /// Longest edge of generated thumbnails, in pixels.
    pub thumbnail_size: u32,
    /// Directory names skipped everywhere.
    pub skip: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: "~/.local/share/rummage".into(),
            server: ServerConfig::default(),
            folders: vec![],
            model: ModelConfig::default(),
            engine: EngineConfig::default(),
            index: IndexConfig::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8099".into(),
            base_path: String::new(),
            auth_token: String::new(),
        }
    }
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            repo: "ggml-org/embeddinggemma-2-GGUF".into(),
            quant: Quant::Q8_0,
            file: String::new(),
            mmproj: String::new(),
            dir: String::new(),
            dim: 768,
            image_tokens: 280,
        }
    }
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            backend: Backend::Auto,
            device: "auto".into(),
            threads: 0,
        }
    }
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            concurrency: 0,
            scan_interval_secs: 3600,
            thumbnail_size: 320,
            skip: [
                "node_modules",
                ".git",
                "target",
                "dist",
                ".cache",
                "__pycache__",
                ".venv",
            ]
            .map(String::from)
            .to_vec(),
        }
    }
}

/// Environment variables and the config keys they override.
pub const ENV_OVERRIDES: &[(&str, &str)] = &[
    ("RUMMAGE_DATA_DIR", "data_dir"),
    ("RUMMAGE_BIND", "server.bind"),
    ("RUMMAGE_BASE_PATH", "server.base_path"),
    ("RUMMAGE_AUTH_TOKEN", "server.auth_token"),
    ("RUMMAGE_FOLDERS", "folders"),
    ("RUMMAGE_MODEL_REPO", "model.repo"),
    ("RUMMAGE_MODEL_QUANT", "model.quant"),
    ("RUMMAGE_MODEL_FILE", "model.file"),
    ("RUMMAGE_MODEL_MMPROJ", "model.mmproj"),
    ("RUMMAGE_MODEL_DIR", "model.dir"),
    ("RUMMAGE_DIM", "model.dim"),
    ("RUMMAGE_IMAGE_TOKENS", "model.image_tokens"),
    ("RUMMAGE_BACKEND", "engine.backend"),
    ("RUMMAGE_DEVICE", "engine.device"),
    ("RUMMAGE_THREADS", "engine.threads"),
    ("RUMMAGE_CONCURRENCY", "index.concurrency"),
    ("RUMMAGE_SCAN_INTERVAL", "index.scan_interval_secs"),
    ("RUMMAGE_THUMB_SIZE", "index.thumbnail_size"),
    ("RUMMAGE_SKIP", "index.skip"),
];

/// Keys that only take effect after a restart of `rummage serve`.
const RESTART_PREFIXES: &[&str] = &["data_dir", "server", "model", "engine", "index.concurrency"];

pub fn needs_restart(key: &str) -> bool {
    RESTART_PREFIXES
        .iter()
        .any(|p| key == *p || key.starts_with(&format!("{p}.")))
}

impl Config {
    /// Checks values that the types alone don't constrain.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            [768, 512, 256, 128].contains(&self.model.dim),
            "model.dim must be 768, 512, 256 or 128 (got {})",
            self.model.dim
        );
        ensure!(
            (70..=1120).contains(&self.model.image_tokens),
            "model.image_tokens must be between 70 and 1120 (got {})",
            self.model.image_tokens
        );
        ensure!(
            (32..=2048).contains(&self.index.thumbnail_size),
            "index.thumbnail_size must be between 32 and 2048 (got {})",
            self.index.thumbnail_size
        );
        self.server
            .bind
            .parse::<std::net::SocketAddr>()
            .with_context(|| format!("server.bind is not an address: {:?}", self.server.bind))?;
        let bp = &self.server.base_path;
        ensure!(
            bp.is_empty() || (bp.starts_with('/') && !bp.contains("//") && !bp.contains('?')),
            "server.base_path must be empty or start with '/' (got {bp:?})"
        );
        ensure!(
            !self.data_dir.trim().is_empty(),
            "data_dir must not be empty"
        );
        for f in &self.folders {
            ensure!(!f.path.trim().is_empty(), "folder path must not be empty");
            for g in f.include.iter().chain(&f.exclude) {
                globset::Glob::new(g).with_context(|| format!("bad glob {g:?} in {}", f.path))?;
            }
        }
        Ok(())
    }

    pub fn data_dir(&self) -> PathBuf {
        expand_home(&self.data_dir)
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir().join("rummage.db")
    }

    pub fn model_dir(&self) -> PathBuf {
        if self.model.dir.is_empty() {
            self.data_dir().join("models")
        } else {
            expand_home(&self.model.dir)
        }
    }

    /// The text model file, explicit or derived from `model.quant`.
    pub fn model_file(&self) -> String {
        if self.model.file.is_empty() {
            format!("embeddinggemma-2-{}.gguf", self.model.quant.as_str())
        } else {
            self.model.file.clone()
        }
    }

    /// The vision/audio projector file, explicit or derived from `model.quant`.
    pub fn mmproj_file(&self) -> String {
        if self.model.mmproj.is_empty() {
            format!("mmproj-embeddinggemma-2-{}.gguf", self.model.quant.as_str())
        } else {
            self.model.mmproj.clone()
        }
    }

    /// `server.base_path` without a trailing slash; empty for the root.
    pub fn base_path(&self) -> String {
        normalize_base_path(&self.server.base_path)
    }

    /// A copy safe to show to API clients: the auth token is masked.
    pub fn redacted(&self) -> Config {
        let mut c = self.clone();
        if !c.server.auth_token.is_empty() {
            c.server.auth_token = REDACTED.into();
        }
        c
    }

    /// Reads one dotted key (e.g. `server.bind`, `folders`) as JSON.
    pub fn get(&self, key: &str) -> Result<Json> {
        let root = serde_json::to_value(self)?;
        if key.is_empty() {
            return Ok(root);
        }
        let mut cur = &root;
        for seg in key.split('.') {
            cur = match cur {
                Json::Object(m) => m.get(seg),
                Json::Array(a) => seg.parse::<usize>().ok().and_then(|i| a.get(i)),
                _ => None,
            }
            .ok_or_else(|| anyhow!("unknown config key: {key}"))?;
        }
        Ok(cur.clone())
    }
}

/// The placeholder shown instead of a set auth token.
pub const REDACTED: &str = "********";

pub fn normalize_base_path(p: &str) -> String {
    let p = p.trim().trim_end_matches('/');
    if p.is_empty() {
        String::new()
    } else if p.starts_with('/') {
        p.to_string()
    } else {
        format!("/{p}")
    }
}

/// Expands a leading `~` to `$HOME`.
pub fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix('~'), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            PathBuf::from(home).join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(p),
    }
}

/// `$RUMMAGE_CONFIG`, else `$XDG_CONFIG_HOME/rummage/config.toml`, else `~/.config/rummage/config.toml`.
pub fn default_config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("RUMMAGE_CONFIG") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| expand_home("~/.config"));
    base.join("rummage").join("config.toml")
}

/// The config file on disk plus the effective config (file + environment).
#[derive(Debug, Clone)]
pub struct ConfigFile {
    path: PathBuf,
    doc: DocumentMut,
    effective: Config,
    env_keys: Vec<String>,
    env: Vec<(String, String)>,
}

impl ConfigFile {
    /// Loads `path` (a missing file means all defaults) and applies `RUMMAGE_*` variables.
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_with_env(path, |k| std::env::var(k).ok())
    }

    /// Like [`ConfigFile::load`], reading the environment through `env`.
    pub fn load_with_env(path: &Path, env: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let doc: DocumentMut = text
            .parse()
            .with_context(|| format!("parsing {}", path.display()))?;
        let env: Vec<(String, String)> = ENV_OVERRIDES
            .iter()
            .filter_map(|(var, _)| env(var).map(|v| (var.to_string(), v)))
            .collect();
        let mut cf = Self {
            path: path.to_path_buf(),
            doc,
            effective: Config::default(),
            env_keys: vec![],
            env,
        };
        cf.recompute()
            .with_context(|| format!("in {}", path.display()))?;
        Ok(cf)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The effective config.
    pub fn config(&self) -> &Config {
        &self.effective
    }

    /// Keys currently overridden by environment variables.
    pub fn env_overrides(&self) -> &[String] {
        &self.env_keys
    }

    /// The config as written in the file, without environment overrides.
    pub fn file_config(&self) -> Result<Config> {
        Ok(toml::from_str(&self.doc.to_string())?)
    }

    fn recompute(&mut self) -> Result<()> {
        let file_cfg = self.file_config()?;
        let mut json = serde_json::to_value(&file_cfg)?;
        let mut keys = vec![];
        for (var, raw) in &self.env {
            let key = ENV_OVERRIDES
                .iter()
                .find(|(v, _)| v == var)
                .map(|(_, k)| *k)
                .expect("known variable");
            let value = if key == "folders" {
                folders_from_list(raw)
            } else {
                parse_for_key(&json, key, raw).with_context(|| format!("in ${var}"))?
            };
            set_json(&mut json, key, value);
            keys.push(key.to_string());
        }
        let cfg: Config = serde_json::from_value(json).context("environment overrides")?;
        cfg.validate()?;
        self.effective = cfg;
        self.env_keys = keys;
        Ok(())
    }

    /// Sets one dotted key from a CLI string (`"8"`, `"true"`, `"a,b"`, `"/search"`) and saves.
    pub fn set(&mut self, key: &str, raw: &str) -> Result<()> {
        self.apply_patch(&patch_for(key, raw)?)?;
        Ok(())
    }

    /// Applies a JSON merge patch (RFC 7396) to the file, validates and saves.
    /// Returns the changed dotted keys.
    pub fn apply_patch(&mut self, patch: &Json) -> Result<Vec<String>> {
        let Json::Object(_) = patch else {
            bail!("settings patch must be a JSON object");
        };
        let before = self.clone();
        let old = serde_json::to_value(self.file_config()?)?;
        let mut leaves = vec![];
        flatten(patch, String::new(), &mut leaves);
        for (key, value) in &leaves {
            // A masked token sent back unchanged is not an edit.
            if key == "server.auth_token" && value.as_str() == Some(REDACTED) {
                continue;
            }
            if let Err(e) = self.write_leaf(key, value) {
                *self = before;
                return Err(e);
            }
        }
        if let Err(e) = self.recompute() {
            *self = before;
            return Err(e);
        }
        self.save()?;
        let new = serde_json::to_value(self.file_config()?)?;
        let mut changed = vec![];
        diff_keys(&old, &new, String::new(), &mut changed);
        Ok(changed)
    }

    /// Replaces the folder list and saves.
    pub fn set_folders(&mut self, folders: &[FolderConfig]) -> Result<()> {
        self.apply_patch(&serde_json::json!({ "folders": folders }))?;
        Ok(())
    }

    fn write_leaf(&mut self, key: &str, value: &Json) -> Result<()> {
        let segs: Vec<&str> = key.split('.').collect();
        let (last, parents) = segs.split_last().expect("non-empty key");
        let mut table = self.doc.as_table_mut() as &mut dyn toml_edit::TableLike;
        for seg in parents {
            let entry = table
                .entry(seg)
                .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
            table = entry
                .as_table_like_mut()
                .ok_or_else(|| anyhow!("config key {key} is not a table"))?;
        }
        if value.is_null() {
            table.remove(last);
        } else {
            table.insert(last, to_item(value)?);
        }
        Ok(())
    }

    /// Writes the file, creating its directory.
    pub fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.doc.to_string())
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("writing {}", self.path.display()))?;
        Ok(())
    }
}

/// A JSON merge patch setting one dotted key from a CLI string, typed by the key's schema.
pub fn patch_for(key: &str, raw: &str) -> Result<Json> {
    let value = if key == "folders" {
        folders_from_list(raw)
    } else {
        parse_for_key(&serde_json::to_value(Config::default())?, key, raw)?
    };
    let mut patch = Json::Null;
    set_json(&mut patch, key, value);
    Ok(patch)
}

/// `"/a:/b"` or `"/a,/b"` to a folder list.
fn folders_from_list(raw: &str) -> Json {
    Json::Array(
        raw.split([':', ','])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| serde_json::json!({ "path": p }))
            .collect(),
    )
}

/// Parses a string for `key`, using the current value's JSON type.
fn parse_for_key(current: &Json, key: &str, raw: &str) -> Result<Json> {
    let mut cur = current;
    for seg in key.split('.') {
        cur = cur
            .as_object()
            .and_then(|m| m.get(seg))
            .ok_or_else(|| anyhow!("unknown config key: {key}"))?;
    }
    Ok(match cur {
        Json::Bool(_) => Json::Bool(match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => true,
            "false" | "0" | "no" | "off" => false,
            other => bail!("{key} expects true or false, got {other:?}"),
        }),
        Json::Number(_) => Json::Number(
            raw.trim()
                .parse::<u64>()
                .map_err(|_| anyhow!("{key} expects a whole number, got {raw:?}"))?
                .into(),
        ),
        Json::Array(_) => Json::Array(
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| Json::String(s.into()))
                .collect(),
        ),
        Json::Object(_) => bail!("{key} is a section; set one of its keys instead"),
        Json::String(_) | Json::Null => Json::String(raw.to_string()),
    })
}

fn set_json(root: &mut Json, key: &str, value: Json) {
    let mut cur = root;
    for seg in key.split('.') {
        if !cur.is_object() {
            *cur = Json::Object(Default::default());
        }
        cur = cur
            .as_object_mut()
            .expect("object")
            .entry(seg)
            .or_insert(Json::Null);
    }
    *cur = value;
}

fn flatten(v: &Json, prefix: String, out: &mut Vec<(String, Json)>) {
    match v {
        Json::Object(m) if !m.is_empty() || prefix.is_empty() => {
            for (k, v) in m {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(v, key, out);
            }
        }
        _ => out.push((prefix, v.clone())),
    }
}

fn diff_keys(a: &Json, b: &Json, prefix: String, out: &mut Vec<String>) {
    match (a, b) {
        (Json::Object(x), Json::Object(y)) => {
            let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                diff_keys(
                    x.get(k).unwrap_or(&Json::Null),
                    y.get(k).unwrap_or(&Json::Null),
                    key,
                    out,
                );
            }
        }
        _ if a != b => out.push(prefix),
        _ => {}
    }
}

/// Converts a JSON value to a TOML item; arrays of objects become `[[tables]]`.
fn to_item(value: &Json) -> Result<toml_edit::Item> {
    let mut wrapper = toml::Table::new();
    wrapper.insert(
        "v".into(),
        toml::Value::try_from(value).context("value can't be stored in TOML")?,
    );
    let mut doc: DocumentMut = toml::to_string(&wrapper)?.parse()?;
    doc.remove("v").ok_or_else(|| anyhow!("empty value"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn load(text: &str, env: &[(&str, &str)]) -> (tempfile::TempDir, Result<ConfigFile>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        let env: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let cf = ConfigFile::load_with_env(&path, |k| env.get(k).cloned());
        (dir, cf)
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cf = ConfigFile::load_with_env(&dir.path().join("nope.toml"), |_| None).unwrap();
        assert_eq!(cf.config(), &Config::default());
        assert_eq!(cf.config().model_file(), "embeddinggemma-2-Q8_0.gguf");
        assert_eq!(
            cf.config().mmproj_file(),
            "mmproj-embeddinggemma-2-Q8_0.gguf"
        );
    }

    #[test]
    fn file_values_load() {
        let (_d, cf) = load(
            r#"
data_dir = "/srv/rummage"
[server]
bind = "0.0.0.0:9000"
base_path = "/search/"
[[folders]]
path = "/home/u/Pictures"
exclude = ["**/*.raw"]
[model]
quant = "BF16"
dim = 256
"#,
            &[],
        );
        let cf = cf.unwrap();
        let c = cf.config();
        assert_eq!(c.data_dir(), PathBuf::from("/srv/rummage"));
        assert_eq!(c.server.bind, "0.0.0.0:9000");
        assert_eq!(c.base_path(), "/search");
        assert_eq!(c.folders.len(), 1);
        assert!(c.folders[0].enabled);
        assert_eq!(c.folders[0].exclude, vec!["**/*.raw"]);
        assert_eq!(c.model.quant, Quant::Bf16);
        assert_eq!(c.model.dim, 256);
        assert_eq!(c.model_file(), "embeddinggemma-2-BF16.gguf");
        assert_eq!(c.index.thumbnail_size, 320);
    }

    #[test]
    fn env_overrides_win_and_are_not_saved() {
        let (dir, cf) = load(
            "[server]\nbind = \"127.0.0.1:1\"\n",
            &[
                ("RUMMAGE_BIND", "0.0.0.0:8099"),
                ("RUMMAGE_DIM", "512"),
                ("RUMMAGE_FOLDERS", "/a:/b"),
                ("RUMMAGE_BACKEND", "vulkan"),
                ("RUMMAGE_SKIP", "node_modules, .git"),
            ],
        );
        let mut cf = cf.unwrap();
        let c = cf.config().clone();
        assert_eq!(c.server.bind, "0.0.0.0:8099");
        assert_eq!(c.model.dim, 512);
        assert_eq!(c.engine.backend, Backend::Vulkan);
        assert_eq!(
            c.folders
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/a", "/b"]
        );
        assert_eq!(c.index.skip, vec!["node_modules", ".git"]);
        assert!(cf.env_overrides().contains(&"server.bind".to_string()));

        cf.set("index.thumbnail_size", "512").unwrap();
        let saved = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(saved.contains("bind = \"127.0.0.1:1\""), "{saved}");
        assert!(saved.contains("thumbnail_size = 512"), "{saved}");
        assert!(!saved.contains("8099"), "{saved}");
    }

    #[test]
    fn bad_values_are_rejected() {
        let (_d, cf) = load("[model]\ndim = 300\n", &[]);
        assert!(cf.unwrap_err().to_string().contains("in "));
        let (_d, cf) = load("[model]\ntypo = 1\n", &[]);
        assert!(cf.is_err());
        let (_d, cf) = load("", &[("RUMMAGE_DIM", "lots")]);
        assert!(cf.is_err());
        let (_d, cf) = load("", &[("RUMMAGE_BIND", "nowhere")]);
        assert!(cf.is_err());
    }

    #[test]
    fn set_keeps_comments_and_rolls_back_on_error() {
        let (dir, cf) = load("# my notes\n[model]\ndim = 768 # full size\n", &[]);
        let mut cf = cf.unwrap();
        cf.set("model.dim", "256").unwrap();
        cf.set("server.base_path", "/search").unwrap();
        let saved = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(saved.contains("# my notes"), "{saved}");
        assert!(saved.contains("dim = 256"), "{saved}");
        assert_eq!(cf.config().base_path(), "/search");

        assert!(cf.set("model.dim", "100").is_err());
        assert_eq!(cf.config().model.dim, 256);
        assert!(cf.set("model.nope", "1").is_err());
        assert_eq!(
            cf.config().get("model.dim").unwrap(),
            serde_json::json!(256)
        );
    }

    #[test]
    fn patch_reports_changed_keys_and_handles_folders() {
        let (_d, cf) = load("", &[]);
        let mut cf = cf.unwrap();
        let changed = cf
            .apply_patch(&serde_json::json!({
                "index": { "thumbnail_size": 400 },
                "folders": [{ "path": "/x", "exclude": ["*.tmp"] }],
                "server": { "auth_token": REDACTED }
            }))
            .unwrap();
        assert!(changed.contains(&"index.thumbnail_size".to_string()));
        assert!(changed.contains(&"folders".to_string()));
        assert_eq!(cf.config().folders[0].exclude, vec!["*.tmp"]);
        assert_eq!(cf.config().server.auth_token, "");

        let reloaded = ConfigFile::load_with_env(cf.path(), |_| None).unwrap();
        assert_eq!(reloaded.config(), cf.config());
    }

    #[test]
    fn redaction_and_restart_keys() {
        let mut c = Config::default();
        c.server.auth_token = "secret".into();
        assert_eq!(c.redacted().server.auth_token, REDACTED);
        assert!(needs_restart("server.bind"));
        assert!(needs_restart("model"));
        assert!(!needs_restart("folders"));
        assert!(!needs_restart("index.thumbnail_size"));
    }
}
