import { test, expect } from '@playwright/test';

/**
 * E-14: player configs are public. A signed-out visitor finds the seeded
 * configs from the nav, opens one, reads its key settings, and can open
 * every setting.
 */
test('E-14: a visitor reads a player config and all of its settings', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('link', { name: 'Players', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Player configs' })).toBeVisible();
  await page.getByRole('link', { name: 'static-equity', exact: true }).click();

  await expect(page.getByRole('heading', { name: 'static-equity' })).toBeVisible();
  const settings = page.locator('.card', { has: page.getByRole('heading', { name: 'Settings' }) });
  await expect(settings.locator('dd').first()).toHaveText('static, by equity');
  const everySetting = settings.locator('details');
  await expect(everySetting.getByText('Move-gen margin')).toBeHidden();
  await everySetting.locator('summary').click();
  await expect(everySetting.getByText('Move-gen margin')).toBeVisible();
});
