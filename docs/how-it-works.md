# How it works

## Supported files

Files are found by extension; everything else is ignored, as are symlinks, the directories in
`index.skip` (plus `.git`, `node_modules` and `__pycache__`), and anything matching a folder's
`exclude` globs.

| Kind | Files | What is searchable | Results point to |
|---|---|---|---|
| image | jpg, png, webp, gif, bmp, tiff; heic, heif, avif (with ffmpeg); RAW (its embedded preview) | the picture, upright; EXIF date, camera and place as keywords | |
| video | mp4, m4v, mkv, webm, mov, avi (with ffmpeg) | 1 frame per second in 10 s windows, plus the sound track | the moment |
| audio | wav, mp3, flac, ogg, m4a, aac | the sound, in 30 s windows overlapping by 5 s | the moment |
| doc | pdf | each page's text, and each page as a picture (first 300 pages; pictures need poppler) | the page |
| text | txt, md, mdx, code, csv, json, yaml, html, ... | passages of about 1,000 tokens; the title is the first heading, the HTML `<title>` or the name | the passage |

## Indexing

`rummage serve` indexes the enabled folders in the background and keeps following
them; `rummage index` does one pass. Changes are noticed through inotify (one watch per indexed
directory; events are batched for half a second) and by a full rescan every
`index.scan_interval_secs`. A file whose size and modification time are unchanged is left alone;
same size with a new time is compared by BLAKE3 hash. A file that appears where another vanished
with the same hash is a move: it keeps its vectors. A folder whose root is missing (an unmounted
disk) is skipped rather than emptied.

Work is queued by priority (searches first, then new and changed files, then the first pass and
re-indexing) and runs on `index.concurrency` workers. A file that fails is recorded with its
error (`rummage status --errors`, `GET /api/v1/index/problems`) and the rest carry on;
`rummage reindex --problems` tries them again. Text is split into passages of 4,000 characters
overlapping by 400; images are embedded upright (EXIF orientation), and their EXIF date, camera
and GPS go into the keyword index. PDFs are read with poppler when it is installed (page images
and text) and with a built-in reader otherwise (text only). A video codec the installed ffmpeg
can't decode (Fedora's `ffmpeg-free` has no HEVC) is skipped with that reason; point
`tools.ffmpeg` at a full build (e.g. a static one from
[BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds)) to index those.

## Search

The query is embedded (`task: search result | query: …`) and scored against every
stored vector exactly (all vectors are held in memory at `model.dim` length and updated as files
are indexed). An FTS5 keyword search runs over names, paths, text and EXIF (words are stemmed, so
"claps" finds `Clap_01.wav` and "invoices" finds "invoice"; names also match by their camelCase
parts, so "petalia" finds `SpiralPetalia.mp4`), and the two rankings are fused with
reciprocal-rank fusion. Results are grouped per file with the best-matching segment, so a video
hit carries the moment and a PDF hit the page; text hits carry a snippet with the matching words
marked. Thumbnails (WebP, `index.thumbnail_size`) are made while indexing; page and frame
previews are rendered on first request. Both live under `<data_dir>/thumbs` and
`<data_dir>/previews`.

## Video playback

Videos in a codec that browsers don't all play (anything but H.264, VP8, VP9
and AV1, as the indexer's ffprobe found it, or any AVI) get a playable copy: H.264 and AAC in a
fast-start MP4, at most 1280 pixels on the long edge and 60 frames a second, made with the
configured ffmpeg (x264, else OpenH264; CPU only) under `<data_dir>/playable`. The web page asks
for it when the browser can't play the original (HEVC phone videos in Chrome or Firefox on
Linux, for example), shows the progress while it is made (a minute of 1080p HEVC takes 7 to
11 s on a 16-core machine), then plays it from the matched moment; if it can't be made, it plays
the original and says when only the sound comes through. `GET /api/v1/items/{id}/raw` always
sends the original.
