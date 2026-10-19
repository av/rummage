# rummage-embed

The embedding engine for rummage: Google DeepMind's **EmbeddingGemma 2** (`google/embeddinggemma-2`,
Apache 2.0) running on **llama.cpp**, which is built into the program as static libraries
(libllama + libmtmd + ggml). It turns text, images, audio and video into 768-number vectors in one
shared space, and implements `rummage_core::embed::Embedder`. On fedora's Radeon 8060S (Vulkan) it
embeds about 34 text chunks of 512 tokens, 4 photos or 250 s of audio per second, and a query
in 6 ms.

```rust
use rummage_embed::{EmbedInput, Embedder, Engine, EngineConfig};

let engine = Engine::new(EngineConfig::default())?; // downloads the Q8_0 GGUFs on first use
let v = engine.embed(&[
    EmbedInput::Query("a cat sleeping in the sun".into()),
    EmbedInput::Document { title: None, text: "Cats sleep up to 16 hours a day".into() },
])?;
```

## What the engine does

| Input | What the model sees | Notes |
|---|---|---|
| `Query(q)` | `<bos>task: search result \| query: {q}<eos>` | prefix from the model card |
| `Document{title,text}` | `<bos>title: {title or none} \| text: {text}<eos>` | cut to `max_tokens` (default 4096) |
| `Image` | `<bos><\|image>` 280 soft tokens `<image\|><eos>` | `image_tokens` 70..1120, default 280 |
| `Audio` (16 kHz mono f32) | `<bos><\|audio>` 25 tokens/s `<audio\|><eos>` | one block per 30 s part (see below) |
| `VideoFrames` (1 fps) | `<bos>` + per frame `<\|image>` 140 soft tokens `<image\|>` + `<eos>` | `video_frame_tokens`, default 140 |

The sequence is mean-pooled, normalised, and (if `dim` is 512/256/128) cut to the Matryoshka size
and normalised again. Inputs of every kind can be mixed in one `embed()` call. Media soft tokens
from libmtmd go into the same non-causal batch as the text tokens (`llama_batch_ext`).

Main settings (`EngineConfig`):

| Field | Default | Meaning |
|---|---|---|
| `precision` | `Q8_0` | or `Bf16`; `f16` is refused |
| `dim` | 768 | Matryoshka cut: 512, 256 or 128 |
| `gpu_layers` | -1 (all) | 0 runs on the CPU even in a GPU build |
| `threads` | 0 (one per physical core) | CPU threads |
| `max_tokens` | 4096 | longest input; also llama.cpp's micro-batch size |
| `batch_tokens` | 512 | packing target: short inputs share a batch, long ones go alone |
| `image_tokens` / `video_frame_tokens` | 280 / 140 | vision tokens per image / frame |
| `flash_attn` | false | float16 K/V, see below |

Batches stay small on purpose: the model attends across every token of a batch (masked between
sequences), so packing four 512-token documents into one 2048-token batch was about 20% slower
on Vulkan than running them one by one (28 vs 36 documents/s).

Images and video frames are decoded and resized on all cores before the engine lock is taken;
the GPU work is serialised behind one mutex. `RUMMAGE_EMBED_TIMING=1` prints per-call stage
times, `RUMMAGE_LLAMA_LOG=0` shows llama.cpp's debug log.

Things that had to be done by hand to match the official model (checked by the parity test):

- **Image resizing.** The official processor always scales an image (up or down) to the largest
  size that fits the token budget, with sides a multiple of 48 px. llama.cpp only rescales images
  outside its limits, so a 500x416 photo would get 90 tokens (480x432) instead of 270. The engine resizes
  with the official formula (bicubic, a = -0.5) before handing the pixels to libmtmd.
- **Video.** llama.cpp has no video path for this model that matches the official one (its
  ffmpeg-based video helper adds "Video:" and timestamp text meant for chat models). The official
  `EmbeddingGemma2VideoProcessor` samples 1 fps (at most 32 frames), sizes each frame for 140
  tokens and lays them out as one `<|image>` ... `<image|>` block per frame with no timestamps
  (`add_timestamps: false`). So the engine takes frames (`VideoFrames`), resizes each to the
  140-token budget and sends them as a run of images. Frame extraction (ffmpeg) is the caller's job.
- **Audio longer than 30 s.** The official processor truncates one audio input at 30 s
  (2999 mel frames, 750 tokens). libmtmd instead splits long audio into 30 s chunks inside one
  `<|audio>` block. The engine splits audio into 30 s parts, each in its own `<|audio>` ... `<audio|>`
  block, which is how the official processor lays out several audio parts in one input. Use windows
  of at most 30 s for indexing; the parity test covers clips shorter than that.
- **`<eos>`.** The tokenizer appends `<eos>` to every input, media included; the engine does too.

### Float16

The model card says EmbeddingGemma 2 must run in bfloat16 or float32, never float16. What the
engine does about it:

