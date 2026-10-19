import type { Kind } from './api';

/** Everything the URL holds: `?q=&type=&folder=&date=&from=&to=&in=&like=&item=`. */
export type UrlState = {
  q: string;
  type: Kind | '';
  folder: string;
  /** `week`, `month`, `year`, `custom` (with from/to) or empty. */
  date: string;
  from: string;
  to: string;
  /** Only files under this directory. */
  dir: string;
  /** "More like this" item id. */
  like: number | null;
  /** The item open in the preview. */
  item: number | null;
};

const KINDS = ['image', 'video', 'audio', 'doc', 'text'];

export function empty(): UrlState {
  return { q: '', type: '', folder: '', date: '', from: '', to: '', dir: '', like: null, item: null };
}

function int(v: string | null): number | null {
  return v && /^\d+$/.test(v) ? Number(v) : null;
}

export function parse(search: string): UrlState {
  const p = new URLSearchParams(search);
  const type = p.get('type') ?? '';
  const from = p.get('from') ?? '';
  const to = p.get('to') ?? '';
  return {
    q: p.get('q') ?? '',
    type: (KINDS.includes(type) ? type : '') as Kind | '',
    folder: p.get('folder') ?? '',
    date: p.get('date') ?? (from || to ? 'custom' : ''),
    from,
    to,
    dir: p.get('in') ?? '',
    like: int(p.get('like')),
    item: int(p.get('item')),
  };
}

export function serialize(s: UrlState): string {
  const p = new URLSearchParams();
  if (s.q) p.set('q', s.q);
  if (s.like != null) p.set('like', String(s.like));
  if (s.type) p.set('type', s.type);
  if (s.folder) p.set('folder', s.folder);
  if (s.date && s.date !== 'custom') p.set('date', s.date);
  if (s.date === 'custom' && s.from) p.set('from', s.from);
  if (s.date === 'custom' && s.to) p.set('to', s.to);
  if (s.dir) p.set('in', s.dir);
  if (s.item != null) p.set('item', String(s.item));
  const out = p.toString();
  return out ? `?${out}` : location.pathname;
}

/** The search part only (no open item), to tell whether results must be re-fetched. */
export function searchKey(s: UrlState): string {
  return serialize({ ...s, item: null });
}

/** The date filter as Unix seconds. */
export function dateRange(s: UrlState, now = Date.now()): { date_from?: number; date_to?: number } {
  const day = 86400;
  const t = Math.floor(now / 1000);
  switch (s.date) {
    case 'week':
      return { date_from: t - 7 * day };
    case 'month':
      return { date_from: t - 31 * day };
    case 'year':
      return { date_from: t - 365 * day };
    case 'custom': {
      const at = (d: string) => (d ? Math.floor(new Date(`${d}T00:00:00`).getTime() / 1000) : undefined);
      const to = at(s.to);
      return { date_from: at(s.from), date_to: to === undefined ? undefined : to + day };
    }
    default:
      return {};
  }
}
