//! Walking folders and matching what is on disk against the index: new, changed (size and
//! mtime, then content hash), moved or renamed (same hash at a path that vanished), deleted.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use rummage_core::config::{Config, FolderConfig, expand_home};
use rummage_core::store::{FileRow, NewFile, ScanRow};
use rummage_core::types::Kind;
use walkdir::WalkDir;

use super::watch::Watcher;
use super::{Priority, Shared};
use crate::extract::kind_of;

/// Directory names always skipped, on top of `index.skip`.
pub const ALWAYS_SKIP: &[&str] = &[".git", "node_modules", "__pycache__"];

/// A watched folder and its include/exclude rules.
pub struct FolderRules {
    pub root: PathBuf,
    /// `root` as stored in the index's `folder` column.
    pub key: String,
    include: Option<GlobSet>,
    exclude: GlobSet,
    skip: HashSet<String>,
    /// Roots of other folders inside this one; they are walked as their own folders.
    nested: Vec<PathBuf>,
}

impl FolderRules {
    pub fn new(f: &FolderConfig, skip: &[String]) -> Result<Self> {
        let root = expand_home(&f.path);
        let set = |globs: &[String]| -> Result<GlobSet> {
            let mut b = GlobSetBuilder::new();
            for g in globs {
                b.add(Glob::new(g).with_context(|| format!("bad glob {g:?}"))?);
            }
            Ok(b.build()?)
        };
        Ok(Self {
            key: root.to_string_lossy().into_owned(),
            root,
            include: (!f.include.is_empty())
                .then(|| set(&f.include))
                .transpose()?,
            exclude: set(&f.exclude)?,
            skip: skip
                .iter()
                .map(String::as_str)
                .chain(ALWAYS_SKIP.iter().copied())
                .map(String::from)
                .collect(),
            nested: vec![],
        })
    }

    /// The enabled folders of a config.
    pub fn all(cfg: &Config) -> Vec<FolderRules> {
        let mut all: Vec<FolderRules> = cfg
            .folders
            .iter()
            .filter(|f| f.enabled)
            .filter_map(|f| match FolderRules::new(f, &cfg.index.skip) {
                Ok(r) => Some(r),
                Err(e) => {
                    tracing::warn!("folder {}: {e:#}", f.path);
                    None
                }
            })
            .collect();
        let roots: Vec<PathBuf> = all.iter().map(|f| f.root.clone()).collect();
        for f in &mut all {
            f.nested = roots
                .iter()
                .filter(|r| **r != f.root && r.starts_with(&f.root))
                .cloned()
                .collect();
        }
        all
    }

    fn rel<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        path.strip_prefix(&self.root).ok()
    }

    /// Whether a directory (relative to the root) is skipped with everything in it.
    pub fn skips_dir(&self, rel: &Path) -> bool {
        if rel.as_os_str().is_empty() {
            return false;
        }
        rel.components()
            .any(|c| self.skip.contains(c.as_os_str().to_string_lossy().as_ref()))
            || self.exclude.is_match(rel)
            || self.exclude.is_match(rel.join("\u{0}"))
    }

    /// Whether a file (relative to the root) is indexed.
    pub fn wants_file(&self, rel: &Path) -> bool {
        let in_skipped_dir = rel.parent().is_some_and(|p| {
            p.components()
                .any(|c| self.skip.contains(c.as_os_str().to_string_lossy().as_ref()))
        });
        !in_skipped_dir
            && !self.exclude.is_match(rel)
            && self.include.as_ref().is_none_or(|i| i.is_match(rel))
    }
}

/// The folder a path belongs to (the deepest root containing it).
pub fn folder_for<'a>(folders: &'a [FolderRules], path: &Path) -> Option<&'a FolderRules> {
    folders
        .iter()
        .filter(|f| path.starts_with(&f.root))
        .max_by_key(|f| f.root.as_os_str().len())
}

/// A file on disk with no row in the index yet.
struct Found {
    path: String,
    folder: String,
    size: i64,
    mtime: i64,
    kind: Kind,
}