- Weights are Q8_0 (default) or BF16; `Precision` refuses `f16`.
- The model has no KV cache (llama.cpp builds it with `build_attn_inp_no_cache`); the context still
  asks for F32 K/V types in case that ever changes.
- **Flash attention is off** for both the text model and the vision/audio encoders. With no KV
  cache, llama.cpp casts K and V to F16 for flash attention. `EngineConfig::flash_attn` turns it on
  (`RUMMAGE_EMBED_FLASH_ATTN=1` in the parity test). On the fixtures it changed no parity number
  measurably and made images 33% faster on Vulkan (see [BENCH.md](BENCH.md)); whether to allow it
  is a product decision.
- The mmproj files contain no F16 tensors (Q8_0 or BF16 weights, F32 norms and biases).
- **Vulkan**: by default ggml-vulkan runs some vision/audio encoder kernels with half-precision
  shaders and accumulators. The engine sets `GGML_VK_DISABLE_F16=1` before the backend starts,
  so Vulkan computes in F32 (on the Radeon 8060S this costs no measurable speed, and the
  hardest parity item moves from 0.9964 to 0.9975). `RUMMAGE_VULKAN_F16=1` opts back in.
- **CPU**: Q8_0 matmuls use int8 dot products, BF16 weights use BF16 dot products, the rest F32.
- **ROCm/CUDA**: Q8_0 matmuls go through ggml's int8 MMQ kernels and attention is F32
  (no flash attention). The per-backend parity numbers in [BENCH.md](BENCH.md) are the check.

## Model files

`ggml-org/embeddinggemma-2-GGUF`, pinned to revision `bfcd2987` with SHA-256 checks
(`src/model_files.rs`):

| Precision | Text model | Vision + audio (mmproj) |
|---|---|---|
| Q8_0 (default) | `embeddinggemma-2-Q8_0.gguf`, 310 MB | `mmproj-embeddinggemma-2-Q8_0.gguf`, 555 MB |
| BF16 | `embeddinggemma-2-BF16.gguf`, 558 MB | `mmproj-embeddinggemma-2-BF16.gguf`, 982 MB |

They are downloaded into `EngineConfig::model_dir` (default `~/.local/share/rummage/models`) on
first use, or you can pass paths in `EngineConfig::model` / `mmproj`. A text-only engine
(`media: false`) skips the mmproj.

## Building

llama.cpp is a git submodule (`crates/rummage-embed/llama.cpp`, pinned to release b11721,
`ff5a2337`, which includes PR 30054 "support embeddinggemma2"). Fetch it once:

```bash
git submodule update --init --depth 1
```

`build.rs` builds libllama, libmtmd and ggml with CMake as static libraries and links them in,
so each build is one binary. Requirements for every backend: a C++17 compiler, CMake ≥ 3.21
(`pip install --user cmake` works without root) and optionally Ninja.

| Feature | Backend | Extra build deps | Runtime deps |
|---|---|---|---|
| `cpu` (default) | CPU | none | none |
| `vulkan` | Vulkan (NVIDIA, AMD, Intel) | Vulkan headers + loader dev files, `glslc` | `libvulkan.so.1` + a Vulkan driver |
| `rocm` | AMD HIP | ROCm: `hipcc`/HIP clang, `hipblas-devel`, `rocblas-devel` | ROCm runtime, hipBLAS, rocBLAS |
| `cuda` | NVIDIA CUDA | CUDA toolkit (`nvcc`, cuBLAS) | NVIDIA driver, cuBLAS (the CUDA runtime is linked statically) |

```bash
cargo build -p rummage-embed --release                      # cpu
cargo build -p rummage-embed --release --features vulkan
cargo build -p rummage-embed --release --features rocm
cargo build -p rummage-embed --release --features cuda
```

Build knobs (environment variables read by `build.rs`):

| Variable | Default | Meaning |
|---|---|---|
| `RUMMAGE_GGML_NATIVE` | `1` | `1` tunes CPU code for the build machine; `0` for portable release builds |
| `RUMMAGE_HIP_TARGETS` | `gfx1151;gfx1150;gfx1100;gfx1101;gfx1102;gfx1200;gfx1201;gfx1030;gfx90a;gfx942` | AMD GPU targets |
| `RUMMAGE_CUDA_ARCHS` | `75;80;86;89;90;100;120` | CUDA architectures (Turing to Blackwell) |
| `RUMMAGE_LLAMA_CPP_DIR` | the submodule | use another llama.cpp checkout |
| `RUMMAGE_CMAKE_ARGS` | | extra `-DKEY=VALUE` CMake arguments |

### Container builds

Fedora 43 lacks the Vulkan SDK bits and `rocblas-devel` by default, and there is no NVIDIA
toolkit on the dev box, so `docker/` has one build image per GPU backend. The Vulkan and ROCm
images are Fedora 43 (same glibc as the host), so the binaries they produce run on the host.

