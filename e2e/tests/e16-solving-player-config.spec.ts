import { test, expect } from '@playwright/test';
import { ADMIN_STATE } from '../lib/env';

test.use({ storageState: ADMIN_STATE });

/**
 * E-16: a player config can solve the end of the game. The pre-endgame needs
 * the endgame, so the form offers it only once the endgame is on; a schedule
 * list that is not whole numbers is named rather than sent; and the config's
 * page says how it solves, beside its search.
 */
test('E-16: a player config solves its endgame and pre-endgame', async ({ page }) => {
  const name = `e16-${crypto.randomUUID().slice(0, 8)}`;
  await page.goto('/admin/player-configs/new');
  await page.getByLabel('Name').fill(name);

  const endgame = page.getByLabel('Uses Endgame', { exact: true });
  const preEndgame = page.getByLabel('Uses Preendgame', { exact: true });
  await expect(preEndgame).toBeDisabled();
  await endgame.check();
  await expect(preEndgame).toBeEnabled();
  await preEndgame.check();
  await expect(page.getByLabel('Preendgame Maximum Bag (-pegbag1)')).toHaveValue('2');
  // Turning the endgame off takes the pre-endgame with it.
  await endgame.uncheck();
  await expect(preEndgame).not.toBeChecked();
  await expect(preEndgame).toBeDisabled();
  await endgame.check();
  await preEndgame.check();

  await page.getByRole('button', { name: 'Show pre-endgame schedule' }).click();
  const survivors = page.getByLabel('PEG Schedule (-pegtopk1)');
  await survivors.fill('4, x');
  await page.getByRole('button', { name: 'Create' }).click();
  await expect(page.getByRole('alert')).toHaveText('PEG Schedule: "x" is not a whole number.');
  await expect(page).toHaveURL(/\/admin\/player-configs\/new$/);

  await survivors.fill('');
  await page.getByRole('button', { name: 'Create' }).click();
  await expect(page).toHaveURL(/\/admin\/player-configs$/);

  await page.goto('/player-configs');
  await page.getByRole('link', { name, exact: true }).click();
  await expect(page.getByRole('heading', { name })).toBeVisible();
  const settings = page.locator('.card', { has: page.getByRole('heading', { name: 'Settings' }) });
  const row = (label: string) =>
    settings.getByRole('row').filter({ has: page.getByRole('cell', { name: label, exact: true }) });
  await expect(page.getByText(/^static, by equity · 6-ply endgame · PEG ≤2 · created/)).toBeVisible();
  await expect(row('Uses Endgame').getByRole('cell')).toHaveText(['Uses Endgame', 'yes (6 plies)']);
  await expect(row('Uses Preendgame').getByRole('cell')).toHaveText(['Uses Preendgame', 'yes (bag ≤ 2)']);
  // The schedule MAGPIE defaults to, written into the config, under All settings.
  await settings.getByRole('button', { name: 'All settings' }).click();
  await expect(row('PEG Schedule').getByRole('cell')).toHaveText(['PEG Schedule', '32, 16, 8, 4, 2']);
  await expect(row('Nested Strides').getByRole('cell')).toHaveText(['Nested Strides', '1, 1, 5, 7']);
});
