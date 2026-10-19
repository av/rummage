/** Query words worth highlighting (two letters or more). */
export function terms(q: string): string[] {
  return [...new Set(q.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter((t) => t.length >= 2))];
}

export type Part = { text: string; hit: boolean };

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
