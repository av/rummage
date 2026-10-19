//! All vectors in memory, scanned exactly on every search.
//!
//! Loaded from the store once, then kept up to date as the indexer adds and removes files, so
//! a search never waits for a reload. Vectors are kept at one length (the embedder's); longer
//! stored vectors are cut and renormalised (Matryoshka), shorter ones are left out until the
//! file is indexed again.
//!
//! Similarities are not comparable across kinds of segment: with EmbeddingGemma 2, audio
//! windows (a phone video's background noise, say) score higher against any text query than
//! photos do, so they would crowd out the photos that actually match. Ranking therefore uses
//! each similarity's z-score within its segment kind for this query (how unusual the match is
//! for an image, a passage, an audio window...), while hits report the plain cosine.

use std::collections::HashMap;

use crate::embed::{dot_fn, normalize};
use crate::types::SegmentKind;

/// Segment kinds, as stored per row.
const KINDS: usize = 5;
/// A kind with fewer vectors than this is ranked with the statistics of all vectors.
const MIN_KIND_ROWS: usize = 16;

fn kind_code(k: SegmentKind) -> u8 {
    match k {
        SegmentKind::Text => 0,
        SegmentKind::Image => 1,
        SegmentKind::Page => 2,
        SegmentKind::Audio => 3,
        SegmentKind::Video => 4,
    }
}

/// A row whose file was removed; reclaimed by the next compaction.
const DEAD: u32 = u32::MAX;

#[derive(Default)]
pub struct VectorIndex {
    dim: usize,
    /// `rows * dim` floats.
    data: Vec<f32>,
    chunk_ids: Vec<i64>,
    /// Segment kind of each row ([`kind_code`]).
    kinds: Vec<u8>,
    /// File slot of each row, or [`DEAD`].
    slots: Vec<u32>,
    /// File id of each slot.
    slot_files: Vec<i64>,
    slot_of: HashMap<i64, u32>,
    rows_of: HashMap<u32, Vec<u32>>,
    dead: usize,
}

/// The best-matching chunk of one file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FileHit {
    pub file_id: i64,
    pub chunk_id: i64,
    /// Cosine similarity.
    pub score: f32,
    /// What hits are ranked by: the similarity's z-score within its segment kind.
    pub rank: f32,
}

