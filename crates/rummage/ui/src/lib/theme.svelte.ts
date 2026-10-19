export type Theme = 'system' | 'light' | 'dark';
const KEY = 'rummage.theme';

function load(): Theme {
  try {
    const t = localStorage.getItem(KEY);
    return t === 'light' || t === 'dark' ? t : 'system';
  } catch {
    return 'system';
  }
}

export const theme = $state({ value: load() });

export function cycleTheme() {
  const next: Record<Theme, Theme> = { system: 'light', light: 'dark', dark: 'system' };
  setTheme(next[theme.value]);
}

export function setTheme(t: Theme) {
  theme.value = t;
  const root = document.documentElement;
  if (t === 'system') delete root.dataset.theme;
  else root.dataset.theme = t;
  try {
    if (t === 'system') localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, t);
  } catch {
    /* private mode */
  }
}
