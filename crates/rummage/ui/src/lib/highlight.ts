/** Query words worth highlighting (two letters or more). */
export function terms(q: string): string[] {
  return [...new Set(q.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter((t) => t.length >= 2))];
}

export type Part = { text: string; hit?: boolean };

/** Splits `text` into parts, marking where query words appear. */
export function highlight(text: string, words: string[]): Part[] {
  if (!words.length || !text) return [{ text, hit: false }];
  const esc = words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  const re = new RegExp(`(${esc.join('|')})`, 'giu');
  return text
    .split(re)
    .filter((t) => t)
    .map((t) => ({ text: t, hit: words.includes(t.toLowerCase()) }));
}

/** A snippet's pieces without the "…" the server puts where it cut the passage. */
export function trimSpans(spans: Part[]): Part[] {
  let a = 0;
  let b = spans.length;
  if (spans[0]?.text === '…' && !spans[0].hit) a++;
  if (b > a && spans[b - 1].text === '…' && !spans[b - 1].hit) b--;
  return spans.slice(a, b);
}

/**
 * Marks `text` with the server's snippet: its pieces where the snippet appears verbatim, and the
 * words it marked everywhere else. Without a snippet (e.g. a deep link), marks `fallback` words.
 */
export function markWithin(text: string, spans: Part[], fallback: string[]): Part[] {
  const core = trimSpans(spans);
  if (!core.length) return highlight(text, fallback);
  const words = [...new Set(core.filter((s) => s.hit).map((s) => s.text.toLowerCase()))];
  const joined = core.map((s) => s.text).join('');
  const at = text.indexOf(joined);
  if (at < 0) return highlight(text, words);
  const out = [...highlight(text.slice(0, at), words), ...core, ...highlight(text.slice(at + joined.length), words)];
  return out.filter((p) => p.text);
}
