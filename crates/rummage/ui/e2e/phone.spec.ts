import { test, expect } from '@playwright/test';

test('on a phone the preview is a full-screen sheet', async ({ page }) => {
  await page.goto('?q=beach');
  const card = page.locator('#results .card').first();
  await card.tap();
  const sheet = page.locator('.panel');
  await expect(sheet).toBeVisible();
  const box = (await sheet.boundingBox())!;
  const vp = page.viewportSize()!;
  expect(box.x).toBe(0);
  expect(box.width).toBe(vp.width);
  expect(Math.round(box.height)).toBe(vp.height);
  await page.getByRole('button', { name: 'Close preview' }).tap();
  await expect(sheet).toBeHidden();
  await expect(card).toBeVisible();
});

test('on a phone the chips scroll instead of wrapping', async ({ page }) => {
  await page.goto('?q=beach');
  const bar = page.locator('.bar');
  const h = (await bar.boundingBox())!.height;
  expect(h).toBeLessThan(60);
  expect(await bar.evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(true);
});
