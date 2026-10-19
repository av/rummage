import type { Kind, Segment } from './api';

const nf = new Intl.NumberFormat();
export const num = (n: number) => nf.format(n);

export function plural(n: number, one: string, many = `${one}s`) {
  return `${num(n)} ${n === 1 ? one : many}`;
}

/** 134 -> "2:14", 3725 -> "1:02:05". */
export function clock(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

const df = new Intl.DateTimeFormat(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
const dtf = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });
export const date = (unix: number) => df.format(new Date(unix * 1000));
export const dateTime = (unix: number) => dtf.format(new Date(unix * 1000));

/** Seconds -> "about 3 min", "about 2 h". */
export function eta(secs: number): string {
  if (secs < 60) return 'under a minute';
  if (secs < 3600) return `about ${Math.round(secs / 60)} min`;
  const h = secs / 3600;
  return `about ${h < 10 ? h.toFixed(1).replace(/\.0$/, '') : Math.round(h)} h`;
}

export const kindLabel: Record<Kind, string> = {
  image: 'Image',
  video: 'Video',
  audio: 'Audio',
  doc: 'Document',
  text: 'Text',
};

/** Where in the file a segment is: "p. 3", "2:14", "0:30–1:00". */
export function where(seg: Segment | null | undefined, kind: Kind): string {
  if (!seg) return '';
  if (seg.page != null && (kind === 'doc' || seg.kind === 'page')) return `p. ${seg.page}`;
  if (seg.t_start != null) {
    if (kind === 'audio' && seg.t_end != null) return `${clock(seg.t_start)}–${clock(seg.t_end)}`;
    return clock(seg.t_start);
  }
  return '';
}

export function dirname(path: string): string {
  const i = path.lastIndexOf('/');
  return i > 0 ? path.slice(0, i) : '/';
}

export function ext(name: string): string {
  const i = name.lastIndexOf('.');
  return i > 0 ? name.slice(i + 1).toLowerCase() : '';
}
