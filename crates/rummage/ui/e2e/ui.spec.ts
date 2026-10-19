import { test, expect, type Page, type APIRequestContext } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

const search = (page: Page) => page.getByRole('searchbox', { name: 'Search your files' });
const cards = (page: Page) => page.locator('#results .card');
const panel = (page: Page) => page.getByRole('dialog', { name: /./ }).filter({ has: page.locator('#preview-title') });

async function hits(request: APIRequestContext, query: string, limit = 60) {
  const res = await request.post('api/v1/search', { data: { query, limit } });
  expect(res.ok()).toBeTruthy();
  return (await res.json()).hits as { item: { id: number; name: string; kind: string } }[];
}

test.describe('acceptance', () => {
  test('(a) page load to an open preview takes at most 2 actions beyond typing', async ({ page }) => {
    let actions = 0;
    const act = async (fn: () => Promise<unknown>) => {
      actions++;
      await fn();
    };
    await page.goto('');
    // The box is focused on load: typing needs no click.
    await expect(search(page)).toBeFocused();
    await page.keyboard.type('sunset');
    await expect(cards(page).first()).toBeVisible();
    await act(() => cards(page).first().click());
    await expect(panel(page)).toBeVisible();
    await expect(page.locator('#preview-title')).not.toBeEmpty();
    expect(actions).toBeLessThanOrEqual(2);
    expect(page.url()).toMatch(/[?&]item=\d+/);

    // The keyboard path is one action too: Enter opens the top result.
    actions = 0;
    await page.goto('');
    await page.keyboard.type('beach');
    await act(() => page.keyboard.press('Enter'));
    await expect(panel(page)).toBeVisible();
    expect(actions).toBeLessThanOrEqual(2);
  });

  test('(b) a deep link opens straight into the preview', async ({ page, request }) => {
    const list = await hits(request, 'sunset');
    const target = list[5].item;
    await page.goto(`?q=sunset&item=${target.id}`);
    await expect(panel(page)).toBeVisible();
    await expect(page.locator('#preview-title')).toHaveText(target.name);
    await expect(search(page)).toHaveValue('sunset');
    await expect(cards(page).first()).toBeVisible();
    // Results around it are there too, and the preview steps through them.
    await expect(page.getByText(`6 of`)).toBeVisible();

    // An item that is not among the results opens as well.
    const other = (await hits(request, 'invoice'))[0].item;
    await page.goto(`?q=sunset&item=${other.id}`);
    await expect(page.locator('#preview-title')).toHaveText(other.name);
  });

  test('(c) everything works from the keyboard', async ({ page }) => {
    await page.goto('');
    await page.evaluate(() => (document.activeElement as HTMLElement).blur());
    await page.keyboard.press('/');
    await expect(search(page)).toBeFocused();
    await page.keyboard.type('beach');
    await expect(cards(page).first()).toBeVisible();

    await page.keyboard.press('ArrowDown');
    await expect(cards(page).nth(0)).toBeFocused();
    await page.keyboard.press('ArrowRight');
    await expect(cards(page).nth(1)).toBeFocused();
    const second = await cards(page).nth(1).getAttribute('data-id');
    await page.keyboard.press('Enter');
    await expect(panel(page)).toBeVisible();
    await expect(page.locator('#preview-title')).toBeFocused();
    expect(new URL(page.url()).searchParams.get('item')).toBe(second);

    const third = await cards(page).nth(2).getAttribute('data-id');
    await page.keyboard.press('ArrowRight');
    await expect.poll(() => new URL(page.url()).searchParams.get('item')).toBe(third);
    await page.keyboard.press('ArrowLeft');
    await expect.poll(() => new URL(page.url()).searchParams.get('item')).toBe(second);

    await page.keyboard.press('Escape');
    await expect(panel(page)).toBeHidden();
    await expect(cards(page).nth(1)).toBeFocused();
    expect(new URL(page.url()).searchParams.get('item')).toBeNull();

    // Up from the first row goes back to the box; Esc there clears it.
    await page.keyboard.press('ArrowLeft');
    await page.keyboard.press('ArrowUp');
    await expect(search(page)).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(search(page)).toHaveValue('');

    // The folders drawer opens and closes from the keyboard too.
    await page.getByRole('button', { name: 'Folders and settings' }).focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('dialog', { name: 'Folders' })).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog', { name: 'Folders' })).toBeHidden();
    await expect(page.getByRole('button', { name: 'Folders and settings' })).toBeFocused();
  });

  test('(d) first results render within 300 ms of the response (10k-item index)', async ({ page }) => {
    const health = await (await page.request.get('api/v1/index/status')).json();
    expect(health.totals.files).toBeGreaterThanOrEqual(10000);
    await page.goto('');
    const gaps: number[] = [];
    for (const q of ['sunset', 'mountain hike', 'rain']) {
      await search(page).fill('');
      await page.evaluate(() => {
        performance.clearMarks();
        performance.clearResourceTimings();
      });
      await search(page).fill(q);
      await expect(cards(page).first()).toBeVisible();
      const gap = await page.evaluate(
        () =>
          new Promise<number>((resolve) => {
            const check = () => {
              const mark = performance.getEntriesByName('rummage:results').at(-1);
              const res = performance
                .getEntriesByType('resource')
                .filter((e) => e.name.includes('/api/v1/search'))
                .at(-1) as PerformanceResourceTiming | undefined;
              if (mark && res) resolve(mark.startTime - res.responseEnd);
              else setTimeout(check, 20);
            };
            check();
          }),
      );
      gaps.push(gap);
    }
    console.log(`response -> first results painted: ${gaps.map((g) => g.toFixed(1)).join(', ')} ms`);
    for (const g of gaps) expect(g).toBeLessThan(300);
  });
});

