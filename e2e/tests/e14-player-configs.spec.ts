import { test, expect } from '@playwright/test';

/**
 * E-14: player configs are public. A signed-out visitor finds the seeded
 * configs from the nav, opens one, reads its key settings in a table, and
 * can open every setting in the same table.
 */
test('E-14: a visitor reads a player config and all of its settings', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('link', { name: 'Players', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Player configs' })).toBeVisible();
  await page.getByRole('link', { name: 'static-equity', exact: true }).click();

  await expect(page.getByRole('heading', { name: 'static-equity' })).toBeVisible();
  const settings = page.locator('.card', { has: page.getByRole('heading', { name: 'Settings' }) });
  const search = settings.getByRole('row').filter({ has: page.getByRole('cell', { name: 'Search', exact: true }) });
  await expect(search.getByRole('cell')).toHaveText(['Search', 'static, by equity']);
  await expect(settings.getByText('Move-gen margin')).toBeHidden();
  await settings.getByRole('button', { name: 'All settings' }).click();
  await expect(settings.getByText('Move-gen margin')).toBeVisible();
  // The search stays the first row when the table grows.
  await expect(settings.getByRole('row').first().getByRole('cell').first()).toHaveText('Search');
  await settings.getByRole('button', { name: 'Key settings only' }).click();
  await expect(settings.getByText('Move-gen margin')).toBeHidden();
});
