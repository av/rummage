# Install

## Release builds

Each [release](https://github.com/av/rummage/releases) has one Linux x86_64 build per backend,
`rummage-<version>-linux-x86_64-<backend>.tar.gz` (the binary, the README, LICENSE, NOTICE and
THIRD-PARTY-NOTICES), with SHA-256 sums. Every GPU build also runs on the CPU, with a warning,
when it finds no GPU.

| Backend | For | Needs on the system |
|---|---|---|
| `vulkan` | any recent GPU: AMD, NVIDIA, Intel. The fastest build on AMD's Radeon 8060S | the Vulkan loader (`libvulkan.so.1`) and a Vulkan driver (Mesa RADV/ANV, or NVIDIA's) |
| `rocm` | AMD GPUs: gfx1151/1150 (Strix Halo/Point), gfx1100-1102 (RX 7000), gfx1200/1201 (RX 9000), gfx1030 (RX 6000), gfx90a/gfx942 (MI200/MI300) | ROCm 7.x: `libamdhip64.so.7`, hipBLAS, rocBLAS |
| `cuda` | NVIDIA GPUs from Turing to Blackwell (sm 75, 80, 86, 89, 90, 100, 120) | the NVIDIA driver (525 or newer) and cuBLAS 12 (`libcublas.so.12`, `libcublasLt.so.12`, from CUDA 12.x) |
| `cpu` | no GPU | nothing extra |

All builds need glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36) and an x86-64-v3 CPU
(AVX2, FMA: Intel Haswell or AMD Zen and later). llama.cpp and the CUDA runtime are linked in
statically; the vendor libraries above stay dynamic.

Two optional tools add file types: **ffmpeg** (and ffprobe) for video and HEIC/AVIF photos, and
**poppler-utils** (`pdftoppm`, `pdftotext`) for PDF page images. Without ffmpeg, videos are
listed as skipped with the reason; without poppler, PDFs are indexed by their text only.

## First run

```sh
./rummage folders add ~/Pictures     # add as many folders as you like (or add them on the page)
./rummage serve                      # index them and serve the web page
```

Open <http://127.0.0.1:8099/> and describe what you are looking for. The first run downloads the
model (about 865 MB); results fill in while the folders are indexed. From another terminal:

```sh
./rummage search "birds singing"
./rummage doctor        # GPU, model files, ffmpeg, poppler, config
```

On an AMD Radeon 8060S a 770-file sample (photos, audio, video, PDFs and notes) indexes in 251 s
with Vulkan and in 1,648 s on the CPU, and a search takes about 10 ms over 4,500 files
([benchmarks](BENCH-fedora.md)).

## Harbor

rummage is a service in [Harbor](https://github.com/av/harbor). It runs the `ghcr.io/av/rummage`
image that matches the hardware Harbor detects (CPU, CUDA, or Vulkan/ROCm on AMD), as your host
user, with config, index and models in a Harbor workspace and your folder mounted read-only at
`/folders`:

```sh
harbor config set rummage.folders ~/Pictures
harbor pull rummage
harbor up rummage --open          # http://localhost:35100
harbor exec rummage rummage folders add /folders
```

GPU groups, the AMD backend choice, settings and troubleshooting are in
[rummage in Harbor](https://github.com/av/harbor/wiki/2.3.103-Satellite-Rummage).

## Docker

Images are `ghcr.io/av/rummage:<version>-<backend>` and `ghcr.io/av/rummage:latest-<backend>`.
They include ffmpeg and poppler, run as user 1000, keep everything (config, index, models) in
the `/data` volume, listen on port 8099 and have a healthcheck on `/api/v1/health`.

```sh
# Vulkan or ROCm on AMD/Intel: pass the GPU device nodes and the host groups that own them.
docker run -d --name rummage -p 8099:8099 -v rummage-data:/data -v ~/Pictures:/media/Pictures:ro \
  --device /dev/dri --group-add $(getent group render | cut -d: -f3) --group-add $(getent group video | cut -d: -f3) \
  ghcr.io/av/rummage:latest-vulkan
# ROCm: the same plus --device /dev/kfd, with the :latest-rocm image.
# CUDA (with the NVIDIA container toolkit):
docker run -d --name rummage -p 8099:8099 -v rummage-data:/data --gpus all ghcr.io/av/rummage:latest-cuda

docker exec rummage rummage folders add /media/Pictures
docker exec rummage rummage doctor
```

## From source

Needs Rust 1.88 or newer, CMake, a C/C++ compiler and [bun](https://bun.sh) (which builds the
web UI; without it the binary serves a placeholder page). GPU builds also need that backend's
SDK: the Vulkan headers and `glslc`, ROCm 7, or the CUDA 12 toolkit.

```sh
git clone --recursive https://github.com/av/rummage
cd rummage
cargo build --release                 # CPU; GPU builds: --features vulkan|rocm|cuda
./target/release/rummage serve
```

`docker build -f docker/Dockerfile --build-arg BACKEND=rocm -t rummage:rocm .` builds an image
for any backend without installing its SDK.
