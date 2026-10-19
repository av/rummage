import { dirname } from './format';

/** Fills a `ui.files_url` template: `{path}` is the file, `{dir}` its folder (both URL-encoded per segment). */
export function filesLink(template: string, path: string, which: 'file' | 'dir' = 'file'): string {
  if (!template) return '';
  const encPath = (p: string) => p.split('/').map(encodeURIComponent).join('/');
  const target = which === 'dir' ? dirname(path) : path;
  return template.replaceAll('{path}', encPath(target)).replaceAll('{dir}', encPath(dirname(path)));
}
