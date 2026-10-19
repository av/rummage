//! Turns files into pieces to embed (passages, pages, images, audio and video windows), each
//! with a locator, plus metadata and thumbnails.

pub mod audio;
pub mod image;
pub mod pdf;
pub mod render;
pub mod text;
pub mod thumb;
pub mod upload;
pub mod video;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use rummage_core::EmbedInput;
use rummage_core::store::ChunkRow;
use rummage_core::types::Kind;

/// One thing to embed and where it sits in its file.
pub struct Piece {
    /// The chunk to store (its `file_id` is filled in by the caller).
    pub chunk: ChunkRow,
    pub input: EmbedInput,
}

/// What extraction learned about a file besides its pieces.
#[derive(Default)]
pub struct Meta {
    /// When the photo or video was taken, if known.
    pub date: Option<i64>,
    pub exif: Option<serde_json::Value>,
    /// The thumbnail (WebP) at the configured size.
    pub thumb: Option<Vec<u8>>,
    /// Extra renderings to cache: `(variant, WebP bytes)` under `previews/<id>/`.
    pub previews: Vec<(String, Vec<u8>)>,
    /// For sound: duration in seconds and peak levels (0 to 1), for `/items/{id}/waveform`.
    pub peaks: Option<(f64, Vec<f32>)>,
}

/// How a file went.
pub enum Outcome {
    Done(Meta),
    /// The file can't be indexed here, and why (e.g. no ffmpeg).
    Skipped(String),
}

/// What an extractor needs to know.
pub struct Ctx<'a> {
    pub tools: &'a Tools,
    /// Longest thumbnail edge.
    pub thumb_size: u32,
}

/// Receives pieces as they are made, so long files stream instead of piling up in memory.
pub type Sink<'a> = dyn FnMut(Piece) -> Result<()> + 'a;

/// Extracts `path` as `kind`, sending pieces to `sink`.
pub fn extract(path: &Path, kind: Kind, ctx: &Ctx, sink: &mut Sink) -> Result<Outcome> {
    match kind {
        Kind::Image => image::extract(path, ctx, sink),
        Kind::Text => text::extract(path, sink),
        Kind::Doc => pdf::extract(path, ctx, sink),
        Kind::Audio => audio::extract(path, ctx, sink),
        Kind::Video => video::extract(path, ctx, sink),
    }
}

/// Image extensions the image crate decodes.
pub const IMAGE_EXT: &[&str] = &[
    "jpg", "jpeg", "jpe", "png", "webp", "gif", "bmp", "tif", "tiff",
];
/// Image extensions decoded through ffmpeg (or, for RAW, their embedded preview).
pub const FFMPEG_IMAGE_EXT: &[&str] = &["heic", "heif", "avif"];
pub const RAW_EXT: &[&str] = &[
    "cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2", "pef", "srw",
];
pub const AUDIO_EXT: &[&str] = &["wav", "mp3", "flac", "ogg", "oga", "m4a", "aac"];
pub const VIDEO_EXT: &[&str] = &["mp4", "m4v", "mkv", "webm", "mov", "avi"];
pub const DOC_EXT: &[&str] = &["pdf"];
/// Text, markup, data and source code.
pub const TEXT_EXT: &[&str] = &[
    "txt", "text", "md", "markdown", "mdx", "rst", "org", "adoc", "tex", "csv", "tsv", "json",
    "jsonl", "ndjson", "yaml", "yml", "toml", "xml", "html", "htm", "xhtml", "ini", "cfg", "conf",
    "env", "log", "rs", "py", "pyi", "js", "mjs", "cjs", "ts", "tsx", "jsx", "go", "java", "kt",
    "kts", "scala", "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "cs", "rb", "php", "swift", "m",
    "mm", "sh", "bash", "zsh", "fish", "ps1", "lua", "r", "jl", "sql", "css", "scss", "sass",
    "less", "vue", "svelte", "astro", "dart", "ex", "exs", "erl", "hrl", "hs", "ml", "mli", "fs",
    "fsx", "clj", "cljs", "edn", "nix", "tf", "hcl", "proto", "graphql", "gql", "gradle", "cmake",
    "zig", "v", "nim", "pl", "pm", "el", "lisp", "scm", "rkt", "bat", "ipynb",
];
/// Files known by their whole name.
const TEXT_NAMES: &[&str] = &[
    "makefile",
    "dockerfile",
    "containerfile",
    "justfile",
    "readme",
    "license",
    "copying",
    "changelog",
    "authors",
    "notice",
    "todo",
    "gemfile",
    "rakefile",
    "procfile",
    "vagrantfile",
];

/// What kind of file this is, by its name; `None` for files rummage doesn't index.
pub fn kind_of(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let Some((_, ext)) = name.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()) else {
        return TEXT_NAMES.contains(&name.as_str()).then_some(Kind::Text);
    };
    let has = |list: &[&str]| list.contains(&ext);
    if has(IMAGE_EXT) || has(FFMPEG_IMAGE_EXT) || has(RAW_EXT) {
        Some(Kind::Image)
    } else if has(AUDIO_EXT) {
        Some(Kind::Audio)
    } else if has(VIDEO_EXT) {
        Some(Kind::Video)
    } else if has(DOC_EXT) {
        Some(Kind::Doc)
    } else if has(TEXT_EXT) {
        Some(Kind::Text)
    } else {
        None
    }
}

