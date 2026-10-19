// Deterministic fixture data for the mock server: folders, items and their segments.
import type { components } from '../src/lib/api-types';

type S = components['schemas'];
export type Kind = S['Kind'];

export type MockSegment = S['Segment'];
export type MockItem = {
  id: number;
  path: string;
  name: string;
  folder: string;
  kind: Kind;
  mime: string;
  size: number;
  mtime: number;
  date: number;
  exif: Record<string, unknown> | null;
  /** Words the mock matches queries against. */
  tags: string[];
  scene: string;
  hue: number;
  duration: number;
  pages: number;
  text: string;
  segments: MockSegment[];
};

export const HOME = '/home/me';
export const FOLDERS = [`${HOME}/Pictures`, `${HOME}/Videos`, `${HOME}/Music`, `${HOME}/Documents`, `${HOME}/notes`];

export function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const SCENES: Record<string, { tags: string[]; hue: number }> = {
  sunset: { tags: ['sunset', 'sky', 'orange', 'evening', 'sun'], hue: 20 },
  beach: { tags: ['beach', 'sea', 'sand', 'summer', 'waves', 'holiday'], hue: 195 },
  mountain: { tags: ['mountain', 'snow', 'hike', 'peak', 'alps'], hue: 215 },
  forest: { tags: ['forest', 'trees', 'green', 'walk', 'moss'], hue: 130 },
  city: { tags: ['city', 'street', 'night', 'lights', 'buildings'], hue: 260 },
  lake: { tags: ['lake', 'water', 'reflection', 'calm', 'boat'], hue: 185 },
  desert: { tags: ['desert', 'dunes', 'sand', 'heat'], hue: 38 },
  field: { tags: ['field', 'flowers', 'spring', 'meadow'], hue: 90 },
};
const SCENE_NAMES = Object.keys(SCENES);
const EXTRAS = ['bicycle', 'dog', 'friends', 'family', 'birthday', 'cat', 'car', 'train', 'coffee', 'garden'];

const TOPICS: { title: string; tags: string[]; lines: string[] }[] = [
  {
    title: 'Tax return 2024',
    tags: ['tax', 'invoice', 'receipt', 'return', 'deduction'],
    lines: [
      'The quarterly tax invoice for 2024 is attached; the payment is due by the end of the month.',
      'Deductions include the home office, the new laptop and two conference trips.',
      'Keep every receipt for five years in case the tax office asks for them.',
    ],
  },
  {
    title: 'Trip to the Alps',
    tags: ['trip', 'alps', 'mountain', 'hike', 'train', 'snow'],
    lines: [
      'We took the early train to Innsbruck and started the hike from the valley at nine.',
      'Snow on the upper trail made the last kilometre slow, but the view from the peak was worth it.',
      'Next time: book the hut in advance and bring proper gloves.',
    ],
  },
  {
    title: 'Garden plan',
    tags: ['garden', 'flowers', 'spring', 'tomatoes', 'seeds'],
    lines: [
      'Tomatoes go along the south fence where the sun lasts until evening.',
      'Order seeds for basil, chard and sunflowers before the end of February.',
      'The red bicycle leaning on the garden fence needs new tyres.',
    ],
  },
  {
    title: 'Recipe: lemon pasta',
    tags: ['recipe', 'pasta', 'lemon', 'dinner', 'cooking'],
    lines: [
      'Boil the pasta in well-salted water and keep a cup of the cooking water.',
      'Melt butter with lemon zest, add the pasta, then the parmesan and a splash of water.',
      'Finish with black pepper and more lemon juice to taste.',
    ],
  },
  {
    title: 'Meeting notes: search project',
    tags: ['meeting', 'search', 'project', 'embedding', 'index'],
    lines: [
      'The index should keep vectors and keyword matches in one SQLite file next to the data.',
      'Results must open at the exact page or moment that matched, not at the start of the file.',
      'Action: measure query time over three hundred thousand vectors on the home server.',
    ],
  },
  {
    title: 'Apartment lease',
    tags: ['lease', 'apartment', 'rent', 'contract', 'deposit'],
    lines: [
      'The lease runs for twelve months and renews unless either side gives notice.',
      'The deposit equals two months of rent and is returned within thirty days.',
      'Pets are allowed with the landlord’s written consent.',
    ],
  },
];

