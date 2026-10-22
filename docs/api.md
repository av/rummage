# HTTP API

Everything the UI does goes through `/api/v1`. The OpenAPI description is at
`/api/openapi.json`, with a readable page at `/api/docs`.

| Route | What it does |
|---|---|
| `GET /api/v1/health` | Liveness, version, embedder (never needs a token) |
| `GET /api/v1/folders`, `POST /api/v1/folders` | List, add folders |
| `GET\|PATCH\|DELETE /api/v1/folders/{id}` | One folder: read, change globs or `enabled`, remove |
| `GET /api/v1/fs/dirs` | Sub-directories of a server directory (`?path=`, `?hidden=true`), for picking folders |
| `GET /api/v1/index/status` | Totals, per kind, per folder, queue |
| `GET /api/v1/index/events` | Live progress (server-sent events) |
| `GET /api/v1/index/problems` | Files that failed or were skipped, with the reason (`?limit=&offset=`) |
| `POST /api/v1/index/scan` | Look for new, changed, moved and deleted files now (`{"folder":…}` for one) |
| `POST /api/v1/index/pause`, `POST /api/v1/index/resume` | Pause or resume indexing; searches keep working |
| `POST /api/v1/reindex` | `{"scope":"all"}`, `{"scope":"folder","id":…}`, `{"scope":"item","id":…}` or `{"scope":"problems"}` |
| `POST /api/v1/search` | By `query`, `item_id` or uploaded `file` (JSON base64, or multipart), with `filters` (types, folders, date range, path glob), `mode`, `limit`, `offset` |
| `GET /api/v1/items/{id}` | Metadata and segments (`?q=` ranks them) |
| `GET /api/v1/items/{id}/thumb` | WebP thumbnail (`?size=`; `?t=` picks a video window's poster) |
| `GET /api/v1/items/{id}/preview` | A document page (`?page=`) or a video frame (`?t=`), WebP |
| `GET /api/v1/items/{id}/raw` | The file, with range requests for audio/video seeking |
| `GET /api/v1/items/{id}/play` | A video in a form browsers play: for one with `urls.play`, its H.264 copy (202 with `{"state","progress"}` until it is made), else the file; ranges as for `raw` |
| `GET /api/v1/items/{id}/similar` | More like this (`?type=image,video&limit=`) |
| `GET /api/v1/items/{id}/waveform` | Peak levels of an audio or video file (`?points=`), for drawing a waveform |
| `GET\|PATCH /api/v1/settings` | The config keys; PATCH takes a JSON merge patch |

Errors are `{"error": "<code>", "message": "…"}` with codes `bad_request`, `unauthorized`,
`not_found`, `conflict`, `unsupported`, `not_ready` and `internal`.
