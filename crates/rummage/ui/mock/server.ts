// A mock of rummage's /api/v1 for UI development and the Playwright tests. It serves the built
// UI (../dist) the way `rummage serve` does: at the root and under BASE (default /search),
// with a <base href> injected into index.html.
//
//   PORT=4173 MOCK_ITEMS=10000 bun mock/server.ts
//   MOCK_EMPTY=1      no folders, no items (first run)
//   MOCK_TOKEN=xyz    require a bearer token
//   MOCK_LATENCY=50   delay search responses (ms)
//   MOCK_FILES_URL=…  initial ui.files_url
//   MOCK_MESSAGES=…   status messages, separated by `|`
import { join } from 'node:path';
import type { components } from '../src/lib/api-types';
import { FOLDERS, HOME, makeItems, envelope, type Kind, type MockItem, type MockSegment } from './data';
import { pageSvg, pdf, sceneSvg, wav } from './art';

type S = components['schemas'];

const PORT = Number(process.env.PORT ?? 4173);
const BASE = process.env.BASE ?? '/search';
const EMPTY = !!process.env.MOCK_EMPTY;
const TOKEN = process.env.MOCK_TOKEN ?? '';
const LATENCY = Number(process.env.MOCK_LATENCY ?? 0);
const DIST = process.env.DIST ?? join(import.meta.dir, '..', 'dist');
const VIDEO = join(import.meta.dir, 'fixtures', 'clip.webm');
const started = Date.now();

const items: MockItem[] = EMPTY ? [] : makeItems(Number(process.env.MOCK_ITEMS ?? 10000));
const byId = new Map(items.map((it) => [it.id, it]));
const words = (s: string) => new Set(s.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean));
const lower = new Map(items.map((it) => [it.id, { words: words(`${it.name} ${it.text}`), tags: it.tags }]));

let filesUrl = process.env.MOCK_FILES_URL ?? '/files/files{path}';
const messages = (process.env.MOCK_MESSAGES ?? '').split('|').filter(Boolean);
let paused = false;

type FolderState = { path: string; exclude: string[]; enabled: boolean; files: number; indexed: number; errors: number; skipped: number; scanning: boolean };
const folders: FolderState[] = EMPTY
  ? []
  : FOLDERS.map((path) => {
      const files = items.filter((it) => it.folder === path).length;
      return { path, exclude: [], enabled: true, files, indexed: files, errors: path.endsWith('Music') ? 2 : 0, skipped: path.endsWith('Videos') ? 1 : 0, scanning: false };
    });

/** Files that failed or were skipped (not searchable, so not among `items`). */
const PROBLEMS: { folder: string; name: string; kind: Kind; status: 'error' | 'skipped'; error: string }[] = EMPTY
  ? []
  : [
      { folder: `${HOME}/Music`, name: 'Live at the Roundhouse.flac', kind: 'audio', status: 'error', error: 'decoding audio: invalid FLAC stream header at byte 0' },
      { folder: `${HOME}/Music`, name: 'Voice memo 14.m4a', kind: 'audio', status: 'error', error: 'reading the file: unexpected end of file (it may still be copying)' },
      { folder: `${HOME}/Videos`, name: 'Screen recording.mkv', kind: 'video', status: 'skipped', error: "the embedding model can't read this kind of content" },
    ];

function fid(path: string) {
  let h = 0xcbf29ce484222325n;
  for (const c of new TextEncoder().encode(path)) h = BigInt.asUintN(64, (h ^ BigInt(c)) * 0x100000001b3n);
  return (h >> 16n).toString(16).padStart(12, '0').slice(-12);
}

function counts(f: FolderState): S['IndexCounts'] {
  return { files: f.files, indexed: f.indexed, pending: f.files - f.indexed, errors: f.errors, skipped: f.skipped, bytes: f.files * 3_000_000, chunks: f.indexed * 3, vectors: f.indexed * 3 };
}

const RATE = 600; // files per minute while indexing
const eta = (pending: number) => (pending > 0 && !paused ? Math.ceil((pending / RATE) * 60) : null);

