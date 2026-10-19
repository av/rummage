//! File watching with inotify (through notify). Every indexed directory gets its own
//! non-recursive watch, so skipped trees like node_modules cost no watches.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use anyhow::Result;
use notify::event::{AccessKind, AccessMode, EventKind};
use notify::{RecursiveMode, Watcher as _};

use super::Msg;

pub struct Watcher {
    inner: notify::RecommendedWatcher,
    watched: HashSet<PathBuf>,
    warned: bool,
}

impl Watcher {
    pub fn new(tx: Sender<Msg>) -> Result<Self> {
        let inner =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(ev) if relevant(&ev.kind) && !ev.paths.is_empty() => {
                    let _ = tx.send(Msg::Fs(ev.paths));
                }
                Ok(_) => {}
                Err(e) => tracing::debug!("watch error: {e}"),
            })?;
        Ok(Self {
            inner,
            watched: HashSet::new(),
            warned: false,
        })
    }

    /// Watches a directory (not its subdirectories).
    pub fn add(&mut self, dir: &Path) {
        if self.watched.contains(dir) {
            return;
        }
        match self.inner.watch(dir, RecursiveMode::NonRecursive) {
            Ok(()) => {
                self.watched.insert(dir.to_path_buf());
            }
            Err(e) => {
                if !self.warned {
                    tracing::warn!(
                        "can't watch {} ({e}); changes there are picked up by the periodic \
                         rescan (raise fs.inotify.max_user_watches if you hit the limit)",
                        dir.display()
                    );
                    self.warned = true;
                }
            }
        }
    }

    /// Forgets directories that no longer exist (the kernel already dropped their watches).
    pub fn prune(&mut self) {
        self.watched.retain(|d| d.is_dir());
    }

    pub fn len(&self) -> usize {
        self.watched.len()
    }
}

fn relevant(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    }
}
