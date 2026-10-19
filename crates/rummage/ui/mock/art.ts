// Generated media for the mock server: scenic SVG "photos" and video frames, document pages,
// WAV audio and a small PDF. All deterministic per item.
import { envelope, rng, type MockItem } from './data';

const hsl = (h: number, s: number, l: number) => `hsl(${h} ${s}% ${l}%)`;

const PALETTE: Record<string, { sky: [string, string]; land: string[]; sun: string; water?: string }> = {
  sunset: { sky: ['#2b1b4f', '#ff8a4c'], land: ['#3a2346', '#24152e', '#160c1d'], sun: '#ffd27a' },
  beach: { sky: ['#5fb4ff', '#d9f1ff'], land: ['#f2d59b', '#e6c27e'], sun: '#fff6c9', water: '#1e8fd6' },
  mountain: { sky: ['#6aa7e8', '#e3f0ff'], land: ['#8495ad', '#56657d', '#2f3a4d'], sun: '#ffffff' },
  forest: { sky: ['#9fd3c7', '#eef7e8'], land: ['#4c8a4a', '#2e6a3a', '#1c4527'], sun: '#fffbe0' },
  city: { sky: ['#0d1330', '#3b2f6b'], land: ['#1b1f3a', '#121530', '#0a0c1f'], sun: '#f5f1d0' },
  lake: { sky: ['#8bc4e8', '#f4f0e0'], land: ['#4f7a5c', '#2f5640'], sun: '#fff3c4', water: '#4d8fb5' },
  desert: { sky: ['#f6b26b', '#ffe6b0'], land: ['#e0a458', '#c9853f', '#a86a2c'], sun: '#fff1c1' },
  field: { sky: ['#7ec8f0', '#f2fbff'], land: ['#9bc95a', '#76a83f', '#4f7f2a'], sun: '#fffbd6' },
};

function ridge(r: () => number, y: number, amp: number, sharp: boolean) {
  let d = `M0 480 L0 ${y}`;
  const steps = sharp ? 7 : 12;
  for (let i = 1; i <= steps; i++) {
    const x = (640 / steps) * i;
    const yy = y - amp * (sharp ? (i % 2 ? 0.4 + r() * 0.8 : r() * 0.2) : 0.3 + r() * 0.7);
    d += sharp ? ` L${x - 640 / steps / 2} ${yy} L${x} ${y - r() * amp * 0.2}` : ` Q${x - 640 / steps / 2} ${yy} ${x} ${y - r() * amp * 0.3}`;
  }
  return `${d} L640 480 Z`;
}

/** A scenic picture for an item; `t` moves the sun a little for video frames. */
export function sceneSvg(it: MockItem, t = 0): string {
  const r = rng(it.id * 31 + Math.floor(t / 4));
  const p = PALETTE[it.scene] ?? PALETTE.field;
  const shift = (t % 20) * 6;
  const sunX = 120 + ((it.id * 37 + shift) % 400);
  const sunY = 90 + (it.id % 7) * 18;
  let body = `<defs><linearGradient id="s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${p.sky[0]}"/><stop offset="1" stop-color="${p.sky[1]}"/></linearGradient>
<radialGradient id="g"><stop offset="0" stop-color="${p.sun}" stop-opacity=".9"/><stop offset="1" stop-color="${p.sun}" stop-opacity="0"/></radialGradient></defs>
<rect width="640" height="480" fill="url(#s)"/><circle cx="${sunX}" cy="${sunY}" r="90" fill="url(#g)"/><circle cx="${sunX}" cy="${sunY}" r="${it.scene === 'city' ? 18 : 30}" fill="${p.sun}"/>`;
  if (it.scene === 'city') {
    for (let i = 0; i < 40; i++) body += `<circle cx="${r() * 640}" cy="${r() * 200}" r="${r() * 1.4 + 0.3}" fill="#fff" opacity="${0.4 + r() * 0.6}"/>`;
  }
  const sharp = it.scene === 'mountain' || it.scene === 'desert';
  p.land.forEach((c, i) => {
    const y = 250 + i * 70 + r() * 30;
    if (it.scene === 'city' && i > 0) {
      let x = 0;
      while (x < 640) {
        const w = 30 + r() * 50;
        const h = 80 + r() * (180 - i * 40);
        body += `<rect x="${x}" y="${y - h + 60}" width="${w}" height="${h + 200}" fill="${c}"/>`;
        for (let k = 0; k < 6; k++) if (r() > 0.4) body += `<rect x="${x + 6 + r() * (w - 14)}" y="${y - h + 70 + r() * h}" width="4" height="6" fill="#ffd77a" opacity=".8"/>`;
        x += w + 4;
      }
    } else {
      body += `<path d="${ridge(r, y, sharp ? 130 - i * 30 : 60 - i * 12, sharp)}" fill="${c}"/>`;
    }
    if (it.scene === 'mountain' && i === 0) body += `<path d="${ridge(r, y - 60, 40, true)}" fill="#fff" opacity=".55"/>`;
  });
  if (p.water) {
    body += `<rect x="0" y="330" width="640" height="150" fill="${p.water}"/>`;
    for (let i = 0; i < 14; i++) body += `<rect x="${r() * 600}" y="${340 + r() * 130}" width="${30 + r() * 80}" height="2" rx="1" fill="#fff" opacity=".35"/>`;
    if (it.scene === 'beach') body += `<path d="M0 420 Q320 380 640 430 L640 480 L0 480Z" fill="${p.land[0]}"/>`;
  }
  if (it.scene === 'forest') {
    for (let i = 0; i < 18; i++) {
      const x = r() * 640;
      const y = 330 + r() * 120;
      const h = 60 + r() * 90;
      body += `<path d="M${x} ${y - h} L${x - h / 3.2} ${y} L${x + h / 3.2} ${y}Z" fill="${hsl(140, 40, 14 + r() * 14)}"/>`;
    }
  }
  if (it.scene === 'field') {
    for (let i = 0; i < 70; i++) body += `<circle cx="${r() * 640}" cy="${360 + r() * 120}" r="${2 + r() * 3}" fill="${['#fff', '#ffd84d', '#ff7aa8', '#c77dff'][i % 4]}"/>`;
  }
  body += `<rect width="640" height="480" fill="${hsl(Math.floor(rng(it.id)() * 360), 70, 50)}" opacity="${(0.06 + rng(it.id + 1)() * 0.14).toFixed(2)}"/>`;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="480" viewBox="0 0 640 480">${body}</svg>`;
}

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

function wrap(text: string, width: number): string[] {
  const out: string[] = [];
  let line = '';
  for (const w of text.split(/\s+/)) {
    if ((line + ' ' + w).trim().length > width) {
      out.push(line);
      line = w;
    } else line = (line + ' ' + w).trim();
  }
  if (line) out.push(line);
  return out;
}

/** A document page as an SVG. */
export function pageSvg(it: MockItem, page: number): string {
  const seg = it.segments.find((s) => s.page === page);
  const title = it.text.split('\n')[0].replace(/^# /, '');
  let y = 90;
  let body = `<rect width="612" height="792" fill="#fff"/><text x="60" y="${y}" font-family="Georgia, serif" font-size="26" fill="#111">${esc(title)}</text>`;
  y += 22;
  body += `<text x="60" y="${y}" font-family="Helvetica, Arial" font-size="12" fill="#777">Page ${page} of ${it.pages}</text>`;
  y += 40;
  for (const line of wrap(seg?.text ?? '', 68)) {
    body += `<text x="60" y="${y}" font-family="Georgia, serif" font-size="15" fill="#222">${esc(line)}</text>`;
    y += 24;
  }
  const r = rng(it.id * 13 + page);
  y += 16;
  while (y < 720) {
    body += `<rect x="60" y="${y - 10}" width="${300 + r() * 190}" height="8" rx="3" fill="#e4e4e4"/>`;
    y += 22;
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" width="612" height="792" viewBox="0 0 612 792">${body}</svg>`;
}