function folderView(f: FolderState): S['Folder'] {
  return { id: fid(f.path), path: f.path, include: [], exclude: f.exclude, enabled: f.enabled, available: true, from_env: false, counts: counts(f) };
}

function status(): S['IndexStatus'] {
  const totals = folders.reduce(
    (a, f) => {
      const c = counts(f);
      for (const k of Object.keys(a) as (keyof S['IndexCounts'])[]) a[k] += c[k] ?? 0;
      return a;
    },
    { files: 0, indexed: 0, pending: 0, errors: 0, skipped: 0, bytes: 0, chunks: 0, vectors: 0 } as S['IndexCounts'],
  );
  const active = folders.filter((f) => f.enabled && f.indexed < f.files);
  const pending = active.reduce((a, f) => a + f.files - f.indexed, 0);
  const kinds = new Map<Kind, number>();
  for (const it of items) kinds.set(it.kind, (kinds.get(it.kind) ?? 0) + 1);
  const working = active.length > 0 && !paused;
  return {
    state: paused ? 'paused' : folders.some((f) => f.scanning) ? 'scanning' : working ? 'indexing' : 'idle',
    totals,
    by_kind: [...kinds].map(([kind, files]) => ({ kind, files })),
    folders: folders.map((f) => ({ id: fid(f.path), path: f.path, counts: counts(f), scanning: f.scanning && !paused, eta_secs: f.enabled ? eta(f.files - f.indexed) : null })),
    current: working ? `${active[0].path}/IMG_${active[0].indexed}.jpg` : null,
    queued: pending,
    files_per_min: working ? RATE : null,
    eta_secs: eta(pending),
    messages,
  };
}

// ---- live progress ----

const listeners = new Set<(e: string) => void>();
function publish() {
  const msg = `event: status\ndata: ${JSON.stringify({ type: 'status', ...status() })}\n\n`;
  for (const l of listeners) l(msg);
}
setInterval(() => {
  if (paused) return;
  let moved = false;
  for (const f of folders) {
    if (f.scanning) {
      // A new folder is walked for a moment before its files are indexed.
      f.scanning = false;
      moved = true;
    } else if (f.enabled && f.indexed < f.files) {
      f.indexed = Math.min(f.files, f.indexed + Math.max(1, Math.ceil(f.files / 40)));
      moved = true;
    }
  }
  if (moved) publish();
}, 1000);

// ---- search ----

function tokens(q: string) {
  return [...new Set(q.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter((t) => t.length >= 2))];
}

function hash(s: string) {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return (h >>> 0) / 4294967296;
}

function textScore(it: MockItem, terms: string[]) {
  const l = lower.get(it.id)!;
  let score = 0;
  let keyword = false;
  for (const t of terms) {
    if (l.tags.some((g) => g === t || (t.length >= 3 && g.startsWith(t)))) score += 2;
    if (l.words.has(t) || (t.length >= 3 && [...l.words].some((w) => w.startsWith(t)))) {
      score += 1;
      keyword = true;
    }
  }
  return { score, keyword };
}

/** Like the server's: about `max` characters around the densest run of query words, in pieces. */
function snippet(text: string, words: string[], max = 300): S['TextSpan'][] {
  const matches: [number, number][] = [];
  for (const m of text.matchAll(/[\p{L}\p{N}]+/gu)) {
    const w = m[0].toLowerCase();
    if (words.some((q) => w.startsWith(q))) matches.push([m.index, m.index + m[0].length]);
  }
  let from = 0;
  let best = -1;
  matches.forEach(([s], i) => {
    const n = matches.slice(i).filter(([, e]) => e <= s + (max * 3) / 4).length;
    if (n > best) [best, from] = [n, s];
  });
  if (from > max / 5) {
    const lead = from - max / 5;
    const sp = text.slice(lead, from).search(/\s/);
    from = sp >= 0 ? lead + sp + 1 : from;
  } else from = 0;
  const to = Math.min(text.length, from + max);
  const out: S['TextSpan'][] = [];
  const push = (t: string, hit: boolean) => t && out.push({ text: t, hit });
  if (from > 0) push('…', false);
  let pos = from;
  for (const [s, e] of matches.filter(([s, e]) => s >= from && e <= to)) {
    push(text.slice(pos, s), false);
    push(text.slice(s, e), true);
    pos = e;
  }
  push(text.slice(pos, to), false);
  if (to < text.length) push('…', false);
  return out;
}

