//! The `rummage` command line.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use rummage_core::config::{ENV_OVERRIDES, default_config_path, expand_home, patch_for};
use rummage_core::service::folder_id;
use rummage_core::types::*;
use rummage_core::{ConfigFile, Store};

use crate::client::{Api, Client, local_url};
use crate::{engine, indexer, server};

#[derive(Parser, Debug)]
#[command(
    name = "rummage",
    version,
    about = "Local multimodal semantic search for your files",
    long_about = "Local multimodal semantic search for your files.\n\n\
        `rummage serve` runs the web UI and the HTTP API. The other commands talk to a running \
        server when there is one, and open the index directly when there isn't.",
    after_help = "Examples:\n  \
        rummage folders add ~/Pictures         start indexing a folder\n  \
        rummage serve                          web UI and API at http://127.0.0.1:8099/\n  \
        rummage search \"boats at sunset\"\n  \
        rummage search --type image,video --after 2024-06-01 beach\n  \
        rummage doctor                         check the GPU, model files, ffmpeg and poppler\n\n\
        Config: ~/.config/rummage/config.toml (see `rummage config path`); RUMMAGE_* variables override it."
)]
pub struct Cli {
    /// Config file to use.
    #[arg(
        long,
        global = true,
        env = "RUMMAGE_CONFIG",
        value_name = "FILE",
        help_heading = "Global options"
    )]
    pub config: Option<PathBuf>,
    /// Talk to the server at this URL (e.g. http://host:8099/search) instead of finding one.
    #[arg(
        long,
        global = true,
        env = "RUMMAGE_SERVER",
        value_name = "URL",
        help_heading = "Global options"
    )]
    pub server: Option<String>,
    /// Open the index directly even if a server is running.
    #[arg(
        long,
        global = true,
        conflicts_with = "server",
        help_heading = "Global options"
    )]
    pub local: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Run the web UI and the HTTP API.
    Serve {
        /// Address to listen on [default: server.bind, 127.0.0.1:8099].
        #[arg(long, value_name = "ADDR")]
        bind: Option<String>,
        /// URL prefix when behind a proxy, e.g. /search [default: server.base_path].
        #[arg(long, value_name = "PATH")]
        base_path: Option<String>,
    },
    /// Add, change, remove or list the folders to index.
    #[command(subcommand)]
    Folders(FoldersCmd),
    /// Index the folders now.
    Index {
        /// Index new and changed files, then exit (the default).
        #[arg(long, conflicts_with_all = ["watch", "pause", "resume"])]
        once: bool,
        /// Keep running and index changes as they happen.
        #[arg(long, conflicts_with_all = ["pause", "resume"])]
        watch: bool,
        /// Pause the running server's indexing (searches keep working).
        #[arg(long, conflicts_with = "resume")]
        pause: bool,
        /// Resume the running server's indexing.
        #[arg(long)]
        resume: bool,
    },
    /// Index files again: everything, a folder, an item, or the ones that failed.
    Reindex {
        /// Only this folder (path or id).
        #[arg(long, value_name = "FOLDER", conflicts_with_all = ["item", "problems"])]
        folder: Option<String>,
        /// Only this item id.
        #[arg(long, value_name = "ID", conflicts_with = "problems")]
        item: Option<i64>,
        /// Only the files that failed or were skipped.
        #[arg(long)]
        problems: bool,
    },
    /// Search the index.
    Search(SearchArgs),
    /// Show one indexed item: what it is and its passages, pages or time windows.
    Item {
        /// The item's id (as `rummage search` prints it).
        id: i64,
        /// Rank its segments by this query, best first.
        #[arg(long, short)]
        query: Option<String>,
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show what is indexed and what is left.
    Status {
        /// Print JSON.
        #[arg(long)]
        json: bool,
        /// List the files that failed or were skipped, and why.
        #[arg(long)]
        errors: bool,
    },
    /// Read or change the config file.
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Check the config, the index, the folders, the GPU and ffmpeg.
    Doctor,
}

