// The /api/v1 client. Every URL is relative to the page's <base href>, so the UI works at the
// root and under a base path alike.
import type { components } from './api-types';

type S = components['schemas'];
export type Kind = S['Kind'];
export type Item = S['Item'];
export type Segment = S['Segment'];
export type SearchHit = S['SearchHit'];
export type SearchRequest = S['SearchRequest'];
export type SearchResponse = S['SearchResponse'];
export type Filters = S['Filters'];
export type ItemDetail = S['ItemDetail'];
export type Folder = S['Folder'];
export type IndexStatus = S['IndexStatus'];
export type FolderProgress = S['FolderProgress'];
export type ProblemList = S['ProblemList'];
export type ReindexRequest = S['ReindexRequest'];
export type IndexEvent = S['IndexEvent'];
export type DirListing = S['DirListing'];
export type Waveform = S['Waveform'];
export type SettingsView = S['SettingsView'];
export type Health = S['Health'];

const TOKEN_KEY = 'rummage.token';

export function token(): string {
  try {
    return localStorage.getItem(TOKEN_KEY) ?? '';
  } catch {
    return '';
  }
}

export function setToken(t: string) {
  try {
    if (t) localStorage.setItem(TOKEN_KEY, t);
    else localStorage.removeItem(TOKEN_KEY);
  } catch {
    /* private mode */
  }
}

/** An error from the API, or from not reaching it (`code` is `network`). */
export class ApiError extends Error {
  constructor(
    public code: string,
    message: string,
    public status = 0,
  ) {
    super(message);
  }
}

export function isAbort(e: unknown): boolean {
  return e instanceof DOMException && e.name === 'AbortError';
}

function url(path: string): string {
  return new URL(path.replace(/^\//, ''), document.baseURI).toString();
}

/** A server-given media URL (thumb, preview, raw) with the token added when one is needed. */
export function media(u: string, params: Record<string, string | number | undefined> = {}): string {
  const out = new URL(u, document.baseURI);
  for (const [k, v] of Object.entries(params)) if (v !== undefined) out.searchParams.set(k, String(v));
  const t = token();
  if (t) out.searchParams.set('access_token', t);
  return out.toString();
}

async function call<T>(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const headers: Record<string, string> = {};
  const t = token();
  if (t) headers.Authorization = `Bearer ${t}`;
  let payload: BodyInit | undefined;
  if (body instanceof FormData) payload = body;
  else if (body !== undefined) {
    headers['Content-Type'] = 'application/json';
    payload = JSON.stringify(body);
  }
  let res: Response;
  try {
    res = await fetch(url(path), { method, headers, body: payload, signal });
  } catch (e) {
    if (isAbort(e)) throw e;
    throw new ApiError('network', "Can't reach the rummage server.");
  }
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  let data: unknown;
  try {
    data = text ? JSON.parse(text) : undefined;
  } catch {
    data = undefined;
  }
  if (!res.ok) {
    const err = data as { error?: string; message?: string } | undefined;
    throw new ApiError(err?.error ?? 'internal', err?.message ?? `${res.status} ${res.statusText}`, res.status);
  }
  return data as T;
}

const enc = encodeURIComponent;

export const api = {
  health: () => call<Health>('GET', 'api/v1/health'),
  search: (req: SearchRequest, signal?: AbortSignal) =>
    call<SearchResponse>('POST', 'api/v1/search', req, signal),
  searchFile: (file: File, req: SearchRequest, signal?: AbortSignal) => {
    const form = new FormData();
    form.append('request', JSON.stringify(req));
    form.append('file', file, file.name);
    return call<SearchResponse>('POST', 'api/v1/search', form, signal);
  },
  item: (id: number, q?: string, signal?: AbortSignal) =>
    call<ItemDetail>('GET', `api/v1/items/${id}${q ? `?q=${enc(q)}` : ''}`, undefined, signal),
  waveform: (id: number, points: number, signal?: AbortSignal) =>
    call<Waveform>('GET', `api/v1/items/${id}/waveform?points=${points}`, undefined, signal),
  status: () => call<IndexStatus>('GET', 'api/v1/index/status'),
  folders: () => call<Folder[]>('GET', 'api/v1/folders'),
  addFolder: (path: string, exclude: string[] = []) =>
    call<Folder>('POST', 'api/v1/folders', { path, exclude, include: [], enabled: true }),
  updateFolder: (id: string, patch: { enabled?: boolean; exclude?: string[]; include?: string[] }) =>
    call<Folder>('PATCH', `api/v1/folders/${enc(id)}`, patch),
  removeFolder: (id: string) => call<void>('DELETE', `api/v1/folders/${enc(id)}`),
  pause: () => call<IndexStatus>('POST', 'api/v1/index/pause'),
  resume: () => call<IndexStatus>('POST', 'api/v1/index/resume'),
  problems: (limit = 100) => call<ProblemList>('GET', `api/v1/index/problems?limit=${limit}`),
  reindex: (req: ReindexRequest) => call<S['ReindexResponse']>('POST', 'api/v1/reindex', req),
  dirs: (path?: string) => call<DirListing>('GET', `api/v1/fs/dirs${path ? `?path=${enc(path)}` : ''}`),
  settings: () => call<SettingsView>('GET', 'api/v1/settings'),
  patchSettings: (patch: unknown) => call<SettingsView>('PATCH', 'api/v1/settings', patch),
  /** Live index events; reconnects on its own. */
  events: (onEvent: (e: IndexEvent) => void): (() => void) => {
    const u = new URL(url('api/v1/index/events'));
    const t = token();
    if (t) u.searchParams.set('access_token', t);
    const es = new EventSource(u);
    const handle = (m: MessageEvent) => {
      try {
        onEvent(JSON.parse(m.data));
      } catch {
        /* ignore malformed events */
      }
    };
    es.addEventListener('status', handle);
    es.addEventListener('file', handle);
    return () => es.close();
  },
};
