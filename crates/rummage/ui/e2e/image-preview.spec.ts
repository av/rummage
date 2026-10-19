// The image preview shows the whole picture, upright, fitted inside its frame: a phone photo
// stored landscape with EXIF Orientation 6 (portrait when upright) must not be cropped.
import { test, expect, type Page } from '@playwright/test';
import { join } from 'node:path';

// 640 x 360 stored, Orientation 6: upright it is 360 x 640, with a red band along the top.
const FIXTURE = join(import.meta.dirname, 'fixtures', 'portrait-exif6.jpg');

// The preview may still be loading the item's details when a test ends.
test.afterEach(({ page }) => page.unrouteAll({ behavior: 'ignoreErrors' }));

/** Opens the first image result, its preview served from the fixture. */
async function openPortrait(page: Page, size?: { width: number; height: number }) {
  await page.route('**/api/v1/items/*/preview', (route) => route.fulfill({ path: FIXTURE, contentType: 'image/jpeg' }));
  if (size) {
    // The size the indexer stores is the upright one; results and item details both carry it.
    await page.route(
      (url) => /\/api\/v1\/(search|items\/\d+)$/.test(url.pathname),
      async (route) => {
        const res = await route.fetch();
        const body = await res.json();
        const items: ({ kind: string; exif?: object | null } | undefined)[] = [...(body.hits ?? []).map((h: { item: object }) => h.item), body.item];
        for (const it of items) {
          if (it?.kind === 'image') it.exif = { ...it.exif, ...size };
        }
        await route.fulfill({ response: res, json: body });
      },
    );
  }
  await page.goto('?q=beach&type=image');
  await page.locator('#results .card').first().click();
  const img = page.locator('.panel .view img.full');
  await expect(img).toHaveClass(/loaded/);
  return img;
}

/** Where the picture is drawn (object-fit applied), the frame, and the colour at its top. */
function geometry(page: Page) {
  return page.evaluate(async () => {
    const img = document.querySelector<HTMLImageElement>('.panel .view img.full')!;
    const frame = img.parentElement!.getBoundingClientRect();
    const box = img.getBoundingClientRect();
    const scale = Math.min(box.width / img.naturalWidth, box.height / img.naturalHeight);
    const drawn = {
      x: box.x + (box.width - img.naturalWidth * scale) / 2,
      y: box.y + (box.height - img.naturalHeight * scale) / 2,
      width: img.naturalWidth * scale,
      height: img.naturalHeight * scale,
    };
    // The bitmap the browser draws, turned by the EXIF orientation like the <img>.
    const bitmap = await createImageBitmap(img);
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const ctx = canvas.getContext('2d')!;
    ctx.drawImage(bitmap, 0, 0);
    const top = [...ctx.getImageData(bitmap.width / 2, 5, 1, 1).data.slice(0, 3)];
    return { natural: [img.naturalWidth, img.naturalHeight], fit: getComputedStyle(img).objectFit, frame, box, drawn, top };
  });
}

function expectWholeAndUpright(g: Awaited<ReturnType<typeof geometry>>) {
  expect(g.natural).toEqual([360, 640]);
  expect(g.fit).toBe('contain');
  const [r, , b] = g.top;
  expect(r).toBeGreaterThan(150);
  expect(b).toBeLessThan(100);
  // The element and the drawn picture both stay inside the frame (nothing clipped).
  for (const rect of [g.box, g.drawn]) {
    expect(rect.x).toBeGreaterThanOrEqual(g.frame.x - 0.5);
    expect(rect.y).toBeGreaterThanOrEqual(g.frame.y - 0.5);
    expect(rect.x + rect.width).toBeLessThanOrEqual(g.frame.x + g.frame.width + 0.5);
    expect(rect.y + rect.height).toBeLessThanOrEqual(g.frame.y + g.frame.height + 0.5);
  }
  expect(g.drawn.width / g.drawn.height).toBeCloseTo(360 / 640, 2);
  // A portrait picture uses the frame's full height.
  expect(g.drawn.height).toBeGreaterThan(g.frame.height - 1);
}

test('a portrait photo turned by its EXIF shows whole and upright, in a portrait frame', async ({ page }) => {
  await openPortrait(page, { width: 360, height: 640 });
  const g = await geometry(page);
  expectWholeAndUpright(g);
  // The frame takes the photo's shape (up to its height limit), so the picture isn't shrunk
  // into a 4:3 box.
  expect(g.frame.height).toBeGreaterThan(g.frame.width);
});

test('without a stored size the 4:3 frame still shows the whole portrait photo', async ({ page }) => {
  await openPortrait(page);
  const g = await geometry(page);
  expectWholeAndUpright(g);
  expect(g.frame.width / g.frame.height).toBeCloseTo(4 / 3, 1);
});
