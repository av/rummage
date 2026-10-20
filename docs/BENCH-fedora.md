# rummage on fedora: real files, ROCm vs Vulkan vs CPU

Measured 2026-10-19 on fedora: AMD Ryzen AI Max+ 395 (16 cores, 32 threads), Radeon 8060S
(gfx1151, 128 GB unified memory), Fedora 43, kernel 7.0.12, Mesa RADV 25.3.6. All three backends
are the same commit of main (`c3709a7`, the CI packages): Vulkan and CPU run on the host; ROCm
runs inside `ghcr.io/av/rummage:0.1.0-rocm` (ROCm 7.2), because the ROCm build needs ROCm 7
libraries and the host has Fedora's ROCm 6.4. Model `embeddinggemma-2` Q8_0, 768 dims, default
settings (`index.concurrency` 8, 280 tokens per image). Video and HEIC go through a static
ffmpeg 8.1 (BtbN build) set with `tools.ffmpeg`. Every run started with a 1-minute load below 22
(70% of the threads) and nothing else on the GPU.

**Verdict: Vulkan is the fastest backend on every kind of file**: 1.5 to 2.2 times ROCm and 6 to
8 times the CPU. It is what `rummage.service` runs. Search costs about 9 ms per query on any
backend.

## Indexing throughput on a fixed sample of real files

A seeded random sample (`random.Random(6)`) of a real personal library (photos, phone videos,
music stems, notes and documents), copied into its own folder: 500 images (mostly phone photos,
plus renders and stills), 50 audio files (1,534 s of music stems and tracks), 10 videos (282 s:
phone clips, a screencast, renders), 200 markdown/text files and 10 PDFs. Each run indexes one kind from an empty index with `rummage index --once` and counts
everything: decoding, ffmpeg, poppler, thumbnails, embedding and SQLite. GPU busy is the mean of
`rocm-smi --showuse`, sampled every second.

| kind (files → chunks) | | Vulkan | ROCm | CPU |
|---|---|---:|---:|---:|
| text, 200 md/txt → 382 passages | wall time | **20.2 s** | 31.0 s | 162.0 s |
| | files/s · chunks/s | **9.90 · 18.9** | 6.45 · 12.3 | 1.23 · 2.36 |
| | GPU busy | 70% | 66% | - |
| PDF, 10 → 157 page images + 174 passages | wall time | **52.5 s** | 87.3 s | 348.7 s |
| | files/s · chunks/s | **0.19 · 6.30** | 0.11 · 3.79 | 0.029 · 0.95 |
| | GPU busy | 82% | 85% | - |
| image, 500 → 500 | wall time | **141.5 s** | 230.6 s | 895.9 s |
| | files/s | **3.53** | 2.17 | 0.56 |
| | GPU busy | 81% | 85% | - |
| audio, 50 (1,534 s) → 89 windows | wall time | **9.0 s** | 20.0 s | 55.4 s |
| | files/s · chunks/s | **5.54 · 9.86** | 2.49 · 4.44 | 0.90 · 1.61 |
| | audio seconds/s | **170** | 77 | 28 |
| | GPU busy | 63% | 32% | - |
| video, 10 (282 s) → 31 frame windows + 12 audio windows | wall time | **28.3 s** | 42.6 s | 185.9 s |
| | files/s · chunks/s | **0.35 · 1.52** | 0.23 · 1.01 | 0.054 · 0.23 |
| | video seconds/s | **10.0** | 6.6 | 1.5 |
| | GPU busy | 79% | 76% | - |
| **all five** | wall time | **251 s** | 412 s | 1,648 s |

- The GPU stays 60 to 85% busy, so indexing is bound by the embedder, not by decoding or ffmpeg.
  Audio on ROCm is the exception (32% busy, under half Vulkan's speed); the sample's audio
  windows are short (31 s per file on average), so fixed per-call costs weigh more, but this
  wasn't profiled.
- ROCm 7.2 from AMD's image is no faster here than Fedora's ROCm 6.4 was in
  `crates/rummage-embed/BENCH.md` (2.17 vs 2.58 images/s there, which counted embedding only).
  So the newer ROCm doesn't close the gap to Vulkan on gfx1151.
- The CPU build is usable for a few hundred files but takes 6 to 8 times longer.

## Search latency at full index size

The full index (4,537 files, 10,101 vectors, below), served by each backend in turn from a
snapshot of the live database: 50 typed queries (photos, video, audio, notes, PDFs), one
warm-up pass, then 3 timed passes, measured by an HTTP client on the same host
(`POST /api/v1/search`, hybrid, 24 results). It includes embedding the query.

| | Vulkan | ROCm | CPU |
|---|---:|---:|---:|
| p50 | **8.9 ms** | 9.0 ms | 11.4 ms |
| p95 | **9.8 ms** | 12.8 ms | 14.7 ms |
| max | 10.6 ms | 1,044 ms | 16.5 ms |

ROCm's maximum is a single slow query (the other 149 stayed under 13 ms), probably a kernel
loaded on first use of a new input size; the cause wasn't traced.

## The full index of a real personal library

`rummage.service` (Vulkan, `Nice=10`, lowest best-effort I/O priority) built it from scratch in **57.5 min**
(07:16:46 to 08:14:18; the GPU was about 87% busy throughout).

| kind | files | size | chunks |
|---|---:|---:|---:|
| image | 2,333 | 6.2 GB | 2,333 |
| video | 796 (795 indexed) | 50.4 GB, 7.6 h | 3,045 frame windows + 1,284 audio windows |
| audio | 282 | 1.1 GB | 470 windows |
| text | 1,114 | 0.04 GB | 2,560 passages |
| doc (PDF) | 12 | 0.03 GB | 192 page images + 215 passages (after #9) |
| **total** | **4,537** | **53.7 GB** | **10,099** |

One error: a damaged video (ffmpeg: "Invalid NAL unit size" on every packet; no frame decodes).
No files were skipped. All 33 HEVC phone videos are indexed with frames and sound, through the
static ffmpeg; Fedora's `ffmpeg-free` can't decode them.

## Search quality

Ten realistic queries on the owner's own library (photos, phone videos, music stems, notes and
PDFs), judged by eye on the top 3 results. Photos, video frames, notes and PDF pages were found
well. The two weak spots found then, audio barely reaching mixed results and generic video sound
tracks outranking matching frames, were fixed in #11 (see docs/REVIEW-2026-10.md). The queries
and results are kept private because they describe personal files.
