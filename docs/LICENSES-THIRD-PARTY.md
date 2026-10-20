# Third-party licences

rummage is licensed under the Apache License 2.0. This page lists what it contains and what its
release archives and Docker images ship, with each licence and whether it allows distributing
rummage under Apache-2.0. Audited 2026-10-20 against Cargo.lock, crates/rummage/ui/bun.lock,
the llama.cpp submodule and docker/Dockerfile.

**Result: nothing incompatible.** No GPL or LGPL code is linked into the rummage binary. The
only copyleft code in the binary is MPL-2.0 (the symphonia audio decoders and option-ext), which
is file-level copyleft: it may be combined with Apache-2.0 code as long as those files stay
available under MPL-2.0 (they are unmodified and published on crates.io). The Docker images
contain GPL programs (ffmpeg, poppler-utils) that rummage runs as separate processes; that is
aggregation, and their source locations are given below and in THIRD-PARTY-NOTICES.

## The binary

| Component | How it is included | Licence | Compatible |
|---|---|---|---|
| llama.cpp and ggml (submodule `crates/rummage-embed/llama.cpp`) | built from source, linked statically | MIT | yes; licence text ships in THIRD-PARTY-NOTICES |
| SQLite (via `libsqlite3-sys`, `bundled`) | built from source, linked statically | public domain | yes |
| libwebp (via `libwebp-sys`) | built from source, linked statically | BSD-3-Clause | yes |
| NVIDIA CUDA runtime (`cudart_static`, CUDA build only) | linked statically | NVIDIA CUDA Toolkit EULA (proprietary, redistributable under Attachment A) | yes: Apache-2.0 allows combining with proprietary code; the CUDA build as a whole is also subject to the EULA for that part |
| cuBLAS, ROCm (HIP, hipBLAS, rocBLAS), Vulkan loader | loaded from the system at run time; not in the release archives | see "Docker images" | yes (not distributed with the binary) |
| Web UI bundle: svelte 5.57.2, @lucide/svelte 1.59.0, clsx 2.1.1 | compiled into the UI embedded in the binary | MIT, ISC, MIT | yes |
| Rust crates (table below) | compiled in | permissive or MPL-2.0 | yes |

### Rust crates

Every crate linked into the binary on x86_64 Linux with all backend features
(`cargo tree --workspace -e normal --target x86_64-unknown-linux-gnu --all-features`; build-only
and dev-only dependencies are not in the binary and are left out). Summary:

| Licence (as declared) | Crates | Compatible |
|---|---|---|
| MIT OR Apache-2.0 | 113 | yes |
| MIT | 46 | yes |
| MPL-2.0 | 15 | yes (file-level copyleft; unmodified, source on crates.io) |
| MIT/Apache-2.0 | 10 | yes |
| Apache-2.0 OR MIT | 8 | yes |
| Unlicense OR MIT | 4 | yes |
| ISC | 4 | yes |
| BSD-3-Clause | 3 | yes |
| Zlib OR Apache-2.0 OR MIT | 2 | yes |
| Unlicense/MIT | 2 | yes |
| MIT OR Zlib OR Apache-2.0 | 2 | yes |
| MIT OR Apache-2.0 OR Zlib | 2 | yes |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 2 | yes |
| BSD-3-Clause OR Apache-2.0 | 2 | yes |
| BSD-2-Clause OR Apache-2.0 OR MIT | 2 | yes |
| BSD-2-Clause | 2 | yes |
| Zlib | 1 | yes |
| MIT AND BSD-3-Clause | 1 | yes |
| CDLA-Permissive-2.0 | 1 | yes |
| CC0-1.0 OR MIT-0 OR Apache-2.0 | 1 | yes |
| CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | 1 | yes |
| CC0-1.0 | 1 | yes |
| BSD-3-Clause/MIT | 1 | yes |
| (Apache-2.0 OR MIT) AND BSD-3-Clause | 1 | yes |
| Apache-2.0 OR ISC OR MIT | 1 | yes |
| Apache-2.0 OR BSL-1.0 | 1 | yes |
| Apache-2.0 AND ISC | 1 | yes |
| Apache-2.0 | 1 | yes |
| 0BSD OR MIT OR Apache-2.0 | 1 | yes |

All crates:

| Crate | Version | Licence |
|---|---|---|
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| aes | 0.9.2 | MIT OR Apache-2.0 |
| aho-corasick | 1.1.5 | Unlicense OR MIT |
| alloc-no-stdlib | 2.0.4 | BSD-3-Clause |
| alloc-stdlib | 0.2.4 | BSD-3-Clause |
| anstream | 1.0.0 | MIT OR Apache-2.0 |
| anstyle | 1.0.14 | MIT OR Apache-2.0 |
| anstyle-parse | 1.0.0 | MIT OR Apache-2.0 |
| anstyle-query | 1.1.5 | MIT OR Apache-2.0 |
| anyhow | 1.0.104 | MIT OR Apache-2.0 |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT |
| axum | 0.8.9 | MIT |
| axum-core | 0.5.6 | MIT |
| base64 | 0.22.1 | MIT OR Apache-2.0 |
| base64 | 0.23.1 | MIT OR Apache-2.0 |
| bitflags | 1.3.2 | MIT/Apache-2.0 |
| bitflags | 2.13.2 | MIT OR Apache-2.0 |
| blake3 | 1.8.7 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 |
| block-buffer | 0.12.1 | MIT OR Apache-2.0 |
| block-padding | 0.4.2 | MIT OR Apache-2.0 |
| brotli-decompressor | 5.0.3 | BSD-3-Clause/MIT |
| bstr | 1.13.1 | MIT OR Apache-2.0 |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| byteorder-lite | 0.1.0 | Unlicense OR MIT |
| bytes | 1.12.1 | MIT |
| cbc | 0.2.1 | MIT OR Apache-2.0 |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 |
| chacha20 | 0.10.2 | MIT OR Apache-2.0 |
| cipher | 0.5.2 | MIT OR Apache-2.0 |
| clap | 4.6.7 | MIT OR Apache-2.0 |
| clap_builder | 4.6.7 | MIT OR Apache-2.0 |
| clap_derive | 4.6.7 | MIT OR Apache-2.0 |
| clap_lex | 1.1.1 | MIT OR Apache-2.0 |
| colorchoice | 1.0.5 | MIT OR Apache-2.0 |
| color_quant | 1.1.0 | MIT |
| const-oid | 0.10.2 | Apache-2.0 OR MIT |
| constant_time_eq | 0.4.2 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| core_detect | 1.0.0 | MIT/Apache-2.0 |
| cpubits | 0.1.1 | MIT OR Apache-2.0 |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 |
| cpufeatures | 0.3.1 | MIT OR Apache-2.0 |
| crc32fast | 1.5.2 | MIT OR Apache-2.0 |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 |
| crypto-common | 0.2.2 | MIT OR Apache-2.0 |
| digest | 0.10.7 | MIT OR Apache-2.0 |
| digest | 0.11.3 | MIT OR Apache-2.0 |
| dirs | 6.0.0 | MIT OR Apache-2.0 |
| dirs-sys | 0.5.0 | MIT OR Apache-2.0 |
| ecb | 0.2.1 | MIT OR Apache-2.0 |
| encoding_rs | 0.8.42 | (Apache-2.0 OR MIT) AND BSD-3-Clause |
| equivalent | 1.0.3 | Apache-2.0 OR MIT |
| errno | 0.3.14 | MIT OR Apache-2.0 |
| extended | 0.1.0 | MIT |
| fallible-iterator | 0.3.0 | MIT/Apache-2.0 |
| fallible-streaming-iterator | 0.1.9 | MIT/Apache-2.0 |
| fax | 0.2.7 | MIT |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 |
| flate2 | 1.1.10 | MIT OR Apache-2.0 |
| foldhash | 0.2.0 | Zlib |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 |
| futures-core | 0.3.34 | MIT OR Apache-2.0 |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 |
| futures-sink | 0.3.34 | MIT OR Apache-2.0 |
| futures-task | 0.3.34 | MIT OR Apache-2.0 |
| futures-util | 0.3.34 | MIT OR Apache-2.0 |
| generic-array | 0.14.7 | MIT |
| getrandom | 0.2.17 | MIT OR Apache-2.0 |
| getrandom | 0.4.3 | MIT OR Apache-2.0 |
| gif | 0.14.2 | MIT OR Apache-2.0 |
| globset | 0.4.20 | Unlicense OR MIT |
| half | 2.7.1 | MIT OR Apache-2.0 |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 |
| hashlink | 0.12.2 | MIT OR Apache-2.0 |
| heck | 0.5.0 | MIT OR Apache-2.0 |
| http | 1.5.0 | MIT OR Apache-2.0 |
| http-body | 1.1.0 | MIT |
| http-body-util | 0.1.5 | MIT |
| http-range-header | 0.4.2 | MIT |
| httparse | 1.10.1 | MIT OR Apache-2.0 |
| httpdate | 1.0.3 | MIT OR Apache-2.0 |
| hybrid-array | 0.4.15 | MIT OR Apache-2.0 |
| hyper | 1.12.0 | MIT |
| hyper-util | 0.1.21 | MIT |
| image | 0.25.10 | MIT OR Apache-2.0 |
| image-webp | 0.2.4 | MIT OR Apache-2.0 |
| indexmap | 2.14.2 | Apache-2.0 OR MIT |
| inotify | 0.11.5 | ISC |
| inotify-sys | 0.1.8 | ISC |
| inout | 0.2.4 | MIT OR Apache-2.0 |
| is_terminal_polyfill | 1.70.2 | MIT OR Apache-2.0 |
| itoa | 1.0.18 | MIT OR Apache-2.0 |
| kamadak-exif | 0.6.1 | BSD-2-Clause |
| lazy_static | 1.5.1 | MIT OR Apache-2.0 |
| libc | 0.2.191 | MIT OR Apache-2.0 |
| libsqlite3-sys | 0.38.2 | MIT |
| libwebp-sys | 0.9.6 | MIT |
| log | 0.4.34 | MIT OR Apache-2.0 |
| lopdf | 0.45.0 | MIT |
| matchers | 0.2.0 | MIT |
| matchit | 0.8.4 | MIT AND BSD-3-Clause |
| md-5 | 0.11.0 | MIT OR Apache-2.0 |
| memchr | 2.8.3 | Unlicense OR MIT |
| mime | 0.3.17 | MIT OR Apache-2.0 |
| mime_guess | 2.0.5 | MIT |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| mio | 1.2.4 | MIT |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 |
| multer | 3.1.0 | MIT |
| multiversion_no_op | 1.0.0 | Apache-2.0 OR MIT |
| mutate_once | 0.1.2 | BSD-2-Clause |
| nom | 8.0.0 | MIT |
| notify | 8.2.0 | CC0-1.0 |
| notify-types | 2.1.0 | MIT OR Apache-2.0 |
| nu-ansi-term | 0.50.3 | MIT |
| num-traits | 0.2.19 | MIT OR Apache-2.0 |
| once_cell | 1.21.4 | MIT OR Apache-2.0 |
| option-ext | 0.2.0 | MPL-2.0 |
| pastey | 0.2.3 | MIT OR Apache-2.0 |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT |
| png | 0.18.1 | MIT OR Apache-2.0 |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 |
| quick-error | 2.0.1 | MIT/Apache-2.0 |
| quote | 1.0.47 | MIT OR Apache-2.0 |
| rand | 0.10.3 | MIT OR Apache-2.0 |
| rand_core | 0.10.1 | MIT OR Apache-2.0 |
| rangemap | 1.8.0 | MIT/Apache-2.0 |
| regex | 1.13.1 | MIT OR Apache-2.0 |
| regex-automata | 0.4.18 | MIT OR Apache-2.0 |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 |
| ring | 0.17.14 | Apache-2.0 AND ISC |
| rusqlite | 0.40.2 | MIT |
| rust-embed | 8.13.0 | MIT |
| rust-embed-impl | 8.13.0 | MIT |
| rust-embed-utils | 8.13.0 | MIT |
| rustls | 0.23.46 | Apache-2.0 OR ISC OR MIT |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 |
| rustls-webpki | 0.103.15 | ISC |
| ryu | 1.0.23 | Apache-2.0 OR BSL-1.0 |
| same-file | 1.0.6 | Unlicense/MIT |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde_core | 1.0.229 | MIT OR Apache-2.0 |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 |
| serde_json | 1.0.154 | MIT OR Apache-2.0 |
| serde_path_to_error | 0.1.20 | MIT OR Apache-2.0 |
| serde_spanned | 1.1.2 | MIT OR Apache-2.0 |
| serde_urlencoded | 0.7.1 | MIT/Apache-2.0 |
| sha2 | 0.10.9 | MIT OR Apache-2.0 |
| sha2 | 0.11.1 | MIT OR Apache-2.0 |
| sharded-slab | 0.1.7 | MIT |
| shellexpand | 3.1.2 | MIT/Apache-2.0 |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 |
| simd-adler32 | 0.3.10 | MIT |
| simdutf8 | 0.1.5 | MIT OR Apache-2.0 |
| slab | 0.4.12 | MIT |
| smallvec | 1.16.3 | MIT OR Apache-2.0 |
| socket2 | 0.6.5 | MIT OR Apache-2.0 |
| spin | 0.9.9 | MIT |
| stringprep | 0.1.5 | MIT/Apache-2.0 |
| strsim | 0.11.1 | MIT |
| subtle | 2.6.1 | BSD-3-Clause |
| symphonia | 0.5.5 | MPL-2.0 |
| symphonia-bundle-flac | 0.5.5 | MPL-2.0 |
| symphonia-bundle-mp3 | 0.5.5 | MPL-2.0 |
| symphonia-codec-aac | 0.5.5 | MPL-2.0 |
| symphonia-codec-adpcm | 0.5.5 | MPL-2.0 |
| symphonia-codec-alac | 0.5.5 | MPL-2.0 |
| symphonia-codec-pcm | 0.5.5 | MPL-2.0 |
| symphonia-codec-vorbis | 0.5.5 | MPL-2.0 |
| symphonia-core | 0.5.5 | MPL-2.0 |
| symphonia-format-isomp4 | 0.5.5 | MPL-2.0 |
| symphonia-format-ogg | 0.5.5 | MPL-2.0 |
| symphonia-format-riff | 0.5.5 | MPL-2.0 |
| symphonia-metadata | 0.5.5 | MPL-2.0 |
| symphonia-utils-xiph | 0.5.5 | MPL-2.0 |
| syn | 2.0.119 | MIT OR Apache-2.0 |
| syn | 3.0.9 | MIT OR Apache-2.0 |
| sync_wrapper | 1.0.2 | Apache-2.0 |
| thiserror | 2.0.21 | MIT OR Apache-2.0 |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 |
| thread_local | 1.1.10 | MIT OR Apache-2.0 |
| tiff | 0.11.3 | MIT |
| tinyvec | 1.13.3 | Zlib OR Apache-2.0 OR MIT |
| tokio | 1.53.3 | MIT |
| tokio-macros | 2.7.2 | MIT |
| tokio-stream | 0.1.19 | MIT |
| tokio-util | 0.7.20 | MIT |
| toml | 1.1.8+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_datetime | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_edit | 0.25.17+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_parser | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_writer | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 |
| tower | 0.5.3 | MIT |
| tower-http | 0.6.11 | MIT |
| tower-layer | 0.3.3 | MIT |
| tower-service | 0.3.3 | MIT |
| tracing | 0.1.44 | MIT |
| tracing-attributes | 0.1.31 | MIT |
| tracing-core | 0.1.36 | MIT |
| tracing-log | 0.2.0 | MIT |
| tracing-subscriber | 0.3.23 | MIT |
| typenum | 1.20.1 | MIT OR Apache-2.0 |
| unicase | 2.10.0 | MIT OR Apache-2.0 |
| unicode-bidi | 0.3.18 | MIT OR Apache-2.0 |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| unicode-normalization | 0.1.26 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| unicode-properties | 0.1.4 | MIT/Apache-2.0 |
| untrusted | 0.9.0 | ISC |
| ureq | 3.4.2 | MIT OR Apache-2.0 |
| ureq-proto | 0.6.5 | MIT OR Apache-2.0 |
| utf8-zero | 0.8.1 | MIT OR Apache-2.0 |
| utf8parse | 0.2.2 | Apache-2.0 OR MIT |
| utoipa | 6.0.0 | MIT OR Apache-2.0 |
| utoipa-axum | 0.3.0 | MIT OR Apache-2.0 |
| utoipa-gen | 6.0.1 | MIT OR Apache-2.0 |
| walkdir | 2.5.0 | Unlicense/MIT |
| webp | 0.3.1 | MIT OR Apache-2.0 |
| webpki-roots | 1.0.9 | CDLA-Permissive-2.0 |
| weezl | 0.1.12 | MIT OR Apache-2.0 |
| weezl | 0.2.1 | MIT OR Apache-2.0 |
| winnow | 1.0.4 | MIT |
| zerocopy | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zerocopy-derive | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zeroize | 1.9.1 | Apache-2.0 OR MIT |
| zmij | 1.0.23 | MIT |
| zune-core | 0.5.3 | MIT OR Apache-2.0 OR Zlib |
| zune-jpeg | 0.5.15 | MIT OR Apache-2.0 OR Zlib |