#[derive(Subcommand, Debug)]
pub enum FoldersCmd {
    /// Start indexing a folder.
    Add {
        /// The folder to index.
        path: PathBuf,
        /// Skip files matching this glob (relative to the folder); repeatable.
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,
        /// Only index files matching this glob; repeatable.
        #[arg(long, value_name = "GLOB")]
        include: Vec<String>,
    },
    /// Change a folder: turn it off or on, or replace its globs.
    #[command(group(clap::ArgGroup::new("change").required(true).multiple(true)
        .args(["off", "on", "exclude", "include"])))]
    Set {
        /// The folder's path or id.
        path: String,
        /// Stop indexing and watching it.
        #[arg(long, conflicts_with = "on")]
        off: bool,
        /// Index and watch it again.
        #[arg(long)]
        on: bool,
        /// Skip files matching this glob, replacing the folder's list; repeatable.
        /// `--exclude ''` clears the list.
        #[arg(long, value_name = "GLOB")]
        exclude: Option<Vec<String>>,
        /// Only index files matching this glob, replacing the folder's list; repeatable.
        /// `--include ''` clears the list.
        #[arg(long, value_name = "GLOB")]
        include: Option<Vec<String>>,
    },
    /// Stop indexing a folder and drop it from the index (the files stay).
    Rm {
        /// The folder's path or id.
        path: String,
    },
    /// List the folders.
    Ls {
        /// Print JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args, Debug)]
#[command(group(clap::ArgGroup::new("what").required(true).args(["query", "similar", "file"])))]
pub struct SearchArgs {
    /// What to look for.
    #[arg(num_args = 1..)]
    pub query: Vec<String>,
    /// Find items like the indexed item with this id.
    #[arg(long, value_name = "ID", conflicts_with_all = ["query", "file"])]
    pub similar: Option<i64>,
    /// Find items like this file (an image, audio or text file).
    #[arg(long, value_name = "PATH", conflicts_with = "query")]
    pub file: Option<PathBuf>,
    /// Only these kinds; repeatable or comma-separated.
    #[arg(long = "type", value_name = "KIND", value_delimiter = ',',
          value_parser = ["image", "video", "audio", "text", "doc"])]
    pub types: Vec<String>,
    /// Only this folder (path or id); repeatable.
    #[arg(long, value_name = "FOLDER")]
    pub folder: Vec<String>,
    /// Only paths matching this glob, e.g. '**/2024/**'.
    #[arg(long, value_name = "GLOB")]
    pub path: Option<String>,
    /// Only items dated on or after this day (YYYY-MM-DD).
    #[arg(long, value_name = "DATE", value_parser = parse_day)]
    pub after: Option<i64>,
    /// Only items dated before this day (YYYY-MM-DD).
    #[arg(long, value_name = "DATE", value_parser = parse_day)]
    pub before: Option<i64>,
    /// Ranking: meaning and words fused (hybrid), meaning only (vector), or words only (keyword).
    #[arg(long, default_value = "hybrid", value_parser = ["hybrid", "vector", "keyword"])]
    pub mode: String,
    /// Number of results.
    #[arg(long, short = 'n', default_value_t = 20)]
    pub limit: u32,
    /// Print JSON.
    #[arg(long)]
    pub json: bool,
}

/// `YYYY-MM-DD` to Unix seconds (UTC midnight).
fn parse_day(s: &str) -> Result<i64, String> {
    let bad = || format!("expected a date like 2024-05-01, got {s:?}");
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 {
        return Err(bad());
    }
    crate::extract::image::parse_exif_date(s).ok_or_else(bad)
}

#[derive(Subcommand, Debug)]
pub enum ConfigCmd {
    /// Print a key (e.g. `server.bind`), or the whole config.
    Get { key: Option<String> },
    /// Set a key in the config file, e.g. `rummage config set model.dim 256`.
    Set { key: String, value: String },
    /// Print the config file's path.
    Path,
}

