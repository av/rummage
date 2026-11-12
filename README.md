<p align="center">
  <img src="crates/rummage/ui/public/favicon.svg" width="72" alt="">
</p>

<h1 align="center">rummage</h1>

<p align="center">
  <b>Find the file by what is in it.</b><br>
  Describe a photo, a sound, a moment in a video or a page in a PDF. rummage finds it.<br>
  Everything stays on your computer. No cloud, no account, no tagging.
</p>

<p align="center">
  <a href="https://github.com/av/rummage/releases"><img src="https://img.shields.io/github/v/release/av/rummage?include_prereleases&label=release" alt="Release"></a>
  <a href="https://github.com/av/rummage/actions/workflows/ci.yml"><img src="https://github.com/av/rummage/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License: Apache-2.0"></a>
</p>

<p align="center">
  <a href="#try-it">Try it</a> ·
  <a href="#what-it-finds">What it finds</a> ·
  <a href="#what-you-get">What you get</a> ·
  <a href="#docs">Docs</a>
</p>

<p align="center">
  <img src="docs/screenshots/hero.webp" width="100%" alt="The search 花火 (Japanese for fireworks) returns a fireworks video and five fireworks photos; each opens in turn">
</p>

<p align="center"><sub>No file or folder name says “fireworks”, and the query is in Japanese. rummage matched what is <i>in</i> the pictures and the video.</sub></p>

## You remember what it looked like, not what it was called

Your library is `IMG_2248.jpg`, `REC_0451.ogg` and `VID_20250504_0611.mp4`. Folder search can’t help.
Neither can scrubbing through a long video for one moment.

rummage looks at your pictures, listens to your recordings, watches your videos and reads your
documents. Then you search all of it from one box, in your own words. Point it at your folders,
type what you remember, and open the result at the exact spot that matched. It doesn’t organise,
move or tag anything: it finds.

## What it finds

| You type | rummage opens |
|---|---|
| **birds singing** | a dawn chorus at 2:30 in a recording; a video at 0:50, the moment a song sparrow starts singing; a nightingale at 2:05 |
| **humpback whale jumping out of the water** | three breaching humpbacks, none of them named “whale” |
| **a storm at night** | a clip at 0:30, where a sunny beach has given way to a night storm |
| **花火** | a fireworks video and fireworks photos |
| **Nordlichter** | the northern lights: aurora photos from Alaska and Lofoten |

Turn the sound on for this one:

<a href="https://cdn.jsdelivr.net/gh/av/rummage@bd1d2de928c07e755dcd4f13711dbf114c57b14b/docs/screenshots/birds-singing.mp4"><img src="docs/screenshots/birds-singing-poster.png" alt="Play the 23-second recording of the search 'birds singing', with sound"></a>

