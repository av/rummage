// The UI shows what the indexer reports: snippet match spans, live folder progress, the global
// pause, files it couldn't read, status messages, and server-drawn waveforms.
import { test, expect, type Page, type Route } from '@playwright/test';

const cards = (page: Page) => page.locator('#results .card');
const drawer = (page: Page) => page.getByRole('dialog', { name: 'Folders' });

/** Answers the event stream with one status snapshot; the browser reconnects after `retry` ms. */
function sse(route: Route, status: object, retry = 100) {
  return route.fulfill({
    status: 200,
    headers: { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' },
    body: `retry: ${retry}\nevent: status\ndata: ${JSON.stringify({ type: 'status', ...status })}\n\n`,
  });
}

test('snippets mark the words the server matched', async ({ page, request }) => {
  const res = await request.post('api/v1/search', { data: { query: 'bicy', filters: { types: ['text'] }, limit: 1 } });
  const spans: { text: string; hit?: boolean }[] = (await res.json()).hits[0].segment.snippet;
  const marked = spans.filter((s) => s.hit).map((s) => s.text);
  // The server marks whole words that start with a query word, which the browser alone did not.
  expect(marked).toContain('bicycle');

  await page.goto('?q=bicy&type=text');
  const card = cards(page).first();
  await expect(card.locator('.snippet')).toHaveText(spans.map((s) => s.text).join(''));
  await expect(card.locator('.snippet mark')).toHaveText(marked);

  // The text preview marks the same words in the file, at the matched passage.
  await card.click();
  await expect(page.locator('.text .hit')).toBeInViewport();
  await expect(page.locator('.text .hit mark').first()).toHaveText('bicycle');
});

test('the folders drawer follows scanning, counts and ETA live', async ({ page, request }) => {
  const base = await (await request.get('api/v1/index/status')).json();
  const pictures = base.folders.findIndex((f: { path: string }) => f.path.endsWith('/Pictures'));
  const files = base.folders[pictures].counts.files;
  const at = (patch: object, state: string, eta: number | null) => ({
    ...base,
    state,
    eta_secs: eta,
    folders: base.folders.map((f: object, i: number) => (i === pictures ? { ...f, ...patch } : f)),
  });
  const scanning = at({ scanning: true, eta_secs: null }, 'scanning', null);
  const indexing = at({ scanning: false, eta_secs: 120, counts: { ...base.folders[pictures].counts, indexed: 4000, pending: files - 4000 } }, 'indexing', 120);
  const done = at({ scanning: false, eta_secs: null }, 'idle', null);

  const steps = [scanning, indexing, done];
  const gates = steps.map(() => Promise.withResolvers<void>());
  gates[0].resolve();
  let n = 0;
  await page.route('**/api/v1/index/events', async (route) => {
    const k = Math.min(n++, steps.length - 1);
    await gates[k].promise;
    await sse(route, steps[k], k === steps.length - 1 ? 60000 : 100);
  });

  await page.goto('');
  await page.getByRole('button', { name: 'Folders and settings' }).click();
  const row = drawer(page).locator('.folder', { hasText: 'Pictures' });
  await expect(row.locator('.tag')).toHaveText('Scanning');
  await expect(row).toContainText('looking for new and changed files');
  await expect(drawer(page)).toContainText('Looking for new and changed files');

  gates[1].resolve();
  await expect(row.locator('.tag')).toHaveText('Indexing');
  await expect(row).toContainText(`4,000 of ${files.toLocaleString('en-US')} files indexed`);
  await expect(row).toContainText('about 2 min left');
  await expect(row.getByRole('progressbar')).toHaveAttribute('aria-valuenow', String(Math.round((100 * 4000) / files)));

  gates[2].resolve();
  await expect(row.locator('.tag')).toHaveText('Up to date');
  await expect(row).not.toContainText('left');
});

test('pause and resume switch all indexing, not each folder', async ({ page }) => {
  const calls: string[] = [];
  page.on('request', (r) => {
    if (r.url().includes('/api/v1/') && r.method() !== 'GET') calls.push(`${r.method()} ${new URL(r.url()).pathname}`);
  });
  await page.goto('http://127.0.0.1:4176/search/');
  await expect(page.locator('#status-line')).not.toContainText('Loading');
  await page.getByRole('button', { name: 'Folders and settings' }).click();
  await drawer(page).getByRole('button', { name: 'Pause indexing' }).click();
  await expect(drawer(page)).toContainText('Indexing is paused');
  await expect(drawer(page).getByRole('button', { name: 'Resume indexing' })).toBeVisible();
  await page.keyboard.press('Escape');

  const line = page.locator('#status-line');
  await expect(line).toContainText('indexing paused');
  await line.getByRole('button', { name: 'Resume' }).click();
  await expect(line).not.toContainText('paused');
  expect(calls).toEqual(['POST /search/api/v1/index/pause', 'POST /search/api/v1/index/resume']);
});

test('files that could not be read are listed with the reason, one click from the status line', async ({ page }) => {
  await page.goto('');
  const link = page.locator('#status-line').getByRole('button', { name: '2 files could not be read, 1 file skipped' });
  await link.click();
  await expect(drawer(page)).toBeVisible();
  const title = drawer(page).getByRole('heading', { name: /Not indexed/ });
  await expect(title).toHaveText('Not indexed 3');
  await expect(title).toBeFocused();
  await expect(title).toBeInViewport();

  const list = drawer(page).locator('.problems li');
  await expect(list).toHaveCount(3);
  await expect(list.filter({ hasText: 'Live at the Roundhouse.flac' })).toContainText('invalid FLAC stream header');
  await expect(list.filter({ hasText: 'Screen recording.mkv' }).locator('.tag')).toHaveText('Skipped');
  await expect(drawer(page).locator('.folder', { hasText: 'Music' })).toContainText('2 could not be read');

  const retry = page.waitForRequest((r) => r.url().endsWith('/api/v1/reindex') && r.postDataJSON()?.scope === 'problems');
  await drawer(page).getByRole('button', { name: 'Try again' }).click();
  await retry;
  await expect(page.getByRole('status')).toHaveText('Trying 3 files again');
});

test('status messages from the server show on the status line', async ({ page, request }) => {
  const messages = [
    'ffmpeg not found: videos (and HEIC/AVIF photos) are skipped; install ffmpeg and rescan to index them',
    'The embedding model is not ready: it is loading (it downloads on first start) or failed to load (see the log); until then searches match words only and indexing waits.',
  ];
  const status = { ...(await (await request.get('api/v1/index/status')).json()), messages };
  await page.route('**/api/v1/index/status', (route) => route.fulfill({ json: status }));
  await page.route('**/api/v1/index/events', (route) => sse(route, status, 60000));
  await page.goto('');
  const notes = page.getByRole('list', { name: 'Notes from the server' });
  await expect(notes.getByRole('listitem')).toHaveText(messages);
  await expect(page.locator('#status-line')).toContainText('up to date');
});

test('audio waveforms come from the server', async ({ page }) => {
  const asked: string[] = [];
  page.on('request', (r) => asked.push(`${r.resourceType()} ${new URL(r.url()).pathname}${new URL(r.url()).search}`));
  await page.goto('?q=rain&type=audio');
  await expect(cards(page).first().locator('svg.wave')).toBeVisible();
  await cards(page).first().click();
  await expect(page.locator('.panel .track svg.wave')).toBeVisible();
  expect(asked.some((a) => /\/waveform\?points=300$/.test(a))).toBe(true);
  // Nothing was downloaded to draw it here.
  expect(asked.filter((a) => a.startsWith('fetch') && a.includes('/raw'))).toEqual([]);
});

test('without a server waveform, the preview draws one from the file', async ({ page }) => {
  await page.route('**/api/v1/items/*/waveform*', (route) => route.fulfill({ status: 404, json: { error: 'not_found', message: 'no waveform yet' } }));
  const decoded = page.waitForRequest((r) => r.resourceType() === 'fetch' && r.url().includes('/raw'));
  await page.goto('?q=rain&type=audio');
  await cards(page).first().click();
  await decoded;
  await expect(page.locator('.panel .track svg.wave')).toBeVisible();
  // Result cards don't download files for this; they show an icon instead.
  await expect(cards(page).first().locator('svg.wave')).toHaveCount(0);
});