function bestSegment(it: MockItem, terms: string[], sim: number): MockSegment | null {
  if (!it.segments.length) return null;
  let best = it.segments[0];
  if (it.kind === 'doc' || it.kind === 'text') {
    let top = -1;
    for (const s of it.segments) {
      const t = (s.text ?? '').toLowerCase();
      const n = terms.filter((w) => t.includes(w)).length;
      if (n > top) [top, best] = [n, s];
    }
  } else if (it.segments.length > 1) {
    best = it.segments[Math.floor(hash(terms.join(' ') + it.id) * it.segments.length)];
  }
  if (!best.text) return { ...best, score: sim };
  const spans = snippet(best.text, terms);
  return { ...best, text: spans.map((s) => s.text).join(''), snippet: spans, score: sim };
}

function itemView(it: MockItem, prefix: string): S['Item'] {
  const base = `${prefix}/api/v1/items/${it.id}`;
  return {
    id: it.id,
    path: it.path,
    name: it.name,
    folder_id: fid(it.folder),
    kind: it.kind,
    mime: it.mime,
    size: it.size,
    mtime: it.mtime,
    date: it.date,
    status: 'indexed',
    error: null,
    exif: it.exif as S['Item']['exif'],
    urls: { thumb: `${base}/thumb`, preview: `${base}/preview`, raw: `${base}/raw` },
  };
}

function globRe(g: string) {
  let re = '';
  for (let i = 0; i < g.length; i++) {
    const c = g[i];
    if (c === '\\' && i + 1 < g.length) re += g[++i].replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    else if (c === '*' && g[i + 1] === '*') {
      re += '.*';
      i++;
    } else if (c === '*') re += '[^/]*';
    else if (c === '?') re += '[^/]';
    else re += c.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }
  return new RegExp(`^${re}$`);
}

type Scored = { it: MockItem; score: number; sim: number; keyword: boolean };