pub fn main() -> ExitCode {
    let cli = Cli::parse();
    let filter = if matches!(cli.cmd, Cmd::Serve { .. }) {
        "info"
    } else {
        "warn"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_writer(std::io::stderr)
        .init();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let config_path = cli.config.clone().unwrap_or_else(default_config_path);
    let server = cli.server.as_deref().filter(|s| !s.trim().is_empty());
    if let Cmd::Config(ConfigCmd::Path) = cli.cmd {
        println!("{}", config_path.display());
        return Ok(ExitCode::SUCCESS);
    }
    let cf = ConfigFile::load(&config_path)?;
    crate::extract::set_ffmpeg(&cf.config().tools.ffmpeg);
    let connect = |cf: ConfigFile| Api::connect(cf, server, cli.local);
    match cli.cmd {
        Cmd::Serve { bind, base_path } => serve(cf, bind, base_path)?,
        Cmd::Folders(cmd) => folders(connect(cf)?, cmd)?,
        Cmd::Index {
            watch,
            pause,
            resume,
            ..
        } => index(connect(cf)?, watch, pause, resume)?,
        Cmd::Reindex {
            folder,
            item,
            problems,
        } => reindex(connect(cf)?, folder, item, problems)?,
        Cmd::Search(args) => search(connect(cf)?, args)?,
        Cmd::Item { id, query, json } => item(connect(cf)?, id, query.as_deref(), json)?,
        Cmd::Status { json, errors } => status(connect(cf)?, json, errors)?,
        Cmd::Config(cmd) => config(cf, cmd, server, cli.local)?,
        Cmd::Doctor => return doctor(cf, server),
    }
    Ok(ExitCode::SUCCESS)
}

fn serve(cf: ConfigFile, bind: Option<String>, base_path: Option<String>) -> Result<()> {
    // Command-line values apply to this run only, like environment variables.
    let flags = [("RUMMAGE_BIND", bind), ("RUMMAGE_BASE_PATH", base_path)];
    let cf = if flags.iter().any(|(_, v)| v.is_some()) {
        ConfigFile::load_with_env(cf.path(), |var| {
            flags
                .iter()
                .find(|(name, _)| *name == var)
                .and_then(|(_, v)| v.clone())
                .or_else(|| std::env::var(var).ok())
        })?
    } else {
        cf
    };
    let bind = cf.config().server.bind.clone();
    let svc = engine::open_service(cf, true)?;
    let rt = tokio::runtime::Runtime::new()?;
    let indexer = indexer::start(svc.clone(), indexer::Options { watch: true })?;
    let prep = svc.clone();
    std::thread::Builder::new()
        .name("playable".into())
        .spawn(move || crate::extract::playable::prepare(prep))?;
    let served = rt.block_on(server::serve(svc, &bind));
    indexer.stop();
    served
}

fn folders(api: Api, cmd: FoldersCmd) -> Result<()> {
    match cmd {
        FoldersCmd::Add {
            path,
            exclude,
            include,
        } => {
            let path = absolute(&path)?;
            let f = api.add_folder(NewFolder {
                path: path.to_string_lossy().into_owned(),
                include,
                exclude,
                enabled: true,
            })?;
            println!("added {} (id {})", f.path, f.id);
        }
        FoldersCmd::Set {
            path,
            off,
            on,
            exclude,
            include,
        } => {
            let globs = |v: Option<Vec<String>>| {
                v.map(|v| v.into_iter().filter(|g| !g.trim().is_empty()).collect())
            };
            let patch = FolderPatch {
                include: globs(include),
                exclude: globs(exclude),
                enabled: (off || on).then_some(on),
            };
            let f = api.update_folder(&folder_ref(&path)?, &patch)?;
            println!(
                "{} {}{}{}",
                f.path,
                if f.enabled { "on" } else { "off" },
                if f.exclude.is_empty() {
                    String::new()
                } else {
                    format!(", exclude {}", f.exclude.join(", "))
                },
                if f.include.is_empty() {
                    String::new()
                } else {
                    format!(", include {}", f.include.join(", "))
                }
            );
        }
        FoldersCmd::Rm { path } => {
            let id = if path.contains('/') || path.starts_with('~') || path == "." {
                let abs = absolute(Path::new(&path))?;
                folder_id(&abs.to_string_lossy())
            } else {
                path.clone()
            };
            api.remove_folder(&id)
                .or_else(|e| api.remove_folder(&path).map_err(|_| e))?;
            println!("removed {path}");
        }
        FoldersCmd::Ls { json } => {
            let folders = api.folders()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&folders)?);
            } else if folders.is_empty() {
                println!("no folders yet; add one with `rummage folders add <path>`");
            } else {
                for f in folders {
                    let mut notes = vec![];
                    if !f.enabled {
                        notes.push("disabled".to_string());
                    }
                    if !f.available {
                        notes.push("missing".to_string());
                    }
                    if !f.exclude.is_empty() {
                        notes.push(format!("exclude {}", f.exclude.join(", ")));
                    }
                    if !f.include.is_empty() {
                        notes.push(format!("include {}", f.include.join(", ")));
                    }
                    println!(
                        "{}  {}  {}/{} indexed{}",
                        f.id,
                        f.path,
                        f.counts.indexed,
                        f.counts.files,
                        if notes.is_empty() {
                            String::new()
                        } else {
                            format!("  ({})", notes.join("; "))
                        }
                    );
                }
            }
        }
    }
    Ok(())
}