/// Collects what a walk or an event batch found.
struct Pass<'a> {
    sh: &'a Shared,
    prio: Priority,
    seen: HashSet<String>,
    new: Vec<Found>,
    /// Directories that could not be read: their indexed files are not treated as deleted.
    unreadable: Vec<String>,
}

impl<'a> Pass<'a> {
    fn new(sh: &'a Shared, prio: Priority) -> Self {
        Self {
            sh,
            prio,
            seen: HashSet::new(),
            new: vec![],
            unreadable: vec![],
        }
    }

    /// Walks `dir` (inside folder `f`), comparing files with `rows` (keyed by path).
    fn walk(
        &mut self,
        f: &FolderRules,
        dir: &Path,
        rows: &HashMap<String, ScanRow>,
        mut watcher: Option<&mut Watcher>,
    ) {
        let walker = WalkDir::new(dir)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                !e.file_type().is_dir()
                    || (f.rel(e.path()).is_none_or(|r| !f.skips_dir(r))
                        && !f.nested.iter().any(|n| n == e.path()))
            });
        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    if let Some(p) = e.path() {
                        tracing::debug!("can't read {}: {e}", p.display());
                        self.unreadable.push(p.to_string_lossy().into_owned());
                    }
                    continue;
                }
            };
            let ft = entry.file_type();
            if ft.is_dir() {
                if let Some(w) = watcher.as_deref_mut() {
                    w.add(entry.path());
                }
                continue;
            }
            if !ft.is_file() {
                continue; // symlinks and special files
            }
            self.file(f, entry.path(), entry.metadata().ok(), |p| {
                rows.get(p).cloned()
            });
        }
    }

    /// Looks at one file; `row` finds its index row by path.
    fn file(
        &mut self,
        f: &FolderRules,
        path: &Path,
        meta: Option<std::fs::Metadata>,
        row: impl FnOnce(&str) -> Option<ScanRow>,
    ) {
        let Some(rel) = f.rel(path) else {
            return;
        };
        if !f.wants_file(rel) {
            return;
        }
        let Some(kind) = kind_of(path) else {
            return;
        };
        let Some(meta) = meta else {
            return;
        };
        let path_str = path.to_string_lossy().into_owned();
        let (size, mtime) = (meta.len() as i64, mtime(&meta));
        self.seen.insert(path_str.clone());
        let result = match row(&path_str) {
            Some(r) => self.known(f, &path_str, size, mtime, kind, &r),
            None => {
                self.new.push(Found {
                    path: path_str,
                    folder: f.key.clone(),
                    size,
                    mtime,
                    kind,
                });
                Ok(())
            }
        };
        if let Err(e) = result {
            tracing::warn!("{}: {e:#}", path.display());
        }
    }

    /// A file the index already has: queue it if it changed.
    fn known(
        &mut self,
        f: &FolderRules,
        path: &str,
        size: i64,
        mtime: i64,
        kind: Kind,
        r: &ScanRow,
    ) -> Result<()> {
        let store = self.sh.svc.store();
        if r.folder != f.key {
            store.move_file(r.id, path, &f.key)?;
        }
        if r.size == size && r.mtime == mtime {
            match r.status.as_str() {
                "pending" => self.sh.enqueue(r.id, self.prio),
                "skipped" if self.sh.tools().can_index(Path::new(path)) => {
                    store.set_status(r.id, "pending", None)?;
                    self.sh.enqueue(r.id, self.prio);
                }
                _ => {}
            }
            return Ok(());
        }
        // Same size, new mtime: maybe only touched.
        if r.size == size
            && r.status == "indexed"
            && let Some(old) = &r.hash
            && hash_file(Path::new(path)).ok().as_ref() == Some(old)
        {
            store.touch_file(r.id, size, mtime)?;
            return Ok(());
        }
        let id = store.upsert_file(&new_file(path, &f.key, size, mtime, kind))?;
        self.sh.enqueue(id, self.prio);
        Ok(())
    }

    /// Matches new files against vanished ones (moves keep their vectors), adds the rest,
    /// and drops what is gone.
    fn finish(self, missing: Vec<ScanRow>) -> Result<()> {
        let store = self.sh.svc.store();
        let missing: Vec<ScanRow> = missing
            .into_iter()
            .filter(|r| !self.seen.contains(&r.path))
            .filter(|r| {
                !self
                    .unreadable
                    .iter()
                    .any(|d| r.path == *d || r.path.starts_with(&format!("{d}/")))
            })
            .collect();
        let mut by_size: HashMap<i64, Vec<&ScanRow>> = HashMap::new();
        for r in missing.iter().filter(|r| r.hash.is_some()) {
            by_size.entry(r.size).or_default().push(r);
        }
        let mut moved = HashSet::new();
        for f in self.new {
            if let Some(candidates) = by_size.get_mut(&f.size)
                && let Ok(h) = hash_file(Path::new(&f.path))
                && let Some(pos) = candidates
                    .iter()
                    .position(|r| r.hash.as_deref() == Some(h.as_str()))
            {
                let r = candidates.remove(pos);
                store.move_file(r.id, &f.path, &f.folder)?;
                store.touch_file(r.id, f.size, f.mtime)?;
                if r.status == "pending" {
                    self.sh.enqueue(r.id, self.prio);
                }
                moved.insert(r.id);
                tracing::debug!("moved {} -> {}", r.path, f.path);
                continue;
            }
            let id = store.upsert_file(&new_file(&f.path, &f.folder, f.size, f.mtime, f.kind))?;
            self.sh.enqueue(id, self.prio);
        }
        for r in missing.iter().filter(|r| !moved.contains(&r.id)) {
            tracing::debug!("gone: {}", r.path);
            self.sh.svc.remove_file(r.id)?;
            self.sh.dequeue(r.id);
        }
        Ok(())
    }
}

