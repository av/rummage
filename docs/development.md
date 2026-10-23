# Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# speed checks (search over 300k vectors, first-pass throughput), on the target machine:
cargo test --release -p rummage --test bench -- --ignored --nocapture
```

## Web UI

The page is a Svelte 5 + Vite app in `crates/rummage/ui`, using only the public API. `cargo build`
builds it with [bun](https://bun.sh) (see `crates/rummage/build.rs`) and embeds it in the binary.
Without bun the binary gets a placeholder page; `RUMMAGE_UI_DIST=<dir>` embeds a UI built
elsewhere, and `RUMMAGE_SKIP_UI=1` skips it.

```sh
cd crates/rummage/ui
bun install
bun run build && bun run mock      # a mock API with 10k items at http://127.0.0.1:4173/search/
RUMMAGE_API=http://127.0.0.1:4173 bun run dev   # Vite dev server against the mock (or a real server)
bun run check                      # svelte-check and TypeScript
bunx playwright install chromium
bunx playwright test               # end-to-end tests against mock servers
RUMMAGE_BIN=../../../target/debug/rummage bunx playwright test   # plus the real binary
bun run screenshots                # screenshots of the mock UI into docs/screenshots, for checking UI changes
SPEC=openapi.json bun run gen:api  # refresh src/lib/api-types.ts from /api/openapi.json
```

The README's screenshots and demo loop come from the real binary and a demo library of public-domain
and CC0 files; [screenshots/README.md](screenshots/README.md) lists the files and how they were made.

## CI and releases

CI (`.github/workflows/ci.yml`) runs on a self-hosted GitHub Actions runner on fedora (one job
at a time; each job waits while the machine's load is above 70% of its threads). It runs those
checks and the web UI's checks and Playwright tests, then builds, tests and packs all four
backends (`build.yml`), each in its toolchain stage of `docker/Dockerfile` on Ubuntu 24.04:
NVIDIA's CUDA 12.9 devel image, AMD's ROCm 7.2 dev image, and plain Ubuntu for CPU and Vulkan.
The Vulkan and ROCm tests, parity test included, run on fedora's Radeon GPU; the CUDA build has
no GPU there and runs them on its CPU fallback. Cargo target dirs, ccache and the bun cache stay
under `/mnt/models/rummage/ci` between runs. `docker/in-toolchain.sh <image> <command>` runs the
same builds by hand. Pushing a `v*` tag runs `release.yml`: the four tarballs and `SHA256SUMS`
go to a GitHub pre-release, and the images to ghcr.io.

Crates: `crates/rummage` (the binary: CLI, server, embedded UI), `crates/rummage-core`
(config, SQLite store and schema, search, API types, the `Embedder` trait and a fake embedder
for tests) and `crates/rummage-embed` (the llama.cpp engine).