/// An absolute path with `~` expanded, canonical when it exists.
fn absolute(path: &Path) -> Result<PathBuf> {
    let p = expand_home(&path.to_string_lossy());
    let p = if p.is_absolute() {
        p
    } else {
        std::env::current_dir()?.join(p)
    };
    Ok(std::fs::canonicalize(&p).unwrap_or(p))
}

fn index(api: Api, watch: bool, pause: bool, resume: bool) -> Result<()> {
    if pause || resume {
        let Api::Remote(_) = &api else {
            anyhow::bail!("no rummage server is running; pausing applies to `rummage serve`");
        };
        let s = if pause { api.pause()? } else { api.resume()? };
        println!("indexing {}", format!("{:?}", s.state).to_lowercase());
        return Ok(());
    }
    match api {
        Api::Remote(client) => {
            if !watch {
                client.send_empty("POST", "/index/scan", &ScanRequest::default())?;
            }
            let mut last = None;
            client.follow_events(|e| match e {
                IndexEvent::Status(s) => {
                    let line = format!(
                        "{:?}: {}/{} files indexed, {} queued, {} errors{}",
                        s.state,
                        s.totals.indexed,
                        s.totals.files,
                        s.queued,
                        s.totals.errors,
                        s.eta_secs
                            .map(|e| format!(", about {} left", indexer::human_secs(e)))
                            .unwrap_or_default()
                    )
                    .to_lowercase();
                    if last.as_ref() != Some(&line) {
                        eprintln!("{line}");
                        last = Some(line);
                    }
                    watch || s.queued > 0 || s.state != IndexState::Idle
                }
                IndexEvent::File {
                    path,
                    status,
                    error,
                    ..
                } => {
                    match error {
                        Some(err) => eprintln!("{status:?} {path}: {err}"),
                        None => eprintln!("{status:?} {path}"),
                    }
                    true
                }
            })
        }
        Api::Local(svc) => {
            if watch {
                tokio::runtime::Runtime::new()?.block_on(indexer::watch(svc))
            } else {
                indexer::run_once(svc)
            }
        }
    }
}

fn reindex(api: Api, folder: Option<String>, item: Option<i64>, problems: bool) -> Result<()> {
    let req = match (folder, item, problems) {
        (Some(f), _, _) => ReindexRequest::Folder {
            id: folder_ref(&f)?,
        },
        (None, Some(id), _) => ReindexRequest::Item { id },
        (None, None, true) => ReindexRequest::Problems,
        (None, None, false) => ReindexRequest::All,
    };
    let queued = api.reindex(&req)?.queued;
    match api {
        Api::Remote(_) => println!("queued {queued} files; the server is indexing them"),
        Api::Local(svc) => {
            eprintln!("queued {queued} files");
            indexer::run_once(svc)?;
        }
    }
    Ok(())
}

/// A folder given as a path (or `.`) becomes its id; anything else is taken as an id.
fn folder_ref(f: &str) -> Result<String> {
    if f.contains('/') || f.starts_with('~') || f == "." {
        Ok(folder_id(&absolute(Path::new(f))?.to_string_lossy()))
    } else {
        Ok(f.to_string())
    }
}