impl VectorIndex {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            ..Default::default()
        }
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Live vectors.
    pub fn len(&self) -> usize {
        self.chunk_ids.len() - self.dead
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Adds one vector. Returns false (and stores nothing) when it is shorter than the index's.
    pub fn push(&mut self, file_id: i64, chunk_id: i64, kind: SegmentKind, v: &[f32]) -> bool {
        self.push_code(file_id, chunk_id, kind_code(kind), v)
    }

    fn push_code(&mut self, file_id: i64, chunk_id: i64, kind: u8, v: &[f32]) -> bool {
        if v.len() < self.dim || self.dim == 0 {
            return false;
        }
        let slot = match self.slot_of.get(&file_id) {
            Some(s) => *s,
            None => {
                let s = self.slot_files.len() as u32;
                self.slot_files.push(file_id);
                self.slot_of.insert(file_id, s);
                s
            }
        };
        let row = self.chunk_ids.len() as u32;
        let start = self.data.len();
        self.data.extend_from_slice(&v[..self.dim]);
        if v.len() > self.dim {
            normalize(&mut self.data[start..]);
        }
        self.chunk_ids.push(chunk_id);
        self.kinds.push(kind);
        self.slots.push(slot);
        self.rows_of.entry(slot).or_default().push(row);
        true
    }

    /// Drops every vector of a file.
    pub fn remove_file(&mut self, file_id: i64) {
        let Some(slot) = self.slot_of.get(&file_id).copied() else {
            return;
        };
        if let Some(rows) = self.rows_of.remove(&slot) {
            for r in &rows {
                self.slots[*r as usize] = DEAD;
            }
            self.dead += rows.len();
        }
        if self.dead > 4096 && self.dead * 4 > self.chunk_ids.len() {
            self.compact();
        }
    }

    /// Replaces a file's vectors: `(chunk id, kind, vector)`.
    pub fn set_file(&mut self, file_id: i64, rows: &[(i64, SegmentKind, Vec<f32>)]) {
        self.remove_file(file_id);
        for (chunk, kind, v) in rows {
            self.push(file_id, *chunk, *kind, v);
        }
    }

    fn compact(&mut self) {
        let dim = self.dim;
        let mut out = VectorIndex::new(dim);
        out.data.reserve(self.len() * dim);
        for (row, slot) in self.slots.iter().enumerate() {
            if *slot != DEAD {
                let file = self.slot_files[*slot as usize];
                out.push_code(
                    file,
                    self.chunk_ids[row],
                    self.kinds[row],
                    &self.data[row * dim..(row + 1) * dim],
                );
            }
        }
        *self = out;
    }

    /// The best chunk of every file against `query` (cut to the index's length), keeping files
    /// that pass `keep`, best first (by [`FileHit::rank`]), at most `limit`.
    pub fn search(&self, query: &[f32], limit: usize, keep: impl Fn(i64) -> bool) -> Vec<FileHit> {
        let dim = self.dim;
        if dim == 0 || query.len() < dim || self.chunk_ids.is_empty() || limit == 0 {
            return vec![];
        }
        let mut q = query[..dim].to_vec();
        normalize(&mut q);
        let rows = self.chunk_ids.len();
        let nslots = self.slot_files.len();
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(16)
            .min(rows.div_ceil(4096))
            .max(1);
        let per = rows.div_ceil(threads);
        let dot = dot_fn();

        // Pass 1: every similarity, and per-kind sums for the mean and spread.
        let mut scores = vec![0f32; rows];
        let score_part = |from: usize, out: &mut [f32]| {
            let mut stats = [(0usize, 0f64, 0f64); KINDS];
            for (i, s) in out.iter_mut().enumerate() {
                let row = from + i;
                if self.slots[row] == DEAD {
                    continue;
                }
                *s = dot(&self.data[row * dim..(row + 1) * dim], &q);
                let st = &mut stats[self.kinds[row] as usize];
                st.0 += 1;
                st.1 += *s as f64;
                st.2 += (*s as f64) * (*s as f64);
            }
            stats
        };
        let parts: Vec<[(usize, f64, f64); KINDS]> = if threads == 1 {
            vec![score_part(0, &mut scores)]
        } else {
            std::thread::scope(|sc| {
                let handles: Vec<_> = scores
                    .chunks_mut(per)
                    .enumerate()
                    .map(|(t, out)| {
                        let f = &score_part;
                        sc.spawn(move || f(t * per, out))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().expect("scan thread"))
                    .collect()
            })
        };
        let mut stats = [(0usize, 0f64, 0f64); KINDS];
        for part in &parts {
            for (s, p) in stats.iter_mut().zip(part) {
                s.0 += p.0;
                s.1 += p.1;
                s.2 += p.2;
            }
        }
        let all = stats.iter().fold((0usize, 0f64, 0f64), |a, s| {
            (a.0 + s.0, a.1 + s.1, a.2 + s.2)
        });
        let mean_sd = |(n, sum, sq): (usize, f64, f64)| {
            let mean = sum / n.max(1) as f64;
            let var = (sq / n.max(1) as f64 - mean * mean).max(0.0);
            (mean as f32, (var.sqrt() as f32).max(1e-3))
        };
        let calib: Vec<(f32, f32)> = stats
            .iter()
            .map(|s| mean_sd(if s.0 >= MIN_KIND_ROWS { *s } else { all }))
            .collect();

        // Pass 2: the best row of each file by calibrated score.
        let mut best = vec![(f32::NEG_INFINITY, u32::MAX); nslots];
        for (row, s) in scores.iter().enumerate() {
            let slot = self.slots[row];
            if slot == DEAD {
                continue;
            }
            let (mean, sd) = calib[self.kinds[row] as usize];
            let z = (s - mean) / sd;
            let b = &mut best[slot as usize];
            if z > b.0 {
                *b = (z, row as u32);
            }
        }
        let mut hits: Vec<FileHit> = best
            .into_iter()
            .enumerate()
            .filter(|(_, (_, row))| *row != u32::MAX)
            .map(|(slot, (rank, row))| FileHit {
                file_id: self.slot_files[slot],
                chunk_id: self.chunk_ids[row as usize],
                score: scores[row as usize],
                rank,
            })
            .filter(|h| keep(h.file_id))
            .collect();
        let by_rank = |a: &FileHit, b: &FileHit| b.rank.total_cmp(&a.rank);
        if hits.len() > limit {
            hits.select_nth_unstable_by(limit - 1, by_rank);
            hits.truncate(limit);
        }
        hits.sort_by(by_rank);
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: SegmentKind = SegmentKind::Text;

    fn unit(v: &[f32]) -> Vec<f32> {
        let mut v = v.to_vec();
        normalize(&mut v);
        v
    }

    #[test]
    fn best_chunk_per_file_and_removal() {
        let mut ix = VectorIndex::new(2);
        ix.push(1, 10, T, &unit(&[1.0, 0.0]));
        ix.push(1, 11, T, &unit(&[0.0, 1.0]));
        ix.push(2, 20, T, &unit(&[0.7, 0.7]));
        assert!(!ix.push(3, 30, T, &[1.0]), "too short");
        let hits = ix.search(&[0.0, 1.0], 10, |_| true);
        assert_eq!(
            hits.iter()
                .map(|h| (h.file_id, h.chunk_id))
                .collect::<Vec<_>>(),
            vec![(1, 11), (2, 20)]
        );
        assert_eq!(ix.search(&[0.0, 1.0], 10, |f| f != 1)[0].file_id, 2);
        ix.remove_file(1);
        assert_eq!(ix.len(), 1);
        assert_eq!(ix.search(&[0.0, 1.0], 10, |_| true).len(), 1);
        ix.set_file(1, &[(12, T, unit(&[1.0, 0.2]))]);
        assert_eq!(ix.search(&[1.0, 0.0], 1, |_| true)[0].chunk_id, 12);
        ix.compact();
        assert_eq!(ix.len(), 2);
        assert_eq!(ix.search(&[1.0, 0.0], 1, |_| true)[0].chunk_id, 12);
    }

    #[test]
    fn longer_vectors_are_cut_and_renormalised() {
        let mut ix = VectorIndex::new(2);
        ix.push(1, 1, T, &unit(&[3.0, 4.0, 12.0]));
        let hit = ix.search(&[0.6, 0.8, 0.0], 1, |_| true)[0];
        assert!((hit.score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn kinds_are_ranked_on_their_own_scale() {
        // Audio scores high against everything; the one image that really matches still wins.
        let mut ix = VectorIndex::new(3);
        let q = unit(&[1.0, 0.0, 0.0]);
        for i in 0..20 {
            let f = i as f32 / 20.0;
            ix.push(
                100 + i,
                100 + i,
                SegmentKind::Audio,
                &unit(&[0.8 + f * 0.05, 0.6, 0.0]),
            );
            ix.push(
                200 + i,
                200 + i,
                SegmentKind::Image,
                &unit(&[0.1 + f * 0.1, 0.0, 1.0]),
            );
        }
        ix.push(1, 1, SegmentKind::Image, &unit(&[0.7, 0.0, 0.7]));
        let hits = ix.search(&q, 3, |_| true);
        assert_eq!(hits[0].file_id, 1, "{hits:?}");
        assert!(
            hits[1].score > hits[0].score,
            "hits report the plain cosine"
        );
    }

    #[test]
    fn parallel_scan_agrees_with_one_thread() {
        let mut ix = VectorIndex::new(16);
        let mut state = 1u64;
        let mut rnd = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (state >> 33) as f32 / (1u64 << 31) as f32 - 0.5
        };
        for i in 0..20_000 {
            let v: Vec<f32> = (0..16).map(|_| rnd()).collect();
            ix.push(i / 3, i, T, &unit(&v));
        }
        let q: Vec<f32> = (0..16).map(|_| rnd()).collect();
        let hits = ix.search(&q, 5, |_| true);
        let mut brute: Vec<(f32, i64)> = (0..ix.chunk_ids.len())
            .map(|r| {
                (
                    crate::embed::dot(&ix.data[r * 16..(r + 1) * 16], &unit(&q)),
                    ix.chunk_ids[r],
                )
            })
            .collect();
        brute.sort_by(|a, b| b.0.total_cmp(&a.0));
        assert_eq!(hits[0].chunk_id, brute[0].1);
        assert_eq!(hits.len(), 5);
    }
}
