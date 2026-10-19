// Regenerates docs/screenshots (SCREENSHOTS=1 bunx playwright test --project screenshots).
import { test, expect, type Page } from '@playwright/test';
import { join } from 'node:path';

const out = (name: string) => join(import.meta.dirname, '..', '..', '..', '..', 'docs', 'screenshots', `${name}.png`);
const settle = (page: Page) =>
  page.waitForFunction(
    () =>
      [...document.images].filter((i) => i.getBoundingClientRect().top < innerHeight).every((i) => i.complete) &&
      document.getAnimations().every((a) => a.playState !== 'running' || a.effect?.getTiming().iterations === Infinity),
  );

const desktop = { viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 };

for (const scheme of ['dark', 'light'] as const) {
  test(`desktop ${scheme}`, async ({ browser }) => {
    const page = await browser.newPage({ ...desktop, colorScheme: scheme });
    await page.goto('http://127.0.0.1:4173/search/');
    await page.evaluate(() => localStorage.setItem('rummage.recent', JSON.stringify(['sunset at the lake', 'tax invoice 2024', 'rain', 'mountain hike'])));
    await page.reload();
    await expect(page.locator('#status-line')).toContainText('up to date');
    await settle(page);
    await page.screenshot({ path: out(`desktop-${scheme}-landing`) });

    await page.keyboard.type('beach');
    await expect(page.locator('#results .card').nth(11)).toBeVisible();
    await page.waitForTimeout(400);
    await settle(page);
    await page.screenshot({ path: out(`desktop-${scheme}-results`) });

    await page.locator('#results .card.k-image').first().click();
    await expect(page.locator('.panel .full.loaded')).toBeVisible();
    await settle(page);
    await page.screenshot({ path: out(`desktop-${scheme}-preview`) });
    await page.close();
  });
}

test('desktop previews and drawer', async ({ browser }) => {
  const page = await browser.newPage({ ...desktop, colorScheme: 'dark' });
  await page.goto('http://127.0.0.1:4173/search/?q=rain&type=audio');
  await page.locator('#results .card').first().click();
  await expect(page.locator('.panel svg.wave')).toBeVisible();
  await page.waitForTimeout(1200);
  await page.locator('.panel button[aria-label="Pause"]').click();
  await settle(page);
  await page.screenshot({ path: out('desktop-dark-audio') });

  await page.goto('http://127.0.0.1:4173/search/?q=invoice&type=doc');
  await page.locator('#results .card').first().click();
  await expect(page.getByRole('img', { name: /^Page/ })).toBeVisible();
  await settle(page);
  await page.screenshot({ path: out('desktop-dark-document') });

  const light = await browser.newPage({ ...desktop, colorScheme: 'light' });
  await light.goto('http://127.0.0.1:4174/search/');
  await expect(light.getByRole('dialog', { name: 'Folders' })).toBeVisible();
  await settle(light);
  await light.screenshot({ path: out('desktop-light-first-run') });
  await page.close();
  await light.close();
});

test('phone', async ({ browser }) => {
  for (const scheme of ['dark', 'light'] as const) {
    const page = await browser.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true, colorScheme: scheme });
    await page.goto('http://127.0.0.1:4173/search/?q=mountain');
    await expect(page.locator('#results .card').nth(3)).toBeVisible();
    await page.waitForTimeout(400);
    await settle(page);
    await page.screenshot({ path: out(`phone-${scheme}-results`) });
    await page.locator('#results .card.k-image').first().tap();
    await expect(page.locator('.panel .full.loaded')).toBeVisible();
    await settle(page);
    await page.screenshot({ path: out(`phone-${scheme}-preview`) });
    await page.close();
  }
});
