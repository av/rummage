<p align="center">
  <img src="crates/rummage/ui/public/favicon.svg" width="72" alt="">
</p>

<h1 align="center">rummage</h1>

<p align="center">
  <b>Search your photos, videos, recordings and documents by what is in them.</b><br>
  Nothing leaves your machine: one binary, on your GPU (Vulkan, ROCm, CUDA) or the CPU.
</p>

<p align="center">
  <a href="https://github.com/av/rummage/releases"><img src="https://img.shields.io/github/v/release/av/rummage?include_prereleases&label=release" alt="Release"></a>
  <a href="https://github.com/av/rummage/actions/workflows/ci.yml"><img src="https://github.com/av/rummage/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License: Apache-2.0"></a>
</p>

<p align="center">
  <img src="docs/screenshots/hero.webp" width="100%" alt="The search 花火 (Japanese for fireworks) returns a fireworks video and five fireworks photos; each opens in turn">
</p>

<p align="center"><sub>Not a single file name or tag says “fireworks”: rummage matched what is <i>in</i> the pictures and the video.</sub></p>

## It listens, too

**“birds singing”** finds a dawn chorus at 2:30, a video at the 50-second mark where a song sparrow
starts singing, a nightingale and a blackbird, in files named `REC_0451.ogg` and
`VID_20250504_0611.mp4`. Turn the sound on:

<a href="https://cdn.jsdelivr.net/gh/av/rummage@bd1d2de928c07e755dcd4f13711dbf114c57b14b/docs/screenshots/birds-singing.mp4"><img src="docs/screenshots/birds-singing-poster.png" alt="Play the 23-second recording of the search 'birds singing', with sound"></a>

<sub>23 s, MP4 with sound: [play](https://cdn.jsdelivr.net/gh/av/rummage@bd1d2de928c07e755dcd4f13711dbf114c57b14b/docs/screenshots/birds-singing.mp4) · [download](docs/screenshots/birds-singing.mp4)</sub>

## It looks inside

**“humpback whale jumping out of the water”**: three breaching humpbacks, none of them named “whale”.

<img src="docs/screenshots/whales.png" alt="Search 'humpback whale jumping out of the water': the top three results are photos of breaching humpbacks; the first is open">

**“a storm at night”**: opens a clip at 0:30, where a sunny beach has given way to a night storm.

<img src="docs/screenshots/storm.png" alt="Search 'a storm at night': a video opened at 0:30 on storm clouds lit by lightning, then two photos of lightning at night">

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

Add more folders the same way (or from the page), open <http://127.0.0.1:8099/> and describe what
you are looking for. The first run downloads the model (about 860 MB); results fill in while it
indexes. From another terminal:

```sh
./rummage search "birds singing"
./rummage doctor        # GPU, model files, ffmpeg, poppler, config
```

On an AMD Radeon 8060S the demo library above (111 files) indexes in under two minutes and a
search answers in about 10 ms; the CPU indexes 6 to 8 times slower ([benchmarks](docs/BENCH-fedora.md)).

## What else it does

- **Search by example.** Drop a photo, a sound or a note on the page to find what is like it.
- **Other languages.** Like “花火” above, “Nordlichter” (German) finds the northern lights.
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