<sub>23 s, MP4 with sound: [play](https://cdn.jsdelivr.net/gh/av/rummage@bd1d2de928c07e755dcd4f13711dbf114c57b14b/docs/screenshots/birds-singing.mp4) · [download](docs/screenshots/birds-singing.mp4)</sub>

<img src="docs/screenshots/whales.png" alt="Search 'humpback whale jumping out of the water': the top three results are photos of breaching humpbacks; the first is open">

<img src="docs/screenshots/storm.png" alt="Search 'a storm at night': a video opened at 0:30 on storm clouds lit by lightning, then two photos of lightning at night">

Photos, videos, recordings, PDFs and text sit in the same results, so a search for the whales also
turns up the booking PDF whose page has a whale photo on it. Every query and file shown here is
listed in [Screenshots and demo library](docs/screenshots/README.md).

## What you get

- **Private.** Your files and searches never leave the machine, and rummage only reads your files:
  its index and thumbnails live in a folder of their own. The model is downloaded once; after that
  it works offline.
- **Opens at the match.** A video starts playing where the match is, an audio file at the matching
  seconds, and a PDF shows the page. Phone videos that browsers can’t play are converted on the fly.
- **Fast.** About 10 ms a search, on a library of 4,500 files too. Results fill in while a folder is
  still being indexed, and new, edited, moved and deleted files are picked up as they happen.
- **Not just English.** The same search works in Japanese and German, as the examples above show.
- **Search by example.** Drop a photo, a sound or a note on the page, or press “More like this” on
  a result. Narrow results by kind, folder and date.
- **One program.** A web page (light and dark, and it works on a phone), a command line and an HTTP
  API for scripts. It uses an AMD, NVIDIA or Intel GPU and falls back to the CPU.

<p align="center"><sub>The same search on a phone.</sub></p>

<table align="center">
  <tr>
    <td><img src="docs/screenshots/phone-results.png" width="260" alt="Search results on a phone"></td>
    <td><img src="docs/screenshots/phone-preview.png" width="260" alt="A result open on a phone, with More like this, Open and Download"></td>
  </tr>
</table>

## Try it

rummage runs on **Linux x86_64 only** for now (Ubuntu 22.04, Debian 12 or newer); there is no macOS
or Windows build yet. This download uses any recent AMD, NVIDIA or Intel GPU through Vulkan, or the
CPU, more slowly. It also needs:

- a CPU with AVX2 (Intel Haswell, AMD Zen or newer);
- [ffmpeg](https://ffmpeg.org), to read video and HEIC photos, and `poppler-utils`, to show PDF pages.

```sh
curl -LO https://github.com/av/rummage/releases/download/v0.1.2/rummage-0.1.2-linux-x86_64-vulkan.tar.gz
tar xzf rummage-0.1.2-linux-x86_64-vulkan.tar.gz
cd rummage-0.1.2-linux-x86_64-vulkan
./rummage folders add ~/Pictures
./rummage serve
```

Open <http://127.0.0.1:8099/> and type something you remember. The first run downloads the model
(about 865 MB); results appear as files are indexed.

On a Radeon 8060S, the 111-file demo library is indexed in under two minutes.
Other builds (ROCm, CUDA, CPU), Docker images and building from source are in
[Install](docs/install.md); the command line and everything the page can do are in
[Using rummage](docs/usage.md).

## Limits

rummage is at an early stage (0.1, a pre-release). It has run on a real library of about 4,500
photos, videos, recordings, notes and PDFs; the [review](docs/REVIEW-2026-10.md) has the numbers and
the [benchmarks](docs/BENCH-fedora.md) compare the GPU backends with the CPU.

- Search ranks by likeness, so a query with no true match still shows the closest files.
- It finds things by what they show and say, not by who is in them: there is no face recognition.
- The first pass over a big library takes a while, and about six to eight times longer without a GPU.

Bug reports and ideas are welcome in [Issues](https://github.com/av/rummage/issues).

## Under the hood

One model, Google DeepMind’s multilingual [EmbeddingGemma 2](https://huggingface.co/google/embeddinggemma-2),
turns text, images, sound and video frames into numbers that can be compared, so a sentence lands
next to the thing it describes. It runs on [llama.cpp](https://github.com/ggml-org/llama.cpp),
built into the binary, and keyword search (names, text, photo dates and cameras) is mixed in with the
meaning search. [How it works](docs/how-it-works.md) has the details.

## Docs

- [Install](docs/install.md): release builds, Docker, from source
- [Using rummage](docs/usage.md): the web page, the command line, running behind a proxy
- [Configuration](docs/configuration.md) · [HTTP API](docs/api.md)
- [How it works](docs/how-it-works.md): supported files, indexing, search
- [Development](docs/development.md)
- [Screenshots and demo library](docs/screenshots/README.md): public-domain and CC0 media from
  Wikimedia Commons, plus a few documents written for the demo

## Licence

[Apache License 2.0](LICENSE); see [NOTICE](NOTICE). rummage builds in llama.cpp and ggml (MIT);
[docs/LICENSES-THIRD-PARTY.md](docs/LICENSES-THIRD-PARTY.md) lists every component and its licence.