const SOUNDS: { name: string; tags: string[] }[] = [
  { name: 'Rain on the window', tags: ['rain', 'storm', 'ambient', 'weather'] },
  { name: 'Morning birds', tags: ['birds', 'morning', 'forest', 'nature'] },
  { name: 'Piano sketch', tags: ['piano', 'music', 'melody', 'sketch'] },
  { name: 'Kick drum loop', tags: ['drum', 'kick', 'beat', 'loop', 'sample'] },
  { name: 'Voice memo', tags: ['voice', 'memo', 'speech', 'idea'] },
  { name: 'Ocean waves', tags: ['ocean', 'waves', 'beach', 'sea'] },
  { name: 'Synth pad', tags: ['synth', 'pad', 'music', 'ambient'] },
];

const pick = <T>(r: () => number, a: T[]) => a[Math.floor(r() * a.length)];
const pad = (n: number, w = 2) => String(n).padStart(w, '0');

function stamp(r: () => number) {
  // Spread over the last six years.
  const now = Date.UTC(2026, 9, 18) / 1000;
  return Math.floor(now - r() * 6 * 365 * 86400);
}

function ymd(t: number) {
  const d = new Date(t * 1000);
  return `${d.getUTCFullYear()}${pad(d.getUTCMonth() + 1)}${pad(d.getUTCDate())}`;
}