/** Mono 8 kHz 16-bit WAV following the item's loudness envelope. */
export function wav(it: MockItem): Uint8Array {
  const rate = 8000;
  const n = Math.floor(it.duration * rate);
  const buf = new DataView(new ArrayBuffer(44 + n * 2));
  const str = (o: number, s: string) => [...s].forEach((c, i) => buf.setUint8(o + i, c.charCodeAt(0)));
  str(0, 'RIFF');
  buf.setUint32(4, 36 + n * 2, true);
  str(8, 'WAVE');
  str(12, 'fmt ');
  buf.setUint32(16, 16, true);
  buf.setUint16(20, 1, true);
  buf.setUint16(22, 1, true);
  buf.setUint32(24, rate, true);
  buf.setUint32(28, rate * 2, true);
  buf.setUint16(32, 2, true);
  buf.setUint16(34, 16, true);
  str(36, 'data');
  buf.setUint32(40, n * 2, true);
  const f1 = 180 + (it.id % 5) * 60;
  for (let i = 0; i < n; i++) {
    const t = i / rate;
    const env = envelope(it, t);
    const s = Math.sin(2 * Math.PI * f1 * t) * 0.6 + Math.sin(2 * Math.PI * f1 * 1.5 * t) * 0.3;
    buf.setInt16(44 + i * 2, Math.max(-1, Math.min(1, s * env * 0.8)) * 32767, true);
  }
  return new Uint8Array(buf.buffer);
}

/** A minimal multi-page PDF with the item's text. */
export function pdf(it: MockItem): Uint8Array {
  const objs: string[] = [];
  const pages = it.segments.filter((s) => s.page != null);
  const kids = pages.map((_, i) => `${4 + i * 2} 0 R`).join(' ');
  objs.push('<< /Type /Catalog /Pages 2 0 R >>');
  objs.push(`<< /Type /Pages /Kids [${kids}] /Count ${pages.length} >>`);
  objs.push('<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>');
  const pdfEsc = (s: string) => s.replace(/[\\()]/g, '\\$&').replace(/[^\x20-\x7e]/g, '');
  pages.forEach((s) => {
    const lines = wrap(s.text ?? '', 80);
    const stream = `BT /F1 12 Tf 60 720 Td 16 TL ${lines.map((l) => `(${pdfEsc(l)}) '`).join(' ')} ET`;
    const contentId = objs.length + 2;
    objs.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents ${contentId} 0 R >>`);
    objs.push(`<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`);
  });
  let out = '%PDF-1.4\n';
  const offsets: number[] = [];
  objs.forEach((o, i) => {
    offsets.push(out.length);
    out += `${i + 1} 0 obj\n${o}\nendobj\n`;
  });
  const xref = out.length;
  out += `xref\n0 ${objs.length + 1}\n0000000000 65535 f \n${offsets.map((o) => `${String(o).padStart(10, '0')} 00000 n \n`).join('')}`;
  out += `trailer\n<< /Size ${objs.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return new TextEncoder().encode(out);
}
