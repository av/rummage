<p align="center">
  <img src="crates/rummage/ui/public/favicon.svg" width="72" alt="">
</p>

<h1 align="center">rummage</h1>

<p align="center">
  <b>Search your photos, videos, recordings and documents by what is in them.</b><br>
  Runs on your machine: one binary, on your GPU (Vulkan, ROCm, CUDA) or the CPU.
</p>

<p align="center">
  <a href="https://github.com/av/rummage/releases"><img src="https://img.shields.io/github/v/release/av/rummage?include_prereleases&label=release" alt="Release"></a>
  <a href="https://github.com/av/rummage/actions/workflows/ci.yml"><img src="https://github.com/av/rummage/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License: Apache-2.0"></a>
</p>

<p align="center">
  <img src="docs/screenshots/hero.webp" width="100%" alt="The search 'birds singing' returns a dawn-chorus recording opened at 2:30, a video opened at 0:50 where a song sparrow starts singing, a nightingale recording, page 1 of a bird-walk flyer PDF, a blackbird recording and a photo of a singing sparrow">
</p>

<p align="center"><sub>One query, <b>“birds singing”</b>, over files named like <code>REC_0451.ogg</code> and <code>IMG_2339.jpg</code>.<br>
rummage <i>listened</i> to the recordings, <i>watched</i> the video and <i>read</i> the PDF.</sub></p>

## What grep and Spotlight can’t find

File names say `IMG_2248.jpg`. rummage searches what is inside: the picture, the sound, the page.

**“humpback whale jumping out of the water”**: breaching humpbacks first, then the rest of the whales. No file name says “whale”.

<img src="docs/screenshots/whales.png" alt="Search 'humpback whale jumping out of the water': three photos of breaching humpbacks first, then a booking PDF and two whale tails; one breaching photo open">

**“a storm at night”**: opens a clip at 0:30, where a sunny beach has given way to lightning.

<img src="docs/screenshots/storm.png" alt="Search 'a storm at night': a video opened at 0:30 on a storm lighting up the clouds, beside photos of lightning">

**“when do the whales come to Hawaii”**: page 2 of a boat booking PDF. “From December to April…”

<img src="docs/screenshots/pdf.png" alt="Search 'when do the whales come to Hawaii': page 2 of a boat booking PDF, the matching paragraph highlighted in its card, beside whale photos and an underwater whale video">

**“how do I bake a loaf of bread”**: the method in a notes file, then the loaves you photographed.

<img src="docs/screenshots/text.png" alt="Search 'how do I bake a loaf of bread': a sourdough note open at the method, beside five photos of loaves">

**“花火”**: Japanese for fireworks. Nothing in the library is in Japanese.

<img src="docs/screenshots/fireworks.png" alt="Search '花火': a fireworks video and five fireworks photos; one photo open">

## Try it

Linux x86_64. This is the Vulkan build: recent AMD, NVIDIA and Intel GPUs, with a CPU fallback.
ROCm, CUDA and CPU builds, Docker images and building from source: [Install](docs/install.md).

```sh
curl -LO https://github.com/av/rummage/releases/download/v0.1.1/rummage-0.1.1-linux-x86_64-vulkan.tar.gz
tar xzf rummage-0.1.1-linux-x86_64-vulkan.tar.gz
cd rummage-0.1.1-linux-x86_64-vulkan
./rummage folders add ~/Pictures
./rummage serve
```

Open <http://127.0.0.1:8099/> and describe what you are looking for. The first run downloads the
model (about 860 MB); results fill in while it indexes. From another terminal:

```sh
./rummage search "birds singing"
./rummage doctor        # GPU, model files, ffmpeg, poppler, config
```

On an AMD Radeon 8060S the demo library above (111 files) indexes in under two minutes, and a
search answers in about 10 ms ([benchmarks](docs/BENCH-fedora.md)).

## Also

- **Search by example.** Drop a photo, a sound or a note on the page to find what is like it.
- **Other languages.** “花火” finds the fireworks; “Nordlichter” finds the northern lights.
  The model is Google DeepMind’s multilingual [EmbeddingGemma 2](https://huggingface.co/google/embeddinggemma-2).
- **Private.** Files and queries never leave your machine. No account, no cloud.
- **Keeps up.** New, changed, moved and deleted files are picked up as they happen.
- **Scriptable.** Everything the page does goes through an HTTP API, and there is a CLI.

## Docs

- [Install](docs/install.md): release builds, Docker, from source
- [Using rummage](docs/usage.md): the web page, the CLI, running behind a proxy
- [Configuration](docs/configuration.md) · [HTTP API](docs/api.md)
- [How it works](docs/how-it-works.md): supported files, indexing, search
- [Development](docs/development.md)
- [Screenshots and demo library](docs/screenshots/README.md): every query and file shown here
  (public-domain and CC0 media from Wikimedia Commons, plus a few documents written for the demo)

## Licence

[Apache License 2.0](LICENSE); see [NOTICE](NOTICE). rummage builds in llama.cpp and ggml (MIT);
[docs/LICENSES-THIRD-PARTY.md](docs/LICENSES-THIRD-PARTY.md) lists every component and its licence.
