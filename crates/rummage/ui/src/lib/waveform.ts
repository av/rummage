import { api, media, type Item } from './api';

export type Wave = { peaks: number[]; duration: number };

const cache = new Map<string, Promise<Wave | null>>();
/** Largest file decoded in the browser when the server has no waveform. */
const DECODE_MAX = 40 * 1024 * 1024;

/** The server's waveform for an item; with `decode`, falls back to decoding the file here. */
export function waveform(item: Item, points: number, decode = false): Promise<Wave | null> {
  const key = `${item.id}:${points}:${decode}`;
  let p = cache.get(key);
  if (!p) {
    p = api
      .waveform(item.id, points)
      .then((w) => ({ peaks: w.peaks, duration: w.duration }) as Wave | null)
      .catch(() => (decode && item.size <= DECODE_MAX ? decodeHere(item, points) : null));
    cache.set(key, p);
  }
  return p;
}

async function decodeHere(item: Item, points: number): Promise<Wave | null> {
  try {
    const res = await fetch(media(item.urls.raw));
    if (!res.ok) return null;
    const ctx = new OfflineAudioContext(1, 1, 8000);
    const buf = await ctx.decodeAudioData(await res.arrayBuffer());
    return { peaks: peaksOf(buf.getChannelData(0), points), duration: buf.duration };
  } catch {
    return null;
  }
}

export function peaksOf(data: Float32Array, points: number): number[] {
  const step = Math.max(1, Math.floor(data.length / points));
  const out: number[] = [];
  let max = 0;
  for (let i = 0; i < points; i++) {
    let peak = 0;
    const end = Math.min(data.length, (i + 1) * step);
    for (let j = i * step; j < end; j += 4) peak = Math.max(peak, Math.abs(data[j]));
    out.push(peak);
    max = Math.max(max, peak);
  }
  return max > 0 ? out.map((p) => p / max) : out;
}
