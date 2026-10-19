# rummage-embed: parity and speed on fedora

Measured 2026-10-19 on fedora: AMD Ryzen AI Max+ 395 (16 cores, 32 threads), Radeon 8060S
integrated GPU (RDNA 3.5, gfx1151, 128 GB unified memory), Fedora 43, kernel 7.0.12, Mesa RADV
25.3.6, ROCm 6.4 (Fedora packages). llama.cpp b11721 (`ff5a2337`). Model files from
`ggml-org/embeddinggemma-2-GGUF` at `bfcd2987`. Every run had a 1-minute load average below 6
when it started (other users' processes; nothing else on the GPU).

## Parity with the official model

Cosine similarity between this engine and `google/embeddinggemma-2` run by sentence-transformers
6.1.0 / transformers 5.20.1 in float32 on CPU (`tests/reference`), per item and backend.
Pass marks: 0.98 for Q8_0, 0.995 for BF16. Every run also passed the token-count check (same
token count as the official processor for every text, image and audio item), the batched vs
one-by-one check, and the retrieval check (each query is nearest its matching document, image
and audio clip).

| item | cpu Q8_0 | cpu BF16 | vulkan Q8_0 | vulkan BF16 | rocm Q8_0 | rocm BF16 |
|---|---:|---:|---:|---:|---:|---:|
| q_bread (query) | 0.99990 | 1.00000 | 0.99989 | 1.00000 | 0.99989 | 0.99999 |
| q_cat (query) | 0.99992 | 1.00000 | 0.99993 | 1.00000 | 0.99993 | 1.00000 |
| d_bread (document) | 0.99970 | 0.99999 | 0.99968 | 0.99999 | 0.99970 | 0.99998 |
| d_cat (document) | 0.99986 | 1.00000 | 0.99987 | 1.00000 | 0.99986 | 1.00000 |
| d_tides (document) | 0.99984 | 1.00000 | 0.99978 | 1.00000 | 0.99980 | 0.99999 |
| img_bread (image) | 0.99970 | 0.99979 | 0.99966 | 0.99980 | 0.99964 | 0.99978 |
| img_cat (image) | 0.99961 | 0.99966 | 0.99962 | 0.99965 | 0.99960 | 0.99964 |
| img_lighthouse (image) | 0.99932 | 0.99947 | 0.99919 | 0.99952 | 0.99940 | 0.99955 |
| aud_bread (speech, 7.3 s) | 0.99961 | 0.99962 | 0.99957 | 0.99963 | 0.99966 | 0.99958 |
| aud_lighthouse (speech, 5.3 s) | 0.99867 | 0.99863 | 0.99745 | 0.99799 | 0.99495 | 0.99860 |
| vid_cat (video, 4 frames) | 0.99955 | 0.99967 | 0.99958 | 0.99968 | 0.99957 | 0.99971 |
| **lowest** | **0.99867** | **0.99863** | **0.99745** | **0.99799** | **0.99495** | **0.99860** |
| lowest at 256 dims | 0.99874 | 0.99877 | 0.99768 | 0.99821 | 0.99543 | 0.99874 |
| lowest batched vs one-by-one | 0.99963 | 0.99999 | 0.99983 | 0.99999 | 0.99979 | 0.99997 |

Text matches almost exactly. Images, audio and video sit at 0.999 even in BF16, which points at
input preparation rather than arithmetic: JPEG decoding and bicubic resizing in Rust vs PIL and
torchvision, and llama.cpp's own mel spectrogram for audio. Audio on the GPU backends is the
loosest item (0.995 on ROCm Q8_0) but far above the bar.

CUDA could not be run here (no NVIDIA GPU); see "CUDA" below.

## Speed

`rummage-bench`, Q8_0 unless noted, default settings (`batch_tokens` 512, flash attention off,
float16 off). Inputs are synthetic but sized like real work: 512-token text chunks, 1600x1200
images at 280 tokens, 30 s audio windows (754 tokens), 1280x720 video frames at 140 tokens in
10-frame (10 s) windows, and short single queries. Each workload is warmed up once before timing.
Counts: GPU runs 256 texts, 48 images, 12 audio windows, 6 videos, 200 queries; the CPU run
64 / 12 / 8 / 2 / 100.

| | CPU Q8_0 | Vulkan Q8_0 | ROCm Q8_0 | Vulkan BF16 | ROCm BF16 |
|---|---:|---:|---:|---:|---:|
| texts/s (512-token chunks) | 5.4 | **34.1** | 29.6 | 17.4 | 25.6 |
| images/s (280 tokens) | 0.70 | **3.96** | 2.58 | 3.11 | 2.46 |
| audio seconds/s (30 s windows) | 43 | **251** | 145 | 144 | 140 |
| video frames/s (140 tokens) | 2.0 | **11.7** | 8.5 | 8.8 | 8.4 |
| query latency p50 ms | 10.0 | **6.3** | 7.5 | 12.9 | 12.7 |
| query latency p95 ms | 13.3 | **7.6** | 8.0 | 14.0 | 12.8 |
| peak RSS MB (process) | 2652 | 945 | 3254 | 946 | 3374 |
| peak GPU memory used MB | - | 2990 | 3513 | 3692 | 4270 |
| model load s | 0.6 | 0.7 | 0.8 | 0.8 | 1.0 |

What this means for indexing a real personal library on fedora with the Vulkan build: about 34 text chunks,
4 photos, 250 s of audio or 12 video frames (12 s of video at 1 fps) per second. The ~26k images
in `content/` would take about 2 hours, the 1,825 photos in `~/Pictures` about 8 minutes plus
their videos. A search query costs 6 ms to embed.

Notes:

- **Vulkan is the fastest backend on this machine**, ahead of ROCm on every workload. ROCm runs
  everything on the GPU (no CPU fallback; checked with `GGML_SCHED_DEBUG`), so the gap is kernel
  quality for gfx1151 in Fedora's ROCm 6.4 builds. One early ROCm run without a warm-up call
  measured only 32 audio-seconds/s; with the warm-up the bench now does, it is the 145 above.
- **Images are the expensive item.** A 280-token image is 2,520 patches through a 16-layer vision
  encoder, about 0.9 TFLOP, and without flash attention every layer writes and reads a
  2520x2520x12 attention matrix. Batching several images through the encoder at once
  (libmtmd's batch API, which the engine uses) gave no gain, so the encoder is compute- and
  memory-bound, not launch-bound.
- **One input per batch is fastest.** The model attends across all tokens of a batch, so packing
  four 512-token chunks into one 2048-token batch dropped Vulkan from 36 to 28 texts/s. The
  default `batch_tokens` of 512 still packs short inputs (queries, short documents) together.
- **CPU threads**: the engine uses one thread per physical core (16 here). Passing 0 to llama.cpp
  had silently meant a much slower default.

### What the float16 rule costs

Flash attention in llama.cpp casts K and V to float16 when there is no KV cache, so it is off by
default (the model card forbids float16). Measured with it on, Vulkan Q8_0:

| | default | `flash_attn = true` |
|---|---:|---:|
| texts/s | 34.1 | 33.0 |
| images/s | 3.96 | 5.27 |
| audio seconds/s | 251 | 244 |
| video frames/s | 11.7 | 13.1 |
| lowest parity cosine | 0.99745 | 0.99759 |

On these fixtures flash attention changed nothing measurable in quality and made images 33%
faster. It stays off until someone decides the float16 K/V is acceptable for this model.

Vulkan float16 in the kernels themselves (`GGML_VK_DISABLE_F16`, which the engine sets by
default): no measurable speed difference on the 8060S; the hardest parity item went from 0.9964
(F16 on) to 0.9975 (F16 off).

## CUDA

No NVIDIA GPU on fedora, so CUDA is a compile check in `docker/cuda.Containerfile`
(nvidia/cuda:12.9.1-devel-ubuntu24.04, Rust 1.95.0), all six default architectures
(`75;80;86;89;90;120`, 290 CUDA objects):

```text
$ podman run ... rummage-build-cuda cargo build -p rummage-embed --release --features cuda --tests --bins
   Compiling rummage-embed v0.1.0 (/src/rummage/crates/rummage-embed)
    Finished `release` profile [optimized] target(s) in 8m 09s

$ ldd target/release/rummage-bench | grep -E 'cuda|cublas'
	libcudart.so.12 => /usr/local/cuda/lib64/libcudart.so.12
	libcublas.so.12 => /usr/local/cuda/lib64/libcublas.so.12
	libcuda.so.1 => not found        # the NVIDIA driver, present on a machine with a GPU
	libcublasLt.so.12 => /usr/local/cuda/lib64/libcublasLt.so.12
```

The first attempt failed at the final link: ggml enables NCCL (multi-GPU collectives) when it finds
it, and the CUDA image ships it. `build.rs` now sets `GGML_CUDA_NCCL=OFF`.

## Reproducing

```bash
git submodule update --init --depth 1
export RUMMAGE_EMBED_MODEL_DIR=/mnt/models/rummage/gguf   # or let the engine download
# parity, one backend and precision per run
RUMMAGE_EMBED_PRECISION=q8_0 RUMMAGE_PARITY_OUT=parity.json \
  cargo test -p rummage-embed --release --features vulkan --test parity -- --nocapture
# speed
cargo run -p rummage-embed --release --features vulkan --bin rummage-bench -- --json
cargo run -p rummage-embed --release --bin rummage-bench -- \
  --texts 64 --images 12 --audio 8 --videos 2 --queries 100          # CPU
```
