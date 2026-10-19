const KEY = 'rummage.recent';
const MAX = 8;

export function loadRecent(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '[]');
    return Array.isArray(v) ? v.filter((s) => typeof s === 'string').slice(0, MAX) : [];
  } catch {
    return [];
  }
}

export function saveRecent(list: string[]) {
  try {
    localStorage.setItem(KEY, JSON.stringify(list.slice(0, MAX)));
  } catch {
    /* private mode */
  }
}

/** Adds `q` to the front, dropping earlier entries it extends ("sun" when "sunset" is added). */
export function pushRecent(list: string[], q: string): string[] {
  const t = q.trim();
  if (t.length < 2) return list;
  const low = t.toLowerCase();
  return [t, ...list.filter((s) => !low.startsWith(s.toLowerCase()) && s.toLowerCase() !== low)].slice(0, MAX);
}
