# Configuration

The config file is `~/.config/rummage/config.toml` (`$XDG_CONFIG_HOME` is honoured;
`--config FILE` or `RUMMAGE_CONFIG` picks another). It is optional: missing keys take the
defaults below. The same keys are readable and writable with `rummage config get|set` and
`GET|PATCH /api/v1/settings`. Edits keep the file's comments.

| Key | Default | Env override | Meaning |
|---|---|---|---|
| `data_dir` | `~/.local/share/rummage` | `RUMMAGE_DATA_DIR` | Index (`rummage.db`), thumbnails and, by default, model files |
| `server.bind` | `127.0.0.1:8099` | `RUMMAGE_BIND` | Listen address for `rummage serve` |
| `server.base_path` | `""` | `RUMMAGE_BASE_PATH` | URL prefix behind a proxy, e.g. `/search` |
| `server.auth_token` | `""` | `RUMMAGE_AUTH_TOKEN` | If set, the API needs `Authorization: Bearer <token>` (or `?access_token=`) |
| `folders` | `[]` | `RUMMAGE_FOLDERS` (`/a:/b`) | Folders to index (see below) |
| `model.repo` | `ggml-org/embeddinggemma-2-GGUF` | `RUMMAGE_MODEL_REPO` | Hugging Face repo with the GGUF files |
| `model.quant` | `Q8_0` | `RUMMAGE_MODEL_QUANT` | `Q8_0`, `BF16` or `F32`. Never float16: the model gives NaN or degraded vectors in it |
| `model.file` | `""` | `RUMMAGE_MODEL_FILE` | Text model file; empty means `embeddinggemma-2-<quant>.gguf` |
| `model.mmproj` | `""` | `RUMMAGE_MODEL_MMPROJ` | Vision/audio file; empty means `mmproj-embeddinggemma-2-<quant>.gguf` |
| `model.dir` | `""` | `RUMMAGE_MODEL_DIR` | Where model files live; empty means `<data_dir>/models` |
| `model.dim` | `768` | `RUMMAGE_DIM` | Vector length kept: 768, 512, 256 or 128 |
| `model.image_tokens` | `280` | `RUMMAGE_IMAGE_TOKENS` | Tokens per image, 70 to 1120 |
| `engine.backend` | `auto` | `RUMMAGE_BACKEND` | `auto`, `cpu`, `vulkan`, `rocm` or `cuda` |
| `engine.device` | `auto` | `RUMMAGE_DEVICE` | GPU index as listed by `rummage doctor` |
| `engine.threads` | `0` | `RUMMAGE_THREADS` | CPU threads; 0 is automatic |
| `index.concurrency` | `0` | `RUMMAGE_CONCURRENCY` | Files extracted in parallel; 0 is automatic |
| `index.scan_interval_secs` | `3600` | `RUMMAGE_SCAN_INTERVAL` | Full re-scan period (file watching covers the rest); 0 disables |
| `index.thumbnail_size` | `320` | `RUMMAGE_THUMB_SIZE` | Longest thumbnail edge in pixels |
| `index.skip` | `node_modules, .git, target, dist, build, .cache, __pycache__, .venv` | `RUMMAGE_SKIP` (comma list) | Directory names skipped everywhere |
| `ui.files_url` | `""` | `RUMMAGE_FILES_URL` | Link for "Open in Files" in the web UI, e.g. `/files/files{path}`; `{path}` is the file's absolute path, `{dir}` its folder. Empty hides it |
| `tools.ffmpeg` | `""` | `RUMMAGE_FFMPEG` | The ffmpeg program for video and HEIC/AVIF photos; ffprobe is taken from the same folder. Empty means `ffmpeg` and `ffprobe` on `PATH` |
| `playback.convert` | `true` | `RUMMAGE_PLAYBACK_CONVERT` | Make an H.264 copy of videos browsers often can't play (HEVC, for one), so the web page can show them (see [How it works](how-it-works.md#video-playback)) |
| `playback.prepare` | `true` | `RUMMAGE_PLAYBACK_PREPARE` | Also make those copies ahead of time in the background, at the lowest priority and one at a time, while the indexer is idle |
| `playback.cache_mb` | `4096` | `RUMMAGE_PLAYBACK_CACHE_MB` | Most disk space the copies may use, in MB (at least 64); the least recently played go first. The background stops at three quarters of it |

Each `[[folders]]` entry has `path`, `include` and `exclude` (globs relative to the folder;
an empty `include` means everything) and `enabled`:

```toml
[[folders]]
path = "/home/me/Pictures"
exclude = ["**/*.raw", "Screenshots/**"]

[[folders]]
path = "/home/me/notes"
include = ["**/*.md"]
enabled = true
```

Environment overrides change only the running program; they are never written to the file.
Keys under `data_dir`, `server`, `model`, `engine`, `tools` and `index.concurrency` need a restart of
`rummage serve`; folders and the other `index` keys apply live.