```bash
cd ~/code/rummage   # the workspace root
podman build -t rummage-build-vulkan -f crates/rummage-embed/docker/vulkan.Containerfile crates/rummage-embed/docker
podman build -t rummage-build-rocm   -f crates/rummage-embed/docker/rocm.Containerfile   crates/rummage-embed/docker
podman build -t rummage-build-cuda   -f crates/rummage-embed/docker/cuda.Containerfile   crates/rummage-embed/docker

# Build with the workspace mounted at the same path. In rootless podman, keep-id makes the
# files yours; the CUDA image has Rust under /root, so it runs as the container's root instead
# (which rootless podman maps to you anyway).
run() { img=$1; shift; podman run --rm --userns=keep-id --security-opt label=disable \
  -v "$PWD:$PWD" -w "$PWD" -e CARGO_HOME="$PWD/target/cargo-home" "$img" "$@"; }
run rummage-build-vulkan cargo build -p rummage-embed --release --features vulkan --target-dir target/vulkan
run rummage-build-rocm   cargo build -p rummage-embed --release --features rocm   --target-dir target/rocm
podman run --rm --security-opt label=disable -v "$PWD:$PWD" -w "$PWD" rummage-build-cuda \
  cargo build -p rummage-embed --release --features cuda --target-dir target/cuda
```

Measured on fedora (32 threads): CPU build of llama.cpp about 20 s, Vulkan about 1 min (shader
generation), ROCm about 5 min for all seven GPU targets (1 min for gfx1151 alone), CUDA about
8 min for all six architectures.

Workarounds for the pinned llama.cpp and the toolchains, in `build.rs`:

- ROCm 6.x: llama.cpp's `topk-moe.cu` (since ggml-org/llama.cpp#29181) calls `__ballot_sync`,
  which ggml's own HIP header hides, so the HIP build fails. `src/hip_compat.h` is force-included
  and maps it to `__ballot`, which is the same thing with a full mask.
- CUDA: ggml turns on NCCL (multi-GPU collectives) whenever it finds it, which adds a runtime
  library and broke the link in the NVIDIA image. It is switched off; one GPU is enough.
- CUDA: the binary does not link the driver library (`libcuda.so.1`). ggml is built with
  `GGML_CUDA_NO_VMM` (VMM is the only part that calls the driver API directly) and the CUDA
  runtime is linked statically, which loads the driver when it starts. So a CUDA build starts on
  a machine without the NVIDIA driver and runs on the CPU, as the Vulkan and ROCm builds do
  without a GPU.

The static libraries are built position-independent, since Rust links a PIE binary and ROCm's
clang on Ubuntu does not default to PIC. The force-included `hip_compat.h` only applies to
ROCm 6; ROCm 7 declares `__ballot_sync` itself.

Running GPU builds needs the device in the container or on the host: `/dev/dri` for Vulkan,
`/dev/kfd` + `/dev/dri` for ROCm, the NVIDIA container toolkit for CUDA.

## Tests

```bash
cargo test -p rummage-embed --release                                 # unit tests + parity (Q8_0)
RUMMAGE_EMBED_PRECISION=bf16 cargo test -p rummage-embed --release --test parity -- --nocapture
```

The parity test (`tests/parity.rs`) embeds 11 fixtures (2 queries, 3 documents, 3 images, 2 speech
clips, 1 video) and compares them with vectors from the official model
(`tests/reference/reference.json`, made by `tests/reference/make_reference.py` with
sentence-transformers in float32 on CPU). It checks, per item: the token count equals the official
processor's; cosine ≥ 0.98 (Q8_0) or ≥ 0.995 (BF16), at 768 and at 256 dims; batched equals
one-by-one; and that each query is nearest its matching document, image and audio clip.

Environment: `RUMMAGE_EMBED_MODEL_DIR` (where the GGUFs are or go), or `RUMMAGE_EMBED_MODEL` +
`RUMMAGE_EMBED_MMPROJ`; `RUMMAGE_EMBED_GPU_LAYERS=0` to force CPU in a GPU build;
`RUMMAGE_EMBED_FLASH_ATTN=1` to test with flash attention; `RUMMAGE_PARITY_OUT=file.json` to save
the numbers; `RUMMAGE_SKIP_PARITY=1` to skip on purpose (it never skips silently).
The video item needs `ffmpeg` on PATH.

Fixtures (`tests/reference/fixtures`, 0.8 MB): three CC0 photos from Wikimedia Commons
("Loaf of sourdough bread cooling", "Cat sleeping on the front porch", "St Mary's Lighthouse at high
tide", 500 px thumbnails), two speech clips made with espeak-ng, and a 4 s video made from the cat
photo with ffmpeg. Sources, commands and licences: `tests/reference/fixtures/LICENSES.md`.

## Benchmark

```bash
cargo run -p rummage-embed --release --features vulkan --bin rummage-bench -- --json
```

`--backend cpu` runs a GPU build on the CPU; `--precision bf16`, `--batch-tokens`, `--flash-attn`
and the item counts are flags too. Results for fedora (AMD Ryzen AI Max+ 395,
Radeon 8060S / gfx1151) are in [BENCH.md](BENCH.md).
