// A document hit on a page's picture has no text: its card shows that page, not an empty box.
import { test, expect } from '@playwright/test';

test('a document page that matched by its picture shows the page', async ({ page }) => {
  let id = 0;
  await page.route('**/api/v1/search', async (route) => {
    const res = await route.fetch();
    const body = await res.json();
    const hit = body.hits.find((h: { item: { kind: string } }) => h.item.kind === 'doc');
    hit.segment = { id: 1, kind: 'page', page: 1, t_start: null, t_end: null, text: null, score: null };
    id = hit.item.id;
    await route.fulfill({ response: res, json: body });
  });
  await page.goto('?q=invoice&type=doc');
  await expect(page.locator('#results .card').first()).toBeVisible();

  const card = page.locator(`#results .card[data-id="${id}"]`);
  const img = card.locator('img.page');
  await expect(img).toHaveAttribute('src', /\/items\/\d+\/preview\?page=1/);
  await expect(img).toHaveClass(/loaded/);
  await expect(card.locator('.snippet')).toHaveCount(0);
  await expect(card.locator('.badge')).toHaveText('p. 1');
});
