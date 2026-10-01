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
  // How it searches, in a few words, beside its name.
  await expect(page.getByText(/^static, by equity · created/)).toBeVisible();
  const settings = page.locator('.card', { has: page.getByRole('heading', { name: 'Settings' }) });
  await expect(settings.getByText('Move-gen margin')).toBeHidden();
  await settings.getByRole('button', { name: 'All settings' }).click();
  await expect(settings.getByText('Move-gen margin')).toBeVisible();
  // The key rows stay first when the table grows.
  await expect(settings.getByRole('row').first().getByRole('cell').first()).toHaveText('Lexicon');
  await settings.getByRole('button', { name: 'Key settings only' }).click();
  await expect(settings.getByText('Move-gen margin')).toBeHidden();
});
