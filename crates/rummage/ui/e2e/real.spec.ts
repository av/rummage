// The UI embedded in the real binary, against the real API (no mock).
import { test, expect } from '@playwright/test';

test('rummage serve embeds the UI and it works against the real API', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto('');
  await expect(page).toHaveTitle('rummage');

  // First run: no folders, so the drawer opens (focused) on the server's home directory.
  const drawer = page.getByRole('dialog', { name: 'Folders' });
  await expect(drawer).toBeVisible();
  await expect(drawer.getByLabel('Folder path')).toBeFocused();
  await drawer.getByRole('button', { name: /^Add .*\/notes$/ }).click();
  await expect(drawer.locator('.folder')).toContainText('notes');
  await expect(drawer.getByText('Indexed', { exact: true })).toBeVisible();
  await page.keyboard.press('Escape');

  await page.getByRole('searchbox').fill('garden');
  // Whether or not the indexer has run, the page shows results or a clear empty state.
  await expect(page.locator('#results .card').first().or(page.getByText(/No matches/))).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0);
  expect(errors).toEqual([]);
});
