//! The SQLite index: files, chunks, vectors, a keyword index and settings, in one WAL file.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

use crate::types::{Kind, SegmentKind};

/// Schema migrations, applied in order. `PRAGMA user_version` holds how many have run.
/// Never edit a released entry; append a new one.
const MIGRATIONS: &[&str] = &[
    // 1: the initial schema.
    r#"
CREATE TABLE files (
    id          INTEGER PRIMARY KEY,
    path        TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    folder      TEXT NOT NULL,
    size        INTEGER NOT NULL,
    mtime       INTEGER NOT NULL,
    date        INTEGER,
    hash        TEXT,
    kind        TEXT NOT NULL,
    mime        TEXT,
    exif        TEXT,
    status      TEXT NOT NULL DEFAULT 'pending',
    error       TEXT,
    indexed_at  INTEGER
);
CREATE INDEX files_folder ON files(folder);
CREATE INDEX files_kind ON files(kind);
CREATE INDEX files_status ON files(status);
CREATE INDEX files_hash ON files(hash);

CREATE TABLE chunks (
    id       INTEGER PRIMARY KEY,
    file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    kind     TEXT NOT NULL,
    page     INTEGER,
    t_start  REAL,
    t_end    REAL,
    text     TEXT
);
CREATE INDEX chunks_file ON chunks(file_id);

CREATE TABLE vectors (
    chunk_id  INTEGER PRIMARY KEY REFERENCES chunks(id) ON DELETE CASCADE,
    dim       INTEGER NOT NULL,
    blob      BLOB NOT NULL
);

-- One row per chunk (rowid = chunk id), kept in step with files and chunks by triggers.
CREATE VIRTUAL TABLE fts USING fts5(
    name, path, text, exif,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
    INSERT INTO fts(rowid, name, path, text, exif)
    SELECT new.id, f.name, f.path, coalesce(new.text, ''), coalesce(f.exif, '')
    FROM files f WHERE f.id = new.file_id;
END;
CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
    DELETE FROM fts WHERE rowid = old.id;
END;
CREATE TRIGGER chunks_au AFTER UPDATE OF text ON chunks BEGIN
    UPDATE fts SET text = coalesce(new.text, '') WHERE rowid = new.id;
END;
CREATE TRIGGER files_au AFTER UPDATE OF name, path, exif ON files BEGIN
    UPDATE fts SET name = new.name, path = new.path, exif = coalesce(new.exif, '')
    WHERE rowid IN (SELECT id FROM chunks WHERE file_id = new.id);
END;

CREATE TABLE settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);
"#,
];

/// Number of the newest schema migration.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// A file row.
#[derive(Debug, Clone, PartialEq)]
pub struct FileRow {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub folder: String,
    pub size: i64,
    pub mtime: i64,
    pub date: Option<i64>,
    pub hash: Option<String>,
    pub kind: Kind,
    pub mime: Option<String>,
    pub exif: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub indexed_at: Option<i64>,
}

/// What the indexer knows about a file before inserting it.
#[derive(Debug, Clone, Default)]
pub struct NewFile {
    pub path: String,
    pub folder: String,
    pub size: i64,
    pub mtime: i64,
    pub date: Option<i64>,
    pub hash: Option<String>,
    pub kind: Kind,
    pub mime: Option<String>,
    pub exif: Option<String>,
}

/// A chunk row: one embeddable piece of a file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChunkRow {
    pub id: i64,
    pub file_id: i64,
    pub kind: SegmentKind,
    pub page: Option<i64>,
    pub t_start: Option<f64>,
    pub t_end: Option<f64>,
    pub text: Option<String>,
}

/// What the scanner compares against the disk.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanRow {
    pub id: i64,
    pub path: String,
    pub folder: String,
    pub size: i64,
    pub mtime: i64,
    pub hash: Option<String>,
    pub kind: Kind,
    pub status: String,
}

/// A file's metadata after extraction, written together with its chunks.
#[derive(Debug, Clone, Default)]
pub struct FileContent {
    pub size: i64,
    pub mtime: i64,
    pub hash: Option<String>,
    pub date: Option<i64>,
    pub mime: Option<String>,
    pub exif: Option<String>,
    /// `indexed`, `skipped` or `error`.
    pub status: String,
    pub error: Option<String>,
}