fn search(api: Api, args: SearchArgs) -> Result<()> {
    let file = match &args.file {
        Some(p) => {
            use base64::Engine as _;
            let bytes = std::fs::read(p).with_context(|| format!("reading {}", p.display()))?;
            Some(UploadedFile {
                name: p.file_name().map(|n| n.to_string_lossy().into_owned()),
                mime: None,
                data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            })
        }
        None => None,
    };
    let folders = args
        .folder
        .iter()
        .map(|f| folder_ref(f))
        .collect::<Result<Vec<_>>>()?;
    let req = SearchRequest {
        query: (!args.query.is_empty()).then(|| args.query.join(" ")),
        item_id: args.similar,
        file,
        filters: Filters {
            types: args.types.iter().filter_map(|t| Kind::parse(t)).collect(),
            folders,
            date_from: args.after,
            date_to: args.before,
            path_glob: args.path.clone(),
        },
        mode: match args.mode.as_str() {
            "vector" => SearchMode::Vector,
            "keyword" => SearchMode::Keyword,
            _ => SearchMode::Hybrid,
        },
        limit: Some(args.limit),
        ..Default::default()
    };
    let resp = api.search(&req)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    if resp.hits.is_empty() {
        eprintln!("no results");
    }
    for hit in &resp.hits {
        let at = hit
            .segment
            .as_ref()
            .map(|s| match (s.page, s.t_start) {
                (Some(p), _) => format!("  (page {p})"),
                (None, Some(t)) => format!("  (at {})", clock(t)),
                _ => String::new(),
            })
            .unwrap_or_default();
        println!(
            "{:>6}  {:.3}  {:<5}  {}{at}",
            hit.item.id,
            hit.similarity.unwrap_or(hit.score),
            hit.item.kind,
            hit.item.path
        );
        if let Some(seg) = &hit.segment
            && !seg.snippet.is_empty()
        {
            let bold = std::io::IsTerminal::is_terminal(&std::io::stdout());
            let text: String = seg
                .snippet
                .iter()
                .map(|s| match (s.hit, bold) {
                    (true, true) => format!("\x1b[1m{}\x1b[0m", s.text),
                    _ => s.text.clone(),
                })
                .collect();
            println!(
                "        {}",
                text.split_whitespace().collect::<Vec<_>>().join(" ")
            );
        }
    }
    Ok(())
}

fn item(api: Api, id: i64, query: Option<&str>, json: bool) -> Result<()> {
    let d = api.item(id, query)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&d)?);
        return Ok(());
    }
    let it = &d.item;
    println!("{}", it.path);
    println!(
        "id {} · {} · {} · {}",
        it.id,
        it.kind,
        it.mime.as_deref().unwrap_or("?"),
        human_bytes(it.size)
    );
    if it.status != FileStatus::Indexed {
        println!(
            "{:?}{}",
            it.status,
            it.error
                .as_deref()
                .map(|e| format!(": {e}"))
                .unwrap_or_default()
        );
    }
    for s in &d.segments {
        let at = match (s.page, s.t_start, s.t_end) {
            (Some(p), _, _) => format!("page {p}"),
            (None, Some(a), Some(b)) => format!("{}-{}", clock(a), clock(b)),
            (None, Some(a), None) => clock(a),
            _ => format!("{:?}", s.kind).to_lowercase(),
        };
        let score = s.score.map(|v| format!("{v:.3}  ")).unwrap_or_default();
        let text = s
            .text
            .as_deref()
            .map(|t| {
                let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
                let cut: String = t.chars().take(100).collect();
                if cut.len() < t.len() {
                    format!("  {cut}…")
                } else {
                    format!("  {cut}")
                }
            })
            .unwrap_or_default();
        println!("  {score}{at}{text}");
    }
    Ok(())
}

/// Seconds as `m:ss` or `h:mm:ss`.
fn clock(t: f64) -> String {
    let t = t.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}