## The web UI

Only three npm packages end up in the shipped UI bundle (checked from the production build's
source map): svelte (MIT), @lucide/svelte (ISC) and clsx (MIT). Everything else is build or test
tooling that is not distributed (vite, svelte-check, typescript, Playwright, axe-core and their
dependencies).

Dev toolchain (`bun install` in crates/rummage/ui, 91 packages): not distributed, listed for
completeness.

| Licence | Packages | Notes |
|---|---|---|
| MIT | 71 | (most packages) |
| Apache-2.0 | 8 | @playwright/test, aria-query, axobject-query, detect-libc, playwright, playwright-core, typescript, yaml-ast-parser |
| MPL-2.0 | 5 | @axe-core/playwright, axe-core, lightningcss, lightningcss-linux-x64-gnu, lightningcss-linux-x64-musl |
| ISC | 4 | @lucide/svelte, minimatch, picocolors, yargs-parser |
| Python-2.0 | 1 | argparse |
| (MIT OR CC0-1.0) | 1 | type-fest |
| BSD-3-Clause | 1 | source-map-js |

## Docker images

The images (`docker/Dockerfile`, ghcr.io/av/rummage) add separate programs and libraries from
Ubuntu's, AMD's and NVIDIA's package repositories. rummage does not link any of them except the
GPU libraries listed for each image, which are permissive or redistributable. Each package's
licence is in `/usr/share/doc/<package>/copyright` in the image; rummage's own LICENSE, NOTICE
and THIRD-PARTY-NOTICES are in `/usr/share/doc/rummage/`.