/// Counts of files by status, plus chunk/vector totals.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Counts {
    pub files: u64,
    pub indexed: u64,
    pub pending: u64,
    pub errors: u64,
    pub skipped: u64,
    pub bytes: u64,
    pub chunks: u64,
    pub vectors: u64,
}

/// Which files to act on.
#[derive(Debug, Clone)]
pub enum Scope<'a> {
    All,
    Folder(&'a str),
    File(i64),
    /// Files that failed or were skipped.
    Problems,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Opens (creating if needed) the database at `path` and migrates it.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    /// An in-memory store, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn migrate(&self) -> Result<()> {
        let mut conn = self.conn();
        let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        anyhow::ensure!(
            current <= SCHEMA_VERSION,
            "the index was created by a newer rummage (schema {current}, this build knows {SCHEMA_VERSION})"
        );
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)
                .with_context(|| format!("schema migration {}", i + 1))?;
            tx.pragma_update(None, "user_version", (i + 1) as u32)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .conn()
            .pragma_query_value(None, "user_version", |r| r.get(0))?)
    }

    /// Inserts a file, or updates the row with the same path. Returns its id.
    pub fn upsert_file(&self, f: &NewFile) -> Result<i64> {
        let name = Path::new(&f.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| f.path.clone());
        let conn = self.conn();
        Ok(conn.query_row(
            "INSERT INTO files (path, name, folder, size, mtime, date, hash, kind, mime, exif, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending')
             ON CONFLICT(path) DO UPDATE SET
                name = excluded.name, folder = excluded.folder, size = excluded.size,
                mtime = excluded.mtime, date = excluded.date, hash = excluded.hash,
                kind = excluded.kind, mime = excluded.mime, exif = excluded.exif,
                status = 'pending', error = NULL
             RETURNING id",
            params![
                f.path,
                name,
                f.folder,
                f.size,
                f.mtime,
                f.date,
                f.hash,
                f.kind.as_str(),
                f.mime,
                f.exif
            ],
            |r| r.get(0),
        )?)
    }

    /// Sets a file's status (`pending`, `indexed`, `error`, `skipped`) and error text.
    pub fn set_status(&self, file_id: i64, status: &str, error: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE files SET status = ?2, error = ?3,
               indexed_at = CASE WHEN ?2 = 'indexed' THEN unixepoch() ELSE indexed_at END
             WHERE id = ?1",
            params![file_id, status, error],
        )?;
        Ok(())
    }

    /// Moves a file to a new path, keeping its chunks and vectors.
    pub fn rename_file(&self, file_id: i64, new_path: &str) -> Result<()> {
        let name = Path::new(new_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| new_path.to_string());
        self.conn().execute(
            "UPDATE files SET path = ?2, name = ?3 WHERE id = ?1",
            params![file_id, new_path, name],
        )?;
        Ok(())
    }

    /// Moves a file to another path and folder, keeping its chunks and vectors.
    pub fn move_file(&self, file_id: i64, new_path: &str, folder: &str) -> Result<()> {
        let name = Path::new(new_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| new_path.to_string());
        self.conn().execute(
            "UPDATE files SET path = ?2, name = ?3, folder = ?4 WHERE id = ?1",
            params![file_id, new_path, name, folder],
        )?;
        Ok(())
    }

    /// Records a new size and mtime for a file whose content did not change.
    pub fn touch_file(&self, file_id: i64, size: i64, mtime: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE files SET size = ?2, mtime = ?3 WHERE id = ?1",
            params![file_id, size, mtime],
        )?;
        Ok(())
    }

    /// Replaces a file's metadata, chunks and vectors in one transaction.
    /// Returns the new chunk ids, or `None` when the file is no longer in the index.
    pub fn replace_content(
        &self,
        file_id: i64,
        content: &FileContent,
        chunks: &[(ChunkRow, Vec<f32>)],
    ) -> Result<Option<Vec<i64>>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        // The file row goes first: the keyword index copies its EXIF when chunks are inserted.
        let updated = tx.execute(
            "UPDATE files SET size = ?2, mtime = ?3, hash = ?4, date = ?5, mime = coalesce(?6, mime),
                exif = ?7, status = ?8, error = ?9,
                indexed_at = CASE WHEN ?8 = 'pending' THEN indexed_at ELSE unixepoch() END
             WHERE id = ?1",
            params![
                file_id,
                content.size,
                content.mtime,
                content.hash,
                content.date,
                content.mime,
                content.exif,
                content.status,
                content.error
            ],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        tx.execute("DELETE FROM chunks WHERE file_id = ?1", [file_id])?;
        let mut ids = Vec::with_capacity(chunks.len());
        {
            let mut chunk_stmt = tx.prepare_cached(
                "INSERT INTO chunks (file_id, kind, page, t_start, t_end, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            let mut vec_stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO vectors (chunk_id, dim, blob) VALUES (?1, ?2, ?3)",
            )?;
            for (c, v) in chunks {
                chunk_stmt.execute(params![
                    file_id,
                    c.kind.as_str(),
                    c.page,
                    c.t_start,
                    c.t_end,
                    c.text
                ])?;
                let id = tx.last_insert_rowid();
                let blob: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
                vec_stmt.execute(params![id, v.len() as i64, blob])?;
                ids.push(id);
            }
        }
        tx.commit()?;
        Ok(Some(ids))
    }

    /// Every file row, as the scanner needs it.
    pub fn scan_rows(&self) -> Result<Vec<ScanRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, path, folder, size, mtime, hash, kind, status FROM files",
        )?;
        let rows = stmt.query_map([], scan_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The file at `path` and every file below it (when `path` is a directory).
    pub fn scan_rows_under(&self, path: &str) -> Result<Vec<ScanRow>> {
        let dir = format!("{}/", path.trim_end_matches('/'));
        // Paths below `dir` sort between `dir` and `dir` with its last '/' replaced by '0'.
        let end = format!("{}0", path.trim_end_matches('/'));
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, path, folder, size, mtime, hash, kind, status FROM files
             WHERE path = ?1 OR (path >= ?2 AND path < ?3)",
        )?;
        let rows = stmt.query_map(params![path, dir, end], scan_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Ids of files waiting to be indexed, oldest first.
    pub fn pending_files(&self) -> Result<Vec<i64>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare_cached("SELECT id FROM files WHERE status = 'pending' ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Files that failed or were skipped, newest first, with the total count.
    pub fn problem_files(&self, limit: usize, offset: usize) -> Result<(u64, Vec<FileRow>)> {
        let conn = self.conn();
        let total: i64 = conn.query_row(
            "SELECT count(*) FROM files WHERE status IN ('error', 'skipped')",
            [],
            |r| r.get(0),
        )?;
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {FILE_COLS} FROM files WHERE status IN ('error', 'skipped')
             ORDER BY coalesce(indexed_at, 0) DESC, id DESC LIMIT ?1 OFFSET ?2"
        ))?;
        let rows = stmt.query_map(params![limit as i64, offset as i64], file_row)?;
        Ok((total as u64, rows.collect::<rusqlite::Result<_>>()?))
    }

    /// Ids of a folder's files.
    pub fn folder_file_ids(&self, folder: &str) -> Result<Vec<i64>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT id FROM files WHERE folder = ?1")?;
        let rows = stmt.query_map([folder], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// SQLite's `data_version`: changes when another connection commits to the database.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self
            .conn()
            .pragma_query_value(None, "data_version", |r| r.get(0))?)
    }

    pub fn delete_file(&self, file_id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM files WHERE id = ?1", [file_id])?;
        Ok(())
    }

    /// Removes every file of a folder from the index. Returns how many.
    pub fn delete_folder(&self, folder: &str) -> Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM files WHERE folder = ?1", [folder])?)
    }

    pub fn file(&self, id: i64) -> Result<Option<FileRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {FILE_COLS} FROM files WHERE id = ?1"),
                [id],
                file_row,
            )
            .optional()?)
    }

    pub fn file_by_path(&self, path: &str) -> Result<Option<FileRow>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {FILE_COLS} FROM files WHERE path = ?1"),
                [path],
                file_row,
            )
            .optional()?)
    }

    pub fn files(&self, ids: &[i64]) -> Result<Vec<FileRow>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare_cached(&format!("SELECT {FILE_COLS} FROM files WHERE id = ?1"))?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(f) = stmt.query_row([id], file_row).optional()? {
                out.push(f);
            }
        }
        Ok(out)
    }

    /// Ids of files matching every given condition (empty lists match all); with a `path_ok`
    /// test, only files whose path passes it.
    pub fn filtered_files(
        &self,
        kinds: &[Kind],
        folders: &[String],
        date_from: Option<i64>,
        date_to: Option<i64>,
        path_ok: Option<&dyn Fn(&str) -> bool>,
    ) -> Result<Vec<i64>> {
        // Without a path test the kind/folder indexes answer alone.
        let mut sql = String::from(if path_ok.is_some() {
            "SELECT id, path FROM files WHERE 1"
        } else {
            "SELECT id FROM files WHERE 1"
        });
        let mut args: Vec<rusqlite::types::Value> = vec![];
        let mut list = |sql: &mut String, col: &str, values: Vec<String>| {
            if !values.is_empty() {
                sql.push_str(&format!(
                    " AND {col} IN ({})",
                    vec!["?"; values.len()].join(",")
                ));
                args.extend(values.into_iter().map(rusqlite::types::Value::Text));
            }
        };
        list(
            &mut sql,
            "kind",
            kinds.iter().map(|k| k.as_str().to_string()).collect(),
        );
        list(&mut sql, "folder", folders.to_vec());
        if let Some(d) = date_from {
            sql.push_str(" AND coalesce(date, mtime) >= ?");
            args.push(d.into());
        }
        if let Some(d) = date_to {
            sql.push_str(" AND coalesce(date, mtime) < ?");
            args.push(d.into());
        }
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(args))?;
        let mut out = vec![];
        while let Some(r) = rows.next()? {
            if let Some(ok) = path_ok
                && !ok(r.get_ref(1)?.as_str()?)
            {
                continue;
            }
            out.push(r.get(0)?);
        }
        Ok(out)
    }

    /// Adds a chunk to a file. Returns its id.
    pub fn insert_chunk(&self, c: &ChunkRow) -> Result<i64> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO chunks (file_id, kind, page, t_start, t_end, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![c.file_id, c.kind.as_str(), c.page, c.t_start, c.t_end, c.text],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Removes a file's chunks (and so its vectors), before re-indexing it.
    pub fn delete_chunks(&self, file_id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM chunks WHERE file_id = ?1", [file_id])?;
        Ok(())
    }

    pub fn chunks(&self, file_id: i64) -> Result<Vec<ChunkRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, file_id, kind, page, t_start, t_end, text FROM chunks WHERE file_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([file_id], chunk_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn chunk(&self, id: i64) -> Result<Option<ChunkRow>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, file_id, kind, page, t_start, t_end, text FROM chunks WHERE id = ?1",
                [id],
                chunk_row,
            )
            .optional()?)
    }

    /// Stores a chunk's vector as little-endian f32s.
    pub fn put_vector(&self, chunk_id: i64, v: &[f32]) -> Result<()> {
        let blob: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.conn().execute(
            "INSERT OR REPLACE INTO vectors (chunk_id, dim, blob) VALUES (?1, ?2, ?3)",
            params![chunk_id, v.len() as i64, blob],
        )?;
        Ok(())
    }

    /// Calls `f(chunk_id, file_id, kind, vector)` for every stored vector.
    pub fn for_each_vector(&self, mut f: impl FnMut(i64, i64, SegmentKind, &[f32])) -> Result<()> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT v.chunk_id, c.file_id, v.blob, c.kind FROM vectors v JOIN chunks c ON c.id = v.chunk_id",
        )?;
        let mut rows = stmt.query([])?;
        let mut buf = Vec::new();
        while let Some(r) = rows.next()? {
            let blob = r.get_ref(2)?.as_blob()?;
            buf.clear();
            buf.extend(
                blob.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b)),
            );
            let kind = SegmentKind::parse(r.get_ref(3)?.as_str()?).unwrap_or_default();
            f(r.get(0)?, r.get(1)?, kind, &buf);
        }
        Ok(())
    }

    /// A cheap fingerprint that changes whenever vectors are added or removed.
    pub fn vectors_fingerprint(&self) -> Result<(i64, i64)> {
        Ok(self.conn().query_row(
            "SELECT count(*), coalesce(max(chunk_id), 0) FROM vectors",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    pub fn vectors_for_file(&self, file_id: i64) -> Result<Vec<(i64, Vec<f32>)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT v.chunk_id, v.blob FROM vectors v JOIN chunks c ON c.id = v.chunk_id WHERE c.file_id = ?1",
        )?;
        let rows = stmt.query_map([file_id], |r| {
            let blob: Vec<u8> = r.get(1)?;
            Ok((
                r.get(0)?,
                blob.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect(),
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Keyword search. Returns `(chunk_id, file_id)` best first.
    pub fn keyword_search(&self, query: &str, limit: usize) -> Result<Vec<(i64, i64)>> {
        let Some(q) = fts_query(query) else {
            return Ok(vec![]);
        };
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT fts.rowid, c.file_id FROM fts JOIN chunks c ON c.id = fts.rowid
             WHERE fts MATCH ?1 ORDER BY bm25(fts, 4.0, 2.0, 1.0, 1.0) LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![q, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Marks files for re-indexing. Returns how many.
    pub fn mark_pending(&self, scope: Scope) -> Result<usize> {
        let conn = self.conn();
        Ok(match scope {
            Scope::All => conn.execute("UPDATE files SET status = 'pending', error = NULL", [])?,
            Scope::Folder(f) => conn.execute(
                "UPDATE files SET status = 'pending', error = NULL WHERE folder = ?1",
                [f],
            )?,
            Scope::File(id) => conn.execute(
                "UPDATE files SET status = 'pending', error = NULL WHERE id = ?1",
                [id],
            )?,
            Scope::Problems => conn.execute(
                "UPDATE files SET status = 'pending', error = NULL
                 WHERE status IN ('error', 'skipped')",
                [],
            )?,
        })
    }

    /// File counts, for all files or one folder.
    pub fn counts(&self, folder: Option<&str>) -> Result<Counts> {
        let conn = self.conn();
        let (filter, arg) = match folder {
            Some(f) => ("WHERE folder = ?1", f),
            None => ("WHERE ?1 = ?1", ""),
        };
        let mut c = conn.query_row(
            &format!(
                "SELECT count(*),
                    coalesce(sum(status = 'indexed'), 0),
                    coalesce(sum(status = 'pending'), 0),
                    coalesce(sum(status = 'error'), 0),
                    coalesce(sum(size), 0),
                    coalesce(sum(status = 'skipped'), 0)
                 FROM files {filter}"
            ),
            [arg],
            |r| {
                Ok(Counts {
                    files: r.get::<_, i64>(0)? as u64,
                    indexed: r.get::<_, i64>(1)? as u64,
                    pending: r.get::<_, i64>(2)? as u64,
                    errors: r.get::<_, i64>(3)? as u64,
                    bytes: r.get::<_, i64>(4)? as u64,
                    skipped: r.get::<_, i64>(5)? as u64,
                    ..Default::default()
                })
            },
        )?;
        let (chunks, vectors): (i64, i64) = conn.query_row(
            &format!(
                "SELECT count(c.id), count(v.chunk_id) FROM chunks c
                 JOIN files f ON f.id = c.file_id
                 LEFT JOIN vectors v ON v.chunk_id = c.id {}",
                filter.replace("folder", "f.folder")
            ),
            [arg],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        c.chunks = chunks as u64;
        c.vectors = vectors as u64;
        Ok(c)
    }

    /// File counts per kind.
    pub fn kind_counts(&self) -> Result<Vec<(Kind, u64)>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare_cached("SELECT kind, count(*) FROM files GROUP BY kind ORDER BY kind")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                Kind::parse(&r.get::<_, String>(0)?).unwrap_or_default(),
                r.get::<_, i64>(1)? as u64,
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

const FILE_COLS: &str =
    "id, path, name, folder, size, mtime, date, hash, kind, mime, exif, status, error, indexed_at";

fn file_row(r: &rusqlite::Row) -> rusqlite::Result<FileRow> {
    Ok(FileRow {
        id: r.get(0)?,
        path: r.get(1)?,
        name: r.get(2)?,
        folder: r.get(3)?,
        size: r.get(4)?,
        mtime: r.get(5)?,
        date: r.get(6)?,
        hash: r.get(7)?,
        kind: Kind::parse(&r.get::<_, String>(8)?).unwrap_or_default(),
        mime: r.get(9)?,
        exif: r.get(10)?,
        status: r.get(11)?,
        error: r.get(12)?,
        indexed_at: r.get(13)?,
    })
}

fn scan_row(r: &rusqlite::Row) -> rusqlite::Result<ScanRow> {
    Ok(ScanRow {
        id: r.get(0)?,
        path: r.get(1)?,
        folder: r.get(2)?,
        size: r.get(3)?,
        mtime: r.get(4)?,
        hash: r.get(5)?,
        kind: Kind::parse(&r.get::<_, String>(6)?).unwrap_or_default(),
        status: r.get(7)?,
    })
}

fn chunk_row(r: &rusqlite::Row) -> rusqlite::Result<ChunkRow> {
    Ok(ChunkRow {
        id: r.get(0)?,
        file_id: r.get(1)?,
        kind: SegmentKind::parse(&r.get::<_, String>(2)?).unwrap_or_default(),
        page: r.get(3)?,
        t_start: r.get(4)?,
        t_end: r.get(5)?,
        text: r.get(6)?,
    })
}

/// Turns free text into an FTS5 query: every word must appear (as a prefix for the last one).
fn fts_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\""))
        .collect();
    let (last, rest) = words.split_last()?;
    let mut q = rest.join(" ");
    if !q.is_empty() {
        q.push(' ');
    }
    q.push_str(last);
    q.push('*');
    Some(q)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, kind: Kind) -> NewFile {
        NewFile {
            path: path.into(),
            folder: "/data".into(),
            size: 10,
            mtime: 1_700_000_000,
            kind,
            ..Default::default()
        }
    }

    #[test]
    fn migrates_fresh_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/rummage.db");
        {
            let s = Store::open(&path).unwrap();
            assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
            let conn = s.conn();
            let tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type IN ('table') ORDER BY name")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            for t in ["chunks", "files", "fts", "settings", "vectors"] {
                assert!(tables.iter().any(|x| x == t), "missing {t} in {tables:?}");
            }
            let mode: String = conn
                .pragma_query_value(None, "journal_mode", |r| r.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
        s.set_setting("k", "v").unwrap();
        s.set_setting("k", "w").unwrap();
        assert_eq!(s.setting("k").unwrap().as_deref(), Some("w"));
    }

    #[test]
    fn refuses_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.db");
        Store::open(&path).unwrap();
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(Store::open(&path).is_err());
    }

    #[test]
    fn keyword_index_follows_files_and_chunks() {
        let s = Store::open_in_memory().unwrap();
        let id = s
            .upsert_file(&file("/data/notes/holiday.md", Kind::Text))
            .unwrap();
        let chunk = s
            .insert_chunk(&ChunkRow {
                file_id: id,
                kind: SegmentKind::Text,
                text: Some("Packing list for the mountains".into()),
                ..Default::default()
            })
            .unwrap();
        s.put_vector(chunk, &[0.5, 0.5, 0.5, 0.5]).unwrap();

        assert_eq!(s.keyword_search("mountain", 10).unwrap(), vec![(chunk, id)]);
        assert_eq!(s.keyword_search("holiday", 10).unwrap(), vec![(chunk, id)]);

        s.rename_file(id, "/data/notes/trip.md").unwrap();
        assert!(s.keyword_search("holiday", 10).unwrap().is_empty());
        assert_eq!(s.keyword_search("trip", 10).unwrap(), vec![(chunk, id)]);
        assert_eq!(s.vectors_for_file(id).unwrap()[0].1, vec![0.5; 4]);

        let c = s.counts(None).unwrap();
        assert_eq!((c.files, c.pending, c.chunks, c.vectors), (1, 1, 1, 1));
        s.set_status(id, "indexed", None).unwrap();
        assert_eq!(s.counts(Some("/data")).unwrap().indexed, 1);
        assert_eq!(s.counts(Some("/other")).unwrap().files, 0);

        s.delete_file(id).unwrap();
        assert!(s.keyword_search("trip", 10).unwrap().is_empty());
        assert_eq!(s.vectors_fingerprint().unwrap().0, 0);
        assert!(s.chunk(chunk).unwrap().is_none());
    }

    #[test]
    fn upsert_resets_status_and_keeps_id() {
        let s = Store::open_in_memory().unwrap();
        let a = s.upsert_file(&file("/data/a.jpg", Kind::Image)).unwrap();
        s.set_status(a, "error", Some("boom")).unwrap();
        let b = s.upsert_file(&file("/data/a.jpg", Kind::Image)).unwrap();
        assert_eq!(a, b);
        let row = s.file(a).unwrap().unwrap();
        assert_eq!((row.status.as_str(), row.error), ("pending", None));
        assert_eq!(row.name, "a.jpg");
        assert_eq!(s.mark_pending(Scope::Folder("/data")).unwrap(), 1);
        assert_eq!(s.delete_folder("/data").unwrap(), 1);
    }

    #[test]
    fn replace_content_moves_and_scan_rows() {
        let s = Store::open_in_memory().unwrap();
        let id = s
            .upsert_file(&file("/data/a/photo.jpg", Kind::Image))
            .unwrap();
        let other = s.upsert_file(&file("/data/ab.jpg", Kind::Image)).unwrap();
        let content = FileContent {
            size: 11,
            mtime: 5,
            hash: Some("h1".into()),
            exif: Some(r#"{"make":"Canon"}"#.into()),
            status: "indexed".into(),
            ..Default::default()
        };
        let chunk = |t: &str| ChunkRow {
            kind: SegmentKind::Text,
            text: Some(t.into()),
            ..Default::default()
        };
        let ids = s
            .replace_content(id, &content, &[(chunk("first"), vec![1.0, 0.0])])
            .unwrap()
            .unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(s.keyword_search("canon", 10).unwrap(), vec![(ids[0], id)]);
        let ids2 = s
            .replace_content(
                id,
                &content,
                &[
                    (chunk("second"), vec![0.0, 1.0]),
                    (chunk("third"), vec![0.6, 0.8]),
                ],
            )
            .unwrap()
            .unwrap();
        assert_eq!(s.chunks(id).unwrap().len(), 2);
        assert!(s.keyword_search("first", 10).unwrap().is_empty());
        assert_eq!(s.vectors_for_file(id).unwrap().len(), 2);
        assert_eq!(s.counts(None).unwrap().indexed, 1);
        assert!(s.replace_content(999, &content, &[]).unwrap().is_none());

        let under: Vec<i64> = s
            .scan_rows_under("/data/a")
            .unwrap()
            .iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(under, vec![id]);
        assert_eq!(s.scan_rows().unwrap().len(), 2);
        assert_eq!(s.pending_files().unwrap(), vec![other]);

        s.move_file(id, "/other/b.jpg", "/other").unwrap();
        let row = s.file(id).unwrap().unwrap();
        assert_eq!(
            (row.name.as_str(), row.folder.as_str()),
            ("b.jpg", "/other")
        );
        assert_eq!(
            s.vectors_for_file(id).unwrap()[0].0,
            ids2[0],
            "a move keeps the vectors"
        );
        s.set_status(other, "error", Some("bad")).unwrap();
        let (total, rows) = s.problem_files(10, 0).unwrap();
        assert_eq!((total, rows[0].id), (1, other));
    }

    #[test]
    fn fts_query_escapes_input() {
        assert_eq!(fts_query("hello wor").unwrap(), "\"hello\" \"wor\"*");
        assert_eq!(fts_query("\"); DROP").unwrap(), "\"DROP\"*");
        assert!(fts_query("  -- ").is_none());
    }
}