fn status(api: Api, json: bool, errors: bool) -> Result<()> {
    let health = api.health()?;
    let s = api.status()?;
    let problems = if errors || json {
        Some(api.problems(if errors { 500 } else { 20 })?)
    } else {
        None
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({ "health": health, "index": s, "problems": problems })
            )?
        );
        return Ok(());
    }
    if errors {
        let p = problems.expect("fetched above");
        if p.items.is_empty() {
            println!("no failed or skipped files");
        }
        for item in &p.items {
            println!(
                "{:<7}  {}: {}",
                format!("{:?}", item.status).to_lowercase(),
                item.path,
                item.error.as_deref().unwrap_or("")
            );
        }
        if p.total > p.items.len() as u64 {
            println!("... and {} more", p.total - p.items.len() as u64);
        }
        return Ok(());
    }
    println!(
        "rummage {} ({}), embedder: {}",
        health.version,
        api.describe(),
        health.embedder.name
    );
    println!(
        "{}: {} files, {} indexed, {} queued, {} errors, {} skipped, {} vectors, {}",
        format!("{:?}", s.state).to_lowercase(),
        s.totals.files,
        s.totals.indexed,
        s.queued,
        s.totals.errors,
        s.totals.skipped,
        s.totals.vectors,
        human_bytes(s.totals.bytes)
    );
    if let Some(rate) = s.files_per_min {
        println!(
            "rate: {rate:.0} files/min{}",
            s.eta_secs
                .map(|e| format!(", about {} left", indexer::human_secs(e)))
                .unwrap_or_default()
        );
    }
    if let Some(c) = &s.current {
        println!("working on: {c}");
    }
    if !s.by_kind.is_empty() {
        let kinds: Vec<String> = s
            .by_kind
            .iter()
            .map(|k| format!("{} {}", k.files, k.kind))
            .collect();
        println!("by kind: {}", kinds.join(", "));
    }
    for f in &s.folders {
        println!(
            "  {}  {}/{} indexed, {} errors{}{}",
            f.path,
            f.counts.indexed,
            f.counts.files,
            f.counts.errors,
            if f.scanning { ", scanning" } else { "" },
            f.eta_secs
                .map(|e| format!(", about {} left", indexer::human_secs(e)))
                .unwrap_or_default()
        );
    }
    for m in &s.messages {
        println!("note: {m}");
    }
    if s.totals.errors + s.totals.skipped > 0 {
        println!(
            "see `rummage status --errors` for the files that failed or were skipped, and \
             `rummage reindex --problems` to try them again"
        );
    }
    Ok(())
}

fn human_bytes(b: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", units[i])
    }
}

fn config(mut cf: ConfigFile, cmd: ConfigCmd, server: Option<&str>, local: bool) -> Result<()> {
    // A running server owns the config: changes go through it so it sees them.
    let remote = if local {
        None
    } else {
        let url = server
            .map(String::from)
            .unwrap_or_else(|| local_url(cf.config()));
        let c = Client::new(&url, &cf.config().server.auth_token);
        match c.health() {
            Ok(_) => Some(c),
            Err(e) if server.is_some() => {
                return Err(e.context(format!("no rummage server at {url}")));
            }
            Err(_) => None,
        }
    };
    match cmd {
        ConfigCmd::Path => unreachable!("handled before loading"),
        ConfigCmd::Get { key } => {
            let settings = match &remote {
                Some(c) => c.get::<SettingsView>("/settings")?.settings,
                None => cf.config().redacted(),
            };
            let value = settings.get(key.as_deref().unwrap_or(""))?;
            match value {
                serde_json::Value::String(s) => println!("{s}"),
                serde_json::Value::Object(_) => print!("{}", toml::to_string_pretty(&value)?),
                v => println!("{v}"),
            }
        }
        ConfigCmd::Set { key, value } => {
            let env_key = ENV_OVERRIDES
                .iter()
                .find(|(_, k)| *k == key)
                .map(|(v, _)| *v);
            match &remote {
                Some(c) => {
                    let view: SettingsView =
                        c.send("PATCH", "/settings", &patch_for(&key, &value)?)?;
                    if view.restart_required.contains(&key) {
                        eprintln!("saved; restart `rummage serve` for it to take effect");
                    }
                }
                None => {
                    cf.set(&key, &value)?;
                }
            }
            if let Some(var) = env_key.filter(|v| std::env::var_os(v).is_some()) {
                eprintln!("note: ${var} is set and overrides this value");
            }
        }
    }
    Ok(())
}