/// Lowercased extension.
pub fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// The outside programs rummage can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    /// ffmpeg and ffprobe: video, and HEIC/AVIF photos.
    pub ffmpeg: bool,
    /// poppler's pdftoppm: PDF pages as images.
    pub pdftoppm: bool,
    /// poppler's pdftotext: PDF text (a built-in reader is used without it).
    pub pdftotext: bool,
}

/// The ffmpeg and ffprobe programs, from `tools.ffmpeg` or `PATH`.
static FFMPEG: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();

/// Uses the ffmpeg at `path` (`tools.ffmpeg`), and the ffprobe in the same folder; empty keeps
/// the `PATH` lookup. Takes effect only before the first use of either program.
pub fn set_ffmpeg(path: &str) {
    let _ = FFMPEG.set(ffmpeg_programs(path));
}

fn ffmpeg_programs(path: &str) -> (PathBuf, PathBuf) {
    let path = path.trim();
    if path.is_empty() {
        return ("ffmpeg".into(), "ffprobe".into());
    }
    let ffmpeg = rummage_core::config::expand_home(path);
    let ffprobe = match ffmpeg.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join("ffprobe"),
        _ => "ffprobe".into(),
    };
    (ffmpeg, ffprobe)
}

/// A command running ffmpeg.
pub fn ffmpeg() -> Command {
    Command::new(&FFMPEG.get_or_init(|| ffmpeg_programs("")).0)
}

/// A command running ffprobe.
pub fn ffprobe() -> Command {
    Command::new(&FFMPEG.get_or_init(|| ffmpeg_programs("")).1)
}

impl Tools {
    pub fn detect() -> Tools {
        Tools {
            ffmpeg: works(ffmpeg().arg("-version")) && works(ffprobe().arg("-version")),
            pdftoppm: runs("pdftoppm", "-v"),
            pdftotext: runs("pdftotext", "-v"),
        }
    }

    /// Detected once per process, for on-demand previews.
    pub fn cached() -> &'static Tools {
        static TOOLS: OnceLock<Tools> = OnceLock::new();
        TOOLS.get_or_init(Tools::detect)
    }

    /// Notes about missing tools, for the status page.
    pub fn messages(&self) -> Vec<String> {
        let mut out = vec![];
        if !self.ffmpeg {
            out.push(
                "ffmpeg not found: videos (and HEIC/AVIF photos) are skipped; install ffmpeg and \
                 rescan to index them"
                    .into(),
            );
        }
        if !self.pdftoppm {
            out.push(
                "pdftoppm (poppler-utils) not found: PDFs are indexed by their text only, \
                 without page images"
                    .into(),
            );
        }
        out
    }

    /// Whether files of `kind` at `path` can be indexed with these tools.
    pub fn can_index(&self, path: &Path) -> bool {
        let e = ext(path);
        !(VIDEO_EXT.contains(&e.as_str()) || FFMPEG_IMAGE_EXT.contains(&e.as_str())) || self.ffmpeg
    }
}

fn runs(program: &str, arg: &str) -> bool {
    works(Command::new(program).arg(arg))
}

fn works(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Runs a program and returns its stdout; fails with its stderr.
pub fn run(cmd: &mut Command) -> Result<Vec<u8>> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {name}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        bail!("{name} failed ({}): {last}", out.status);
    }
    Ok(out.stdout)
}

/// A temporary directory removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Result<TempDir> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "rummage-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).with_context(|| format!("creating {}", p.display()))?;
        Ok(TempDir(p))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_name() {
        let k = |p: &str| kind_of(Path::new(p));
        assert_eq!(k("/a/IMG_1.JPG"), Some(Kind::Image));
        assert_eq!(k("/a/x.heic"), Some(Kind::Image));
        assert_eq!(k("/a/song.flac"), Some(Kind::Audio));
        assert_eq!(k("/a/clip.MOV"), Some(Kind::Video));
        assert_eq!(k("/a/paper.pdf"), Some(Kind::Doc));
        assert_eq!(k("/a/notes.mdx"), Some(Kind::Text));
        assert_eq!(k("/a/main.rs"), Some(Kind::Text));
        assert_eq!(k("/a/Makefile"), Some(Kind::Text));
        assert_eq!(k("/a/fedora.iso"), None);
        assert_eq!(k("/a/pkg.rpm"), None);
        assert_eq!(k("/a/project.flp"), None);
        assert_eq!(k("/a/.bashrc"), None);
        assert_eq!(k("/a/noext"), None);
    }

    #[test]
    fn ffmpeg_from_config() {
        let p = |s: &str| ffmpeg_programs(s);
        assert_eq!(p(""), ("ffmpeg".into(), "ffprobe".into()));
        assert_eq!(
            p("/opt/ff/bin/ffmpeg"),
            ("/opt/ff/bin/ffmpeg".into(), "/opt/ff/bin/ffprobe".into())
        );
        assert_eq!(p("ffmpeg7"), ("ffmpeg7".into(), "ffprobe".into()));
    }
}