function search(req: S['SearchRequest'], prefix: string, upload?: { name: string; type: string; text?: string; size: number }): S['SearchResponse'] | Response {
  const t0 = performance.now();
  const f = req.filters ?? ({} as S['Filters']);
  let scored: Scored[] = [];
  let terms: string[] = [];
  if (upload) {
    if (upload.type.startsWith('image/')) {
      const scene = items[Math.floor(hash(upload.name + upload.size) * items.length)]?.scene ?? 'sunset';
      scored = items.filter((it) => it.scene === scene && (it.kind === 'image' || it.kind === 'video')).map((it) => ({ it, score: 1, sim: 0.5 + hash(`${it.id}`) * 0.35, keyword: false }));
    } else if (upload.text != null) {
      terms = tokens(upload.text).slice(0, 40);
    } else {
      return json({ error: 'unsupported', message: `searching by ${upload.type || 'this kind of'} files is not supported yet; use an image or text file` }, 415);
    }
  } else if (req.item_id != null) {
    const src = byId.get(req.item_id);
    if (!src) return json({ error: 'not_found', message: `no item ${req.item_id}` }, 404);
    for (const it of items) {
      if (it.id === src.id) continue;
      const shared = it.tags.filter((t) => src.tags.includes(t)).length + (it.scene === src.scene && (it.kind === 'image' || it.kind === 'video') ? 1 : 0);
      if (shared) scored.push({ it, score: shared, sim: Math.min(0.95, 0.4 + shared * 0.08 + hash(`${it.id}`) * 0.05), keyword: false });
    }
  } else {
    const q = (req.query ?? '').trim();
    if (!q) return json({ error: 'bad_request', message: 'give exactly one of query, item_id or file' }, 400);
    terms = tokens(q);
  }
  if (terms.length) {
    for (const it of items) {
      const { score, keyword } = textScore(it, terms);
      if (score > 0) scored.push({ it, score, sim: Math.min(0.92, 0.3 + 0.05 * score + hash(it.id + terms.join()) * 0.25), keyword });
    }
  }
  const folderPaths = (f.folders ?? []).map((id) => folders.find((x) => fid(x.path) === id)?.path).filter(Boolean);
  const re = f.path_glob ? globRe(f.path_glob) : null;
  const pass = scored.filter(
    ({ it }) =>
      (!folderPaths.length || folderPaths.includes(it.folder)) &&
      (f.date_from == null || it.date >= f.date_from) &&
      (f.date_to == null || it.date < f.date_to) &&
      (!re || re.test(it.path)),
  );
  const kinds = new Map<Kind, number>();
  for (const s of pass) kinds.set(s.it.kind, (kinds.get(s.it.kind) ?? 0) + 1);
  const typed = f.types?.length ? pass.filter((s) => f.types.includes(s.it.kind)) : pass;
  typed.sort((a, b) => b.sim - a.sim || a.it.id - b.it.id);
  const limit = Math.min(500, Math.max(1, req.limit ?? 50));
  const offset = req.offset ?? 0;
  const page = typed.slice(offset, offset + limit);
  return {
    hits: page.map((s) => ({
      item: itemView(s.it, prefix),
      score: s.sim,
      similarity: s.sim,
      keyword_match: s.keyword,
      segment: bestSegment(s.it, terms, s.sim),
    })),
    total: typed.length,
    offset,
    limit,
    took_ms: performance.now() - t0,
    kinds: (['image', 'video', 'audio', 'text', 'doc'] as Kind[]).filter((k) => kinds.has(k)).map((kind) => ({ kind, files: kinds.get(kind)! })),
  };
}

function detail(it: MockItem, q: string, prefix: string): S['ItemDetail'] {
  const terms = tokens(q);
  let segments = it.segments.map((s) => ({ ...s }));
  if (terms.length) {
    const best = bestSegment(it, terms, 0.8);
    segments = segments.map((s) => ({
      ...s,
      score: s.id === best?.id ? 0.8 : Math.max(0.1, 0.55 - hash(`${s.id}${q}`) * 0.4 + ((s.text ?? '').toLowerCase().includes(terms[0]) ? 0.2 : 0)),
    }));
    segments.sort((a, b) => (b.score ?? 0) - (a.score ?? 0));
  }
  return { item: itemView(it, prefix), segments };
}

// ---- fake directory tree ----

const TREE: Record<string, string[]> = {
  '/': ['home', 'mnt', 'srv'],
  '/home': ['me'],
  [HOME]: ['.cache', '.config', 'Desktop', 'Documents', 'Downloads', 'Music', 'Pictures', 'Videos', 'code', 'notes'],
  [`${HOME}/Pictures`]: ['2021', '2022', '2023', '2024', '2025', '2026', 'Screenshots'],
  [`${HOME}/code`]: ['notes', 'rummage'],
  '/mnt': ['models', 'photos-archive'],
};

function dirs(path: string | null, hidden: boolean): S['DirListing'] | Response {
  const p = (path || HOME).replace(/^~/, HOME).replace(/\/+$/, '') || '/';
  if (!p.startsWith('/')) return json({ error: 'bad_request', message: `path must be absolute: ${p}` }, 400);
  const names = (TREE[p] ?? []).filter((n) => hidden || !n.startsWith('.'));
  const watched = new Set(folders.map((f) => f.path));
  const parent = p === '/' ? null : p.slice(0, p.lastIndexOf('/')) || '/';
  return { path: p, parent, dirs: names.map((name) => ({ name, path: `${p === '/' ? '' : p}/${name}`, watched: watched.has(`${p === '/' ? '' : p}/${name}`) })) };
}