test.describe('search page', () => {
  test('URL holds the state; Back and Forward behave', async ({ page }) => {
    await page.goto('');
    await page.keyboard.type('sunset');
    await expect.poll(() => new URL(page.url()).searchParams.get('q')).toBe('sunset');
    await page.getByRole('radio', { name: /Images/ }).click();
    await expect.poll(() => new URL(page.url()).searchParams.get('type')).toBe('image');
    await expect
      .poll(() => cards(page).evaluateAll((els) => els.length > 0 && els.slice(0, 12).every((e) => e.classList.contains('k-image'))))
      .toBe(true);

    await cards(page).first().click();
    await expect(panel(page)).toBeVisible();
    await page.goBack();
    await expect(panel(page)).toBeHidden();
    await expect.poll(() => new URL(page.url()).searchParams.get('item')).toBeNull();
    await page.goBack();
    await expect(page.getByRole('radio', { name: /All/ })).toHaveAttribute('aria-checked', 'true');
    await page.goForward();
    await expect(page.getByRole('radio', { name: /Images/ })).toHaveAttribute('aria-checked', 'true');
    await page.goBack();
    await page.goBack();
    await expect(search(page)).toHaveValue('');
    await expect(page.locator('#status-line')).toBeVisible();
  });

  test('filter chips show counts per kind', async ({ page }) => {
    await page.goto('?q=beach');
    const audio = page.getByRole('radio', { name: /Audio/ });
    await expect(audio).toContainText(/\d/);
    const n = Number((await audio.locator('.count').innerText()).replace(/\D/g, ''));
    expect(n).toBeGreaterThan(0);
    await audio.click();
    await expect(cards(page).first()).toHaveClass(/k-audio/);
    // Counts stay for the other kinds while one is selected.
    await expect(page.getByRole('radio', { name: /Images/ }).locator('.count')).toHaveText(/[1-9]/);
  });

  test('infinite scroll adds results without moving the ones shown', async ({ page }) => {
    await page.goto('?q=sunset');
    await expect(cards(page)).toHaveCount(60);
    const before = await cards(page).first().boundingBox();
    await page.mouse.wheel(0, 20000);
    await expect.poll(() => cards(page).count()).toBeGreaterThan(60);
    await page.evaluate(() => window.scrollTo(0, 0));
    expect(await cards(page).first().boundingBox()).toEqual(before);
  });

  test('works at the root as well as under /search', async ({ page }) => {
    await page.goto('/?q=sunset');
    await expect(cards(page).first()).toBeVisible();
    const src = await cards(page).first().locator('img').getAttribute('src');
    expect(new URL(src!).pathname).toMatch(/^\/api\/v1\/items\/\d+\//);
    await page.goto('/search?q=sunset');
    expect(new URL(page.url()).pathname).toBe('/search/');
    const src2 = await cards(page).first().locator('img').getAttribute('src');
    expect(new URL(src2!).pathname).toMatch(/^\/search\/api\/v1\/items\/\d+\//);
  });

  test('More like this searches by the item', async ({ page }) => {
    await page.goto('?q=beach');
    await cards(page).first().click();
    const id = new URL(page.url()).searchParams.get('item');
    await page.getByRole('button', { name: 'More like this' }).click();
    await expect.poll(() => new URL(page.url()).searchParams.get('like')).toBe(id);
    await expect(page.locator('.by')).toBeVisible();
    await expect(cards(page).first()).toBeVisible();
    expect(await cards(page).evaluateAll((els, id) => els.some((e) => e.getAttribute('data-id') === id), id)).toBe(false);
  });

  test('dropping or picking a file searches by it', async ({ page }) => {
    await page.goto('');
    const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64');
    await page.locator('input[type=file]').setInputFiles({ name: 'photo.png', mimeType: 'image/png', buffer: png });
    await expect(page.locator('.by')).toContainText('photo.png');
    await expect(cards(page).first()).toBeVisible();

    await page.goto('');
    const dt = await page.evaluateHandle(() => {
      const d = new DataTransfer();
      d.items.add(new File(['the quarterly tax invoice'], 'note.txt', { type: 'text/plain' }));
      return d;
    });
    await page.dispatchEvent('body', 'dragenter', { dataTransfer: dt });
    await expect(page.getByText('Drop to search by this file')).toBeVisible();
    await page.dispatchEvent('body', 'drop', { dataTransfer: dt });
    await expect(page.locator('.by')).toContainText('note.txt');
    await expect(cards(page).first()).toBeVisible();

    // A file the server can't search by explains itself.
    await page.locator('input[type=file]').setInputFiles({ name: 'song.mp3', mimeType: 'audio/mpeg', buffer: Buffer.from('ID3') });
    await expect(page.getByRole('alert')).toContainText("Can't search by this file");
  });

  test('preview actions: copy path, open in files, reveal folder', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await page.goto('?q=garden&type=text');
    await cards(page).first().click();
    const path = await page.locator('.path').getAttribute('title');
    await page.getByRole('button', { name: 'Copy path' }).click();
    await expect(page.getByRole('status')).toHaveText('Path copied');
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(path);
    await expect(page.getByRole('link', { name: 'Open in Files' })).toHaveAttribute('href', `/files/files${path}`);
    await expect(page.getByRole('link', { name: 'Open', exact: true })).toHaveAttribute('href', /\/raw/);
    // The text preview scrolls to the matched passage.
    await expect(page.locator('.text .hit')).toBeInViewport();
    await page.getByRole('button', { name: 'Reveal folder' }).click();
    const dir = path!.slice(0, path!.lastIndexOf('/'));
    await expect.poll(() => new URL(page.url()).searchParams.get('in')).toBe(dir);
    await expect(page.getByText(`In ${dir.split('/').pop()}`)).toBeVisible();
  });

  test('media previews open at the matched moment and page', async ({ page }) => {
    await page.goto('?q=sunset&type=video');
    const first = (await (await page.request.post('api/v1/search', { data: { query: 'sunset', filters: { types: ['video'] } } })).json()).hits[0];
    await cards(page).first().click();
    const video = page.locator('.panel video');
    await expect(video).toBeVisible();
    await expect.poll(() => video.evaluate((v: HTMLVideoElement) => v.currentTime), { timeout: 10000 }).toBeGreaterThanOrEqual(first.segment.t_start);
    await expect(page.locator('.panel .track .mark').first()).toBeVisible();

    await page.goto('?q=invoice&type=doc');
    const doc = (await (await page.request.post('api/v1/search', { data: { query: 'invoice', filters: { types: ['doc'] } } })).json()).hits[0];
    await cards(page).first().click();
    await expect(page.getByText(`Page ${doc.segment.page} of`)).toBeVisible();
    await expect(page.getByRole('img', { name: `Page ${doc.segment.page} of` })).toBeVisible();

    await page.goto('?q=rain&type=audio');
    await cards(page).first().click();
    await expect(page.locator('.panel .track svg.wave')).toBeVisible();
  });

  test('says what to do when nothing matches or the server is down', async ({ page }) => {
    await page.goto('?q=zzqxv');
    await expect(page.getByText('No matches for “zzqxv”')).toBeVisible();
    await page.goto('?q=zzqxv&type=image');
    await page.getByRole('button', { name: 'Clear filters' }).first().click();
    await expect.poll(() => new URL(page.url()).searchParams.get('type')).toBeNull();

    await page.route('**/api/v1/search', (r) => r.abort());
    await page.goto('?q=sunset');
    await expect(page.getByRole('alert')).toContainText("Can't reach rummage");
    await page.unroute('**/api/v1/search');
    await page.getByRole('button', { name: 'Try again' }).click();
    await expect(cards(page).first()).toBeVisible();
  });

  test('theme follows the system and can be toggled', async ({ page }) => {
    await page.emulateMedia({ colorScheme: 'dark' });
    await page.goto('');
    const bg = () => page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    expect(await bg()).toBe('rgb(14, 16, 20)');
    await page.getByRole('button', { name: /Theme/ }).click();
    expect(await page.evaluate(() => document.documentElement.dataset.theme)).toBe('light');
    expect(await bg()).toBe('rgb(247, 247, 248)');
    await page.reload();
    expect(await bg()).toBe('rgb(247, 247, 248)');
  });

  test('no accessibility violations (axe, WCAG AA) in both themes', async ({ page }) => {
    for (const scheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      const check = async (where: string) => {
        // Let enter transitions finish; mid-fade text has lower contrast than it ends with.
        await page.waitForFunction(() => document.getAnimations().every((a) => a.playState !== 'running' || a.effect?.getTiming().iterations === Infinity));
        const r = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
        expect(r.violations.map((v) => `${where}/${scheme}: ${v.id} ${v.nodes.map((n) => n.target).join(' ')}`)).toEqual([]);
      };
      await page.goto('');
      await expect(page.locator('#status-line')).not.toContainText('Loading');
      await check('landing');
      await page.goto('?q=garden');
      await expect(cards(page).first()).toBeVisible();
      await check('results');
      await cards(page).first().click();
      await expect(panel(page)).toBeVisible();
      await check('preview');
      await page.keyboard.press('Escape');
      await page.getByRole('button', { name: 'Folders and settings' }).click();
      await expect(page.getByRole('dialog', { name: 'Folders' })).toBeVisible();
      await check('drawer');
    }
  });
});

test.describe('folders', () => {
  test('first run: the drawer opens and one click starts indexing', async ({ page }) => {
    await page.goto('http://127.0.0.1:4174/search/');
    const drawer = page.getByRole('dialog', { name: 'Folders' });
    await expect(drawer).toBeVisible();
    await expect(drawer).toContainText('Pick the folders you want to search');
    await drawer.getByRole('button', { name: 'Add /home/me/Pictures' }).click();
    await expect(drawer.locator('.folder')).toContainText('Pictures');
    await expect(drawer.locator('.folder')).toContainText(/of [\d,]+ files indexed/);
    await expect(page.locator('#status-line')).toContainText('Indexing');
  });

  test('add by path, pause, exclude, reindex and remove', async ({ page }) => {
    await page.goto('http://127.0.0.1:4176/search/');
    await page.getByRole('button', { name: 'Folders and settings' }).click();
    const drawer = page.getByRole('dialog', { name: 'Folders' });
    await drawer.getByLabel('Folder path').fill('relative/path');
    await drawer.getByRole('button', { name: 'Add', exact: true }).click();
    await expect(drawer.getByRole('alert')).toContainText('must be absolute');
    await drawer.getByLabel('Folder path').fill('/srv/archive');
    await drawer.getByRole('button', { name: 'Add', exact: true }).click();
    const row = drawer.locator('.folder', { hasText: 'archive' });
    await expect(row).toBeVisible();

    await row.getByRole('button', { name: 'Pause' }).click();
    await expect(row.getByText('Paused')).toBeVisible();
    await row.getByRole('button', { name: 'Resume' }).click();
    await expect(row.getByRole('button', { name: 'Pause' })).toBeVisible();

    await row.getByRole('button', { name: 'Exclude…' }).click();
    await row.getByRole('textbox').fill('**/*.raw\nScreenshots/**');
    await row.getByRole('button', { name: 'Save' }).click();
    await expect(row).toContainText('2 exclusions');

    await row.getByRole('button', { name: 'Reindex' }).click();
    await expect(page.getByRole('status')).toHaveText('Reindexing');

    await row.getByRole('button', { name: 'Remove' }).click();
    await row.getByRole('button', { name: 'Remove' }).click();
    await expect(row).toBeHidden();
  });

  test('an access token is asked for when the server needs one', async ({ page }) => {
    await page.goto('http://127.0.0.1:4175/search/?q=sunset');
    const dialog = page.getByRole('dialog', { name: /access token/ });
    await expect(dialog).toBeVisible();
    await dialog.getByLabel('Access token').fill('secret');
    await dialog.getByRole('button', { name: 'Continue' }).click();
    await expect(dialog).toBeHidden();
    await expect(cards(page).first()).toBeVisible();
    // Media URLs carry the token, since <img> can't send headers.
    expect(await cards(page).first().locator('img').getAttribute('src')).toContain('access_token=secret');
  });
});
