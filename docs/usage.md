# Using rummage

## The web page

`rummage serve` serves the search page at <http://127.0.0.1:8099/> (or under `--base-path`). Results come as you type; a click or Enter opens a preview at the part that matched.

- `/` jumps to the search box from anywhere; Enter opens the top result. Arrow keys move through
  results; in the preview, Left/Right go to the previous/next result and Esc closes it.
- Drop or paste an image, audio or text file anywhere to search by it; "More like this" does the
  same for any result.
- Filter by kind, folder and date. The URL holds the search (`?q=&type=&folder=&date=&in=&like=&item=`),
  so links, bookmarks and Back/Forward work.
- The folder button opens the folders drawer: add folders (paste a path or browse the server),
  exclude files, reindex, pause or resume indexing, watch each folder's progress and time left,
  and see which files could not be read and why. On first run it opens by itself.
- Light and dark themes, and a layout for phones ([screenshots](screenshots/README.md)).

## Command line

```
rummage serve [--bind ADDR] [--base-path PATH]     run the web UI and the API
rummage folders add <path> [--exclude GLOB]... [--include GLOB]...
rummage folders set <path|id> [--off | --on] [--exclude GLOB]... [--include GLOB]...
                                                   turn a folder off or on, or replace its globs
rummage folders rm <path|id>                       stop indexing; drops it from the index, not the disk
rummage folders ls [--json]
rummage index [--once | --watch]                   index now; --watch keeps following changes
rummage index --pause | --resume                   pause or resume a running server's indexing
rummage reindex [--folder PATH|ID | --item ID | --problems]
                                                   index again (everything by default)
rummage search <query...> | --similar ID | --file PATH
               [--type KIND,...] [--folder PATH|ID]... [--path GLOB]
               [--after YYYY-MM-DD] [--before YYYY-MM-DD] [--mode hybrid|vector|keyword]
               [--limit N] [--json]
rummage item <id> [--query Q] [--json]             one item and its passages, pages or time windows
rummage status [--json] [--errors]                 counts per kind and folder; --errors lists failures
rummage config get [key] | set <key> <value> | path
rummage doctor                                     check config, index, folders, GPU, ffmpeg, poppler
```

The CLI talks to a running server when it finds one (at `server.bind`, or `--server URL`) and
opens the index directly otherwise (`--local` forces that). Global options: `--config FILE`
(`RUMMAGE_CONFIG`), `--server URL` (`RUMMAGE_SERVER`), `--local`. `rummage <command> --help`
describes each one. Log detail follows `RUST_LOG`.

### Behind a reverse proxy

To serve under a sub-path (e.g. nginx `location /search/ { proxy_pass http://127.0.0.1:8099/; }`):

```sh
rummage serve --bind 0.0.0.0:8099 --base-path /search
```

rummage accepts requests with or without the prefix and honours `X-Forwarded-Prefix` when the
proxy strips it, so links in responses and the UI always carry the right prefix. Set
`server.auth_token` before exposing it beyond your machine.