fn doctor(cf: ConfigFile, server: Option<&str>) -> Result<ExitCode> {
    let mut failed = false;
    let mut check = |ok: bool, warn_only: bool, msg: String| {
        let mark = if ok {
            "ok  "
        } else if warn_only {
            "warn"
        } else {
            failed = true;
            "FAIL"
        };
        println!("[{mark}] {msg}");
    };
    let cfg = cf.config().clone();

    check(
        true,
        false,
        format!(
            "config {} ({})",
            cf.path().display(),
            if cf.path().exists() {
                "loaded"
            } else {
                "not created yet; using defaults"
            }
        ),
    );
    if !cf.env_overrides().is_empty() {
        check(
            true,
            false,
            format!("environment overrides: {}", cf.env_overrides().join(", ")),
        );
    }

    let data = cfg.data_dir();
    let writable = std::fs::create_dir_all(&data).is_ok() && tempfile_in(&data).is_ok();
    check(
        writable,
        false,
        format!("data dir {} writable", data.display()),
    );

    match Store::open(&cfg.db_path()) {
        Ok(store) => {
            let counts = store.counts(None).unwrap_or_default();
            check(
                true,
                false,
                format!(
                    "index {} (schema {}, {} files, {} vectors)",
                    cfg.db_path().display(),
                    store.schema_version().unwrap_or(0),
                    counts.files,
                    counts.vectors
                ),
            );
        }
        Err(e) => check(
            false,
            false,
            format!("index {}: {e:#}", cfg.db_path().display()),
        ),
    }

    if cfg.folders.is_empty() {
        check(
            false,
            true,
            "no folders configured (`rummage folders add <path>`)".into(),
        );
    }
    for f in &cfg.folders {
        let p = expand_home(&f.path);
        let readable = std::fs::read_dir(&p).is_ok();
        check(
            readable || !f.enabled,
            !f.enabled,
            format!(
                "folder {}{}",
                p.display(),
                match (readable, f.enabled) {
                    (true, true) => "",
                    (true, false) => " (disabled)",
                    (false, _) => " is missing or unreadable",
                }
            ),
        );
    }

    let url = server.map(String::from).unwrap_or_else(|| local_url(&cfg));
    match Client::new(&url, &cfg.server.auth_token).health() {
        Ok(h) => check(
            true,
            false,
            format!(
                "server running at {url} (version {}, embedder {})",
                h.version, h.embedder.name
            ),
        ),
        Err(_) => check(true, true, format!("no server running at {url}")),
    }

    check(
        true,
        false,
        format!(
            "backends compiled in: {}; configured: {}",
            engine::compiled_backends().join(", "),
            serde_json::to_value(cfg.engine.backend)?
                .as_str()
                .unwrap_or("?")
        ),
    );
    match engine::embedder(&cfg) {
        Ok(e) => check(
            !e.name().starts_with("fake"),
            true,
            format!("embedder: {}", e.name()),
        ),
        Err(e) => check(false, false, format!("embedder: {e:#}")),
    }
    let model_dir = cfg.model_dir();
    for file in [cfg.model_file(), cfg.mmproj_file()] {
        let p = model_dir.join(&file);
        check(
            p.exists(),
            true,
            format!(
                "model file {}{}",
                p.display(),
                if p.exists() {
                    ""
                } else {
                    " not downloaded yet"
                }
            ),
        );
    }

    let devices = rummage_embed::devices();
    for d in &devices {
        check(
            true,
            false,
            format!(
                "device {}: {}{}",
                d.name,
                d.description,
                if d.gpu {
                    format!(
                        " ({} of {} free)",
                        human_bytes(d.memory_free as u64),
                        human_bytes(d.memory_total as u64)
                    )
                } else {
                    String::new()
                }
            ),
        );
    }
    if let Some(warning) = rummage_embed::cpu_fallback_warning(&devices) {
        check(false, true, warning);
    }

    let gpus = gpus();
    check(
        !gpus.is_empty(),
        true,
        if gpus.is_empty() {
            "no GPU found; the CPU backend will be used".into()
        } else {
            format!("GPU: {}", gpus.join("; "))
        },
    );

    let tools = crate::extract::Tools::detect();
    check(
        tools.pdftoppm,
        true,
        if tools.pdftoppm {
            format!(
                "poppler found: PDF pages are indexed as images{}",
                if tools.pdftotext { " and text" } else { "" }
            )
        } else {
            "pdftoppm (poppler-utils) not found; PDFs are indexed by text only".into()
        },
    );
    match crate::extract::ffmpeg().arg("-version").output() {
        Ok(out) if out.status.success() => {
            let first = String::from_utf8_lossy(&out.stdout)
                .lines()
                .next()
                .unwrap_or("ffmpeg")
                .to_string();
            check(true, false, first);
        }
        _ => check(
            false,
            true,
            "ffmpeg not found; videos will be skipped".into(),
        ),
    }

    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn tempfile_in(dir: &Path) -> Result<()> {
    let p = dir.join(format!(".rummage-write-test-{}", std::process::id()));
    std::fs::write(&p, b"").context("write")?;
    std::fs::remove_file(&p)?;
    Ok(())
}

/// GPUs visible to this process, described briefly.
fn gpus() -> Vec<String> {
    let mut out = vec![];
    if let Ok(o) = std::process::Command::new("nvidia-smi").arg("-L").output()
        && o.status.success()
    {
        out.extend(
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| format!("NVIDIA {}", l.trim())),
        );
    }
    if Path::new("/dev/kfd").exists() {
        out.push("AMD ROCm device (/dev/kfd)".into());
    }
    if let Ok(entries) = std::fs::read_dir("/dev/dri") {
        let mut render: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("renderD"))
            .collect();
        render.sort();
        if !render.is_empty() {
            out.push(format!("render nodes {} (Vulkan)", render.join(", ")));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_the_documented_commands() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(std::iter::once("rummage").chain(args.iter().copied()))
        };
        assert!(parse(&["serve", "--bind", "0.0.0.0:8099", "--base-path", "/search"]).is_ok());
        assert!(
            parse(&[
                "folders",
                "add",
                "/tmp",
                "--exclude",
                "*.tmp",
                "--exclude",
                "**/raw/**"
            ])
            .is_ok()
        );
        assert!(parse(&["folders", "rm", "/tmp"]).is_ok());
        assert!(parse(&["folders", "ls"]).is_ok());
        assert!(parse(&["index", "--once"]).is_ok());
        assert!(parse(&["index", "--watch"]).is_ok());
        assert!(parse(&["index", "--once", "--watch"]).is_err());
        assert!(parse(&["index", "--pause"]).is_ok());
        assert!(parse(&["index", "--pause", "--resume"]).is_err());
        assert!(parse(&["search", "--similar", "12"]).is_ok());
        assert!(parse(&["search", "--file", "/tmp/a.jpg", "--type", "image"]).is_ok());
        assert!(parse(&["search"]).is_err());
        assert!(parse(&["search", "x", "--after", "2024-01-01", "--path", "**/a/**"]).is_ok());
        assert!(parse(&["search", "x", "--after", "yesterday"]).is_err());
        assert!(parse(&["status", "--errors"]).is_ok());
        assert!(parse(&["reindex"]).is_ok());
        assert!(parse(&["reindex", "--problems"]).is_ok());
        assert!(parse(&["reindex", "--folder", "/tmp", "--item", "3"]).is_err());
        assert!(
            parse(&[
                "search",
                "red",
                "bike",
                "--type",
                "image,video",
                "--limit",
                "5",
                "--json"
            ])
            .is_ok()
        );
        assert!(parse(&["search", "x", "--type", "nope"]).is_err());
        assert!(parse(&["status", "--json"]).is_ok());
        assert!(parse(&["config", "get", "server.bind"]).is_ok());
        assert!(parse(&["config", "set", "model.dim", "256"]).is_ok());
        assert!(parse(&["config", "path"]).is_ok());
        assert!(parse(&["doctor"]).is_ok());
        assert!(parse(&["status", "--local", "--server", "http://x"]).is_err());
    }

    #[test]
    fn clock_times() {
        assert_eq!(clock(134.4), "2:14");
        assert_eq!(clock(3725.0), "1:02:05");
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human_bytes(10), "10 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
    }
}