| Image | Component | Licence | Compatible |
|---|---|---|---|
| all | Ubuntu 24.04 base | per package, mostly GPL/LGPL/MIT/BSD | yes, aggregation; source via `apt-get source` or https://launchpad.net/ubuntu |
| all | ffmpeg 6.1.1 (Ubuntu `ffmpeg`, configured with `--enable-gpl`, libx264, libx265; no `--enable-nonfree`) | GPL-2.0-or-later | yes, aggregation: rummage runs `ffmpeg`/`ffprobe` as separate processes and does not link libav*. Source: https://launchpad.net/ubuntu/+source/ffmpeg |
| all | poppler-utils (`pdftotext`) | GPL-2.0 or GPL-3.0 | yes, aggregation: run as a separate process. Source: https://launchpad.net/ubuntu/+source/poppler |
| vulkan | Vulkan loader (`libvulkan1`) | Apache-2.0 | yes |
| vulkan | Mesa Vulkan drivers (`mesa-vulkan-drivers`) | MIT | yes |
| rocm | HIP runtime (`hip-runtime-amd`), hipBLAS, rocBLAS, hipBLASLt, from repo.radeon.com | MIT (with LLVM's Apache-2.0 WITH LLVM-exception for the compiler support library) | yes |
| cuda | `nvidia/cuda:12.9.2-base-ubuntu24.04` base and cuBLAS 12 (`libcublas-12-9`) | NVIDIA CUDA Toolkit EULA; cuBLAS and the CUDA runtime are redistributable under its Attachment A | yes, with the EULA's conditions; pulling the image means accepting NVIDIA's EULA (stated in THIRD-PARTY-NOTICES) |

The release archives ship none of these: only the binary, README, LICENSE, NOTICE and
THIRD-PARTY-NOTICES.

## Not distributed

- The EmbeddingGemma 2 model is downloaded on first use, not shipped. It is under Apache 2.0, and
  the Gemma prohibited use policy applies.
- The fixtures in `crates/rummage-embed/tests/reference/fixtures` have their own sources and
  licences in that folder's LICENSES.md.

## Updating this page

```sh
cargo tree --workspace -e normal --target x86_64-unknown-linux-gnu --all-features --locked \
    -f '{p}|{l}' --prefix none | sort -u
```

gives the crate table; for the UI, the `license` field of each package.json under
`crates/rummage/ui/node_modules` after `bun install`.