// ---- settings ----

function settings(): S['SettingsView'] {
  return {
    settings: {
      data_dir: '~/.local/share/rummage',
      server: { bind: `127.0.0.1:${PORT}`, base_path: BASE, auth_token: TOKEN ? '********' : '' },
      folders: folders.map((f) => ({ path: f.path, include: [], exclude: f.exclude, enabled: f.enabled })),
      model: { repo: 'ggml-org/embeddinggemma-2-GGUF', quant: 'Q8_0', file: '', mmproj: '', dir: '', dim: 768, image_tokens: 280 },
      engine: { backend: 'auto', device: 'auto', threads: 0 },
      index: { concurrency: 0, scan_interval_secs: 3600, thumbnail_size: 320, skip: ['node_modules', '.git'] },
      ui: { files_url: filesUrl },
    },
    config_path: `${HOME}/.config/rummage/config.toml`,
    env_overrides: [],
    restart_required: [],
  };
}

// ---- HTTP ----

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const svg = (s: string) => new Response(s, { headers: { 'content-type': 'image/svg+xml', 'cache-control': 'max-age=3600' } });

async function ranged(req: Request, data: Blob, type: string) {
  const size = data.size;
  const m = /bytes=(\d*)-(\d*)/.exec(req.headers.get('range') ?? '');
  const headers: Record<string, string> = { 'content-type': type, 'accept-ranges': 'bytes' };
  if (!m) return new Response(data, { headers: { ...headers, 'content-length': String(size) } });
  const start = m[1] ? Number(m[1]) : size - Number(m[2]);
  const end = m[1] && m[2] ? Math.min(size - 1, Number(m[2])) : size - 1;
  return new Response(data.slice(start, end + 1), {
    status: 206,
    headers: { ...headers, 'content-range': `bytes ${start}-${end}/${size}`, 'content-length': String(end - start + 1) },
  });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function api(req: Request, url: URL, path: string, prefix: string): Promise<Response> {
  const m = req.method;
  if (path !== '/api/v1/health' && TOKEN) {
    const given = req.headers.get('authorization')?.replace(/^Bearer /, '') ?? url.searchParams.get('access_token');
    if (given !== TOKEN) return json({ error: 'unauthorized', message: 'missing or wrong bearer token' }, 401);
  }
  if (path === '/api/v1/health')
    return json({ status: 'ok', version: 'mock', schema_version: 1, embedder: { name: 'mock', dim: 768, modalities: { text: true, image: true, audio: true, video: true } }, uptime_secs: Math.floor((Date.now() - started) / 1000), auth_required: !!TOKEN } satisfies S['Health']);
  if (path === '/api/v1/index/status') return json(status());
  if ((path === '/api/v1/index/pause' || path === '/api/v1/index/resume') && m === 'POST') {
    paused = path.endsWith('pause');
    publish();
    return json(status());
  }
  if (path === '/api/v1/index/problems') {
    const limit = Math.min(500, Math.max(1, Number(url.searchParams.get('limit') ?? 50)));
    const list = PROBLEMS.filter((p) => folders.some((f) => f.path === p.folder));
    return json({
      total: list.length,
      items: list.slice(0, limit).map((p, i) => {
        const base = `${prefix}/api/v1/items/${900000 + i}`;
        const t = Math.floor(started / 1000) - i * 3600;
        return { id: 900000 + i, path: `${p.folder}/${p.name}`, name: p.name, folder_id: fid(p.folder), kind: p.kind, mime: null, size: 0, mtime: t, date: t, status: p.status, error: p.error, exif: null, urls: { thumb: `${base}/thumb`, preview: `${base}/preview`, raw: `${base}/raw` } };
      }),
    } satisfies S['ProblemList']);
  }
  if (path === '/api/v1/index/events') {
    let send: (e: string) => void = () => {};
    const stream = new ReadableStream({
      start(c) {
        const enc = new TextEncoder();
        send = (e) => c.enqueue(enc.encode(e));
        listeners.add(send);
        send(`event: status\ndata: ${JSON.stringify({ type: 'status', ...status() })}\n\n`);
      },
      cancel() {
        listeners.delete(send);
      },
    });
    req.signal.addEventListener('abort', () => listeners.delete(send));
    return new Response(stream, { headers: { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' } });
  }
  if (path === '/api/v1/folders' && m === 'GET') return json(folders.map(folderView));
  if (path === '/api/v1/folders' && m === 'POST') {
    const body = (await req.json()) as S['NewFolder'];
    const p = body.path.replace(/^~/, HOME).replace(/\/+$/, '');
    if (!p.startsWith('/')) return json({ error: 'bad_request', message: `folder path must be absolute: ${body.path}` }, 400);
    if (folders.some((f) => f.path === p)) return json({ error: 'conflict', message: `already watched: ${p}` }, 409);
    const f: FolderState = { path: p, exclude: body.exclude ?? [], enabled: true, files: 480 + Math.floor(hash(p) * 2000), indexed: 0, errors: 0, skipped: 0, scanning: true };
    folders.push(f);
    publish();
    return json(folderView(f), 201);
  }
  let mm = /^\/api\/v1\/folders\/([^/]+)$/.exec(path);
  if (mm) {
    const id = decodeURIComponent(mm[1]);
    const i = folders.findIndex((f) => fid(f.path) === id || f.path === id);
    if (i < 0) return json({ error: 'not_found', message: `no folder ${id}` }, 404);
    if (m === 'DELETE') {
      folders.splice(i, 1);
      publish();
      return new Response(null, { status: 204 });
    }
    if (m === 'PATCH') {
      const patch = (await req.json()) as S['FolderPatch'];
      if (patch.exclude) folders[i].exclude = patch.exclude;
      if (patch.enabled != null) folders[i].enabled = patch.enabled;
      publish();
    }
    return json(folderView(folders[i]));
  }
  if (path === '/api/v1/fs/dirs') {
    const r = dirs(url.searchParams.get('path'), url.searchParams.get('hidden') === 'true');
    return r instanceof Response ? r : json(r);
  }
  if (path === '/api/v1/reindex' && m === 'POST') {
    const body = (await req.json()) as S['ReindexRequest'];
    if (body.scope === 'problems') return json({ queued: PROBLEMS.length }, 202);
    let queued = 0;
    for (const f of folders) {
      if (body.scope === 'all' || (body.scope === 'folder' && fid(f.path) === body.id)) {
        const n = Math.ceil(f.files / 4);
        f.indexed = Math.max(0, f.files - n);
        queued += n;
      }
    }
    publish();
    return json({ queued }, 202);
  }
  if (path === '/api/v1/settings') {
    if (m === 'PATCH') {
      const patch = (await req.json()) as { ui?: { files_url?: string } };
      if (patch.ui?.files_url != null) filesUrl = patch.ui.files_url;
    }
    return json(settings());
  }
  if (path === '/api/v1/search' && m === 'POST') {
    if (LATENCY) await sleep(LATENCY);
    const type = req.headers.get('content-type') ?? '';
    let r: ReturnType<typeof search>;
    if (type.startsWith('multipart/form-data')) {
      const form = await req.formData();
      const file = form.get('file');
      if (!(file instanceof File)) return json({ error: 'bad_request', message: 'missing `file` part' }, 400);
      const request = JSON.parse(String(form.get('request') ?? '{}')) as S['SearchRequest'];
      const ftype = file.type || (file.name.endsWith('.md') || file.name.endsWith('.txt') ? 'text/plain' : '');
      r = search(request, prefix, { name: file.name, type: ftype, size: file.size, text: ftype.startsWith('text/') ? await file.text() : undefined });
    } else {
      r = search((await req.json()) as S['SearchRequest'], prefix);
    }
    return r instanceof Response ? r : json(r);
  }
  mm = /^\/api\/v1\/items\/(\d+)(\/[a-z]+)?$/.exec(path);
  if (mm) {
    const it = byId.get(Number(mm[1]));
    if (!it) return json({ error: 'not_found', message: `no item ${mm[1]}` }, 404);
    const sub = mm[2] ?? '';
    if (sub === '') return json(detail(it, url.searchParams.get('q') ?? '', prefix));
    if (sub === '/thumb') {
      if (it.kind === 'image') return svg(sceneSvg(it));
      if (it.kind === 'video') return svg(sceneSvg(it, 0));
      if (it.kind === 'doc') return svg(pageSvg(it, 1));
      return json({ error: 'not_found', message: `no thumbnail of item ${it.id}` }, 404);
    }
    if (sub === '/preview') {
      if (it.kind === 'image') return svg(sceneSvg(it));
      if (it.kind === 'video') {
        const t = Number(url.searchParams.get('t') ?? 0);
        return svg(sceneSvg(it, t));
      }
      if (it.kind === 'doc') {
        const page = Number(url.searchParams.get('page') ?? 1);
        if (page < 1 || page > it.pages) return json({ error: 'not_found', message: 'no such page' }, 404);
        return svg(pageSvg(it, page));
      }
      return json({ error: 'not_found', message: `no rendering of item ${it.id}` }, 404);
    }
    if (sub === '/raw') {
      const disp = { 'content-disposition': `inline; filename="${it.name}"` };
      if (it.kind === 'image') return new Response(sceneSvg(it), { headers: { 'content-type': 'image/svg+xml', ...disp } });
      if (it.kind === 'video') return ranged(req, Bun.file(VIDEO), 'video/webm');
      if (it.kind === 'audio') return ranged(req, new Blob([wav(it) as BlobPart]), 'audio/wav');
      if (it.kind === 'doc') return new Response(pdf(it) as BodyInit, { headers: { 'content-type': 'application/pdf', ...disp } });
      return new Response(it.text, { headers: { 'content-type': 'text/markdown; charset=utf-8', ...disp } });
    }
    if (sub === '/waveform') {
      if (it.kind !== 'audio' && it.kind !== 'video') return json({ error: 'unsupported', message: `item ${it.id} is not audio or video` }, 415);
      const n = Math.min(2000, Number(url.searchParams.get('points') ?? 200));
      const dur = it.duration;
      return json({ duration: dur, peaks: Array.from({ length: n }, (_, i) => envelope(it, ((i + 0.5) / n) * dur)) } satisfies S['Waveform']);
    }
    if (sub === '/similar') {
      const r = search({ item_id: it.id, limit: Number(url.searchParams.get('limit') ?? 50), offset: 0 } as S['SearchRequest'], prefix);
      return r instanceof Response ? r : json(r);
    }
  }
  return json({ error: 'not_found', message: `no route ${path}` }, 404);
}

async function ui(path: string, prefix: string) {
  const rel = path.replace(/^\/+/, '');
  if (rel && rel !== 'index.html' && !rel.includes('..')) {
    const f = Bun.file(join(DIST, rel));
    if (await f.exists()) return new Response(f);
  }
  const index = Bun.file(join(DIST, 'index.html'));
  if (!(await index.exists())) return new Response('build the UI first: bun run build', { status: 500 });
  const html = (await index.text()).replace('<head>', `<head>\n<base href="${prefix}/">`);
  return new Response(html, { headers: { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-cache' } });
}

Bun.serve({
  port: PORT,
  hostname: '127.0.0.1',
  idleTimeout: 0,
  async fetch(req) {
    const url = new URL(req.url);
    let path = url.pathname;
    let prefix = '';
    if (BASE && path === BASE) return Response.redirect(`${BASE}/${url.search}`, 308);
    if (BASE && path.startsWith(`${BASE}/`)) {
      path = path.slice(BASE.length);
      prefix = BASE;
    }
    if (path.startsWith('/api/')) return api(req, url, path, prefix);
    return ui(path, prefix);
  },
});

console.log(`rummage mock on http://127.0.0.1:${PORT}${BASE}/ (${items.length} items)`);