export function makeItems(n: number): MockItem[] {
  const items: MockItem[] = [];
  let segId = 1;
  for (let i = 0; i < n; i++) {
    const id = i + 1;
    const r = rng(id * 7919);
    const roll = r();
    const kind: Kind = roll < 0.46 ? 'image' : roll < 0.56 ? 'video' : roll < 0.66 ? 'audio' : roll < 0.78 ? 'doc' : 'text';
    const date = stamp(r);
    const scene = pick(r, SCENE_NAMES);
    const extra = r() < 0.3 ? [pick(r, EXTRAS)] : [];
    const it: MockItem = {
      id,
      path: '',
      name: '',
      folder: '',
      kind,
      mime: '',
      size: 0,
      mtime: date + Math.floor(r() * 86400 * 30),
      date,
      exif: null,
      tags: [],
      scene,
      hue: SCENES[scene].hue,
      duration: 0,
      pages: 0,
      text: '',
      segments: [],
    };
    const year = new Date(date * 1000).getUTCFullYear();
    if (kind === 'image') {
      it.folder = FOLDERS[0];
      it.name = `IMG_${ymd(date)}_${pad(Math.floor(r() * 240000), 6)}.jpg`;
      it.path = `${it.folder}/${year}/${it.name}`;
      it.mime = 'image/jpeg';
      it.size = 2_000_000 + Math.floor(r() * 6_000_000);
      it.tags = [...SCENES[scene].tags, ...extra];
      it.exif = {
        Make: pick(r, ['FUJIFILM', 'Google', 'Apple', 'SONY']),
        Model: pick(r, ['X-T4', 'Pixel 8', 'iPhone 15', 'A7 III']),
        LensModel: pick(r, ['XF23mmF2 R WR', '24-70mm F2.8', 'Main camera']),
        FNumber: pick(r, [1.8, 2, 2.8, 4, 5.6, 8]),
        ExposureTime: pick(r, ['1/60', '1/125', '1/250', '1/1000']),
        ISO: pick(r, [100, 200, 400, 800, 1600]),
        FocalLength: pick(r, [23, 26, 35, 50]),
        ImageWidth: 6240,
        ImageLength: 4160,
        DateTimeOriginal: new Date(date * 1000).toISOString().replace('T', ' ').slice(0, 19),
        GPSLatitude: 46 + r() * 6,
        GPSLongitude: 7 + r() * 12,
      };
      it.segments = [{ id: segId++, kind: 'image', page: null, t_start: null, t_end: null, text: null, score: null }];
    } else if (kind === 'video') {
      it.folder = FOLDERS[1];
      it.name = `VID_${ymd(date)}_${pad(Math.floor(r() * 240000), 6)}.webm`;
      it.path = `${it.folder}/${year}/${it.name}`;
      it.mime = 'video/webm';
      it.size = 30_000_000 + Math.floor(r() * 200_000_000);
      it.tags = [...SCENES[scene].tags, ...extra, 'video'];
      it.duration = 20;
      for (let t = 0; t < it.duration; t += 4)
        it.segments.push({ id: segId++, kind: 'video', page: null, t_start: t, t_end: Math.min(it.duration, t + 4), text: null, score: null });
    } else if (kind === 'audio') {
      const s = pick(r, SOUNDS);
      it.folder = FOLDERS[2];
      it.name = `${s.name} ${pad(1 + Math.floor(r() * 99))}.wav`;
      it.path = `${it.folder}/${pick(r, ['samples', 'memos', 'field recordings'])}/${it.name}`;
      it.mime = 'audio/wav';
      it.duration = 30 + Math.floor(r() * 150);
      it.size = it.duration * 16000;
      it.tags = [...s.tags];
      for (let t = 0; t < it.duration; t += 30)
        it.segments.push({ id: segId++, kind: 'audio', page: null, t_start: t, t_end: Math.min(it.duration, t + 30), text: null, score: null });
    } else {
      const topic = pick(r, TOPICS);
      const isDoc = kind === 'doc';
      it.folder = isDoc ? FOLDERS[3] : FOLDERS[4];
      const slug = topic.title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/-$/, '');
      it.name = isDoc ? `${topic.title} (${pad(1 + (id % 97))}).pdf` : `${slug}-${pad(1 + (id % 97))}.md`;
      it.path = `${it.folder}/${isDoc ? year : pick(r, ['journal', 'projects', 'reference'])}/${it.name}`;
      it.mime = isDoc ? 'application/pdf' : 'text/markdown';
      it.tags = [...topic.tags];
      const paras: string[] = [];
      const count = 4 + Math.floor(r() * 6);
      for (let p = 0; p < count; p++) {
        const a = topic.lines[(p + id) % topic.lines.length];
        const b = topic.lines[(p + id + 1) % topic.lines.length];
        paras.push(p % 3 === 2 ? `${a} ${b}` : a);
      }
      it.text = `# ${topic.title}\n\n${paras.join('\n\n')}\n`;
      it.size = it.text.length * (isDoc ? 40 : 1);
      if (isDoc) {
        it.pages = paras.length;
        paras.forEach((t, p) => it.segments.push({ id: segId++, kind: 'page', page: p + 1, t_start: null, t_end: null, text: t, score: null }));
      } else {
        paras.forEach((t) => it.segments.push({ id: segId++, kind: 'text', page: null, t_start: null, t_end: null, text: t, score: null }));
      }
    }
    items.push(it);
  }
  return items;
}

/** A 0..1 loudness envelope of a mock audio item at time t (seconds). */
export function envelope(it: MockItem, t: number): number {
  const r = rng(it.id);
  const a = 2 + r() * 5;
  const b = 0.3 + r() * 1.2;
  const base = 0.35 + 0.25 * Math.sin(t / a) + 0.2 * Math.sin(t * b + it.id);
  const beat = Math.pow(Math.abs(Math.sin(t * Math.PI * (1 + (it.id % 3)))), 6) * 0.35;
  return Math.max(0.04, Math.min(1, base + beat));
}