/// Walks `folders` (or just the one whose root is `only`) and brings the index in line.
pub fn scan(
    sh: &Shared,
    folders: &[FolderRules],
    only: Option<&str>,
    prio: Priority,
    mut watcher: Option<&mut Watcher>,
) -> Result<()> {
    let rows: HashMap<String, ScanRow> = sh
        .svc
        .store()
        .scan_rows()?
        .into_iter()
        .map(|r| (r.path.clone(), r))
        .collect();
    let mut pass = Pass::new(sh, prio);
    let mut scanned: Vec<&str> = vec![];
    for f in folders.iter().filter(|f| only.is_none_or(|o| o == f.key)) {
        // A missing or unreadable root (an unmounted disk) must not look like deleted files.
        if std::fs::read_dir(&f.root).is_err() {
            tracing::warn!("folder {} is missing or unreadable; skipping it", f.key);
            continue;
        }
        sh.svc.live().scanning.insert(f.key.clone());
        sh.svc.publish_status();
        pass.walk(f, &f.root, &rows, watcher.as_deref_mut());
        scanned.push(&f.key);
    }
    // Files of a folder nested in another belong to the inner one; both may be scanned.
    let missing: Vec<ScanRow> = rows
        .into_values()
        .filter(|r| scanned.contains(&r.folder.as_str()))
        .collect();
    let result = pass.finish(missing);
    let mut live = sh.svc.live();
    for k in &scanned {
        live.scanning.remove(*k);
    }
    drop(live);
    result
}

/// Handles a batch of changed paths reported by the watcher.
pub fn apply_events(
    sh: &Shared,
    folders: &[FolderRules],
    paths: HashSet<PathBuf>,
    mut watcher: Option<&mut Watcher>,
) -> Result<()> {
    let store = sh.svc.store();
    let mut pass = Pass::new(sh, Priority::Live);
    let mut missing: Vec<ScanRow> = vec![];
    let mut paths: Vec<PathBuf> = paths.into_iter().collect();
    paths.sort();
    for p in paths {
        let Some(f) = folder_for(folders, &p) else {
            continue;
        };
        let p_str = p.to_string_lossy().into_owned();
        match std::fs::symlink_metadata(&p) {
            Ok(m) if m.is_dir() => {
                if f.rel(&p).is_some_and(|r| f.skips_dir(r)) {
                    continue;
                }
                let rows: HashMap<String, ScanRow> = store
                    .scan_rows_under(&p_str)?
                    .into_iter()
                    .map(|r| (r.path.clone(), r))
                    .collect();
                pass.walk(f, &p, &rows, watcher.as_deref_mut());
                missing.extend(rows.into_values());
            }
            Ok(m) if m.is_file() => {
                let row = store.file_by_path(&p_str)?.map(scan_row);
                let before = pass.seen.len();
                pass.file(f, &p, Some(m), |_| row.clone());
                if pass.seen.len() == before
                    && let Some(r) = row
                {
                    // Now excluded (or no longer a kind we index).
                    missing.push(r);
                }
            }
            Ok(_) => {}
            Err(_) => missing.extend(store.scan_rows_under(&p_str)?),
        }
    }
    pass.finish(missing)
}

fn scan_row(r: FileRow) -> ScanRow {
    ScanRow {
        id: r.id,
        path: r.path,
        folder: r.folder,
        size: r.size,
        mtime: r.mtime,
        hash: r.hash,
        kind: r.kind,
        status: r.status,
    }
}

fn new_file(path: &str, folder: &str, size: i64, mtime: i64, kind: Kind) -> NewFile {
    NewFile {
        path: path.to_string(),
        folder: folder.to_string(),
        size,
        mtime,
        kind,
        mime: mime_guess::from_path(path).first_raw().map(String::from),
        ..Default::default()
    }
}

/// Modification time in Unix seconds.
pub fn mtime(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// BLAKE3 of a file's content, hex.
pub fn hash_file(path: &Path) -> Result<String> {
    let mut h = blake3::Hasher::new();
    h.update_reader(std::fs::File::open(path)?)?;
    Ok(h.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(include: &[&str], exclude: &[&str]) -> FolderRules {
        FolderRules::new(
            &FolderConfig {
                path: "/data".into(),
                include: include.iter().map(|s| s.to_string()).collect(),
                exclude: exclude.iter().map(|s| s.to_string()).collect(),
                enabled: true,
            },
            &Config::default().index.skip,
        )
        .unwrap()
    }

    #[test]
    fn rules_skip_dirs_and_match_globs() {
        let r = rules(&[], &["Screenshots/**", "**/*.tmp"]);
        assert!(r.skips_dir(Path::new("node_modules")));
        assert!(r.skips_dir(Path::new("a/.git")));
        assert!(r.skips_dir(Path::new("x/build")));
        assert!(r.skips_dir(Path::new("Screenshots")));
        assert!(!r.skips_dir(Path::new("Holiday")));
        assert!(!r.skips_dir(Path::new("")));
        assert!(r.wants_file(Path::new("Holiday/a.jpg")));
        assert!(!r.wants_file(Path::new("Holiday/a.tmp")));
        assert!(!r.wants_file(Path::new("Screenshots/a.png")));
        assert!(!r.wants_file(Path::new("app/node_modules/x.js")));
        let only_md = rules(&["**/*.md"], &[]);
        assert!(only_md.wants_file(Path::new("notes/a.md")));
        assert!(only_md.wants_file(Path::new("a.md")));
        assert!(!only_md.wants_file(Path::new("a.txt")));
    }

    #[test]
    fn deepest_folder_wins() {
        let outer = rules(&[], &[]);
        let mut inner = rules(&[], &[]);
        inner.root = PathBuf::from("/data/inner");
        let all = [outer, inner];
        assert_eq!(
            folder_for(&all, Path::new("/data/inner/x.md"))
                .unwrap()
                .root,
            PathBuf::from("/data/inner")
        );
        assert_eq!(
            folder_for(&all, Path::new("/data/x.md")).unwrap().root,
            PathBuf::from("/data")
        );
        assert!(folder_for(&all, Path::new("/elsewhere/x.md")).is_none());
        assert!(folder_for(&all, Path::new("/database/x.md")).is_none());
    }
}
