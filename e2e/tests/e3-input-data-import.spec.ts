import { test, expect } from '@playwright/test';
import { ADMIN_STATE, IMPORTED_DATA } from '../lib/env';

test.use({ storageState: ADMIN_STATE });

/**
 * E-3: an admin imports input data, reviews the staged diff, and confirms it.
 *
 * The version imported is the fixture's chunked one, fetched from the fixture
 * server exactly as from GitHub -- ref resolved to a sha, chunks walked until
 * one is missing. Against the version the seed imported it re-cuts one file
 * (a collision) and adds one, so the review shows every disposition.
 */
test('E-3: an admin imports a tarball, reviews the staged diff and confirms it', async ({ page }) => {
  await page.goto('/admin/input-data');
  await expect(page.getByRole('heading', { name: 'Input data' })).toBeVisible();

  await page.getByLabel('Version (YYYYMMDD)').fill(IMPORTED_DATA);
  await page.getByLabel('Ref').fill('main');
  await page.getByRole('button', { name: 'Fetch and diff' }).click();

  // The staged diff: one new file, one changed, the rest already known.
  const wizard = page.locator('.card', { has: page.getByRole('heading', { name: 'Import a tarball' }) });
  await expect(wizard.locator('p', { hasText: 'already known.' })).toHaveText(
    '1 new, 1 changed, 4 already known.'
  );
  await expect(wizard.getByText('A changed file is a path already known under different bytes')).toBeVisible();
  const staged = wizard.locator('tbody tr');
  await expect(staged).toHaveCount(2);
  await expect(staged.nth(0)).toContainText('strategy/winpct.csv');
  await expect(staged.nth(0)).toContainText('collision');
  await expect(staged.nth(1)).toContainText('strategy/winpct_fixture.csv');
  await expect(staged.nth(1)).toContainText('new');

  // Nothing is inserted until the admin says so.
  const files = page.locator('.card', { has: page.getByRole('columnheader', { name: 'Pinned by' }) });
  await expect(files.getByText(`data-${IMPORTED_DATA} or later`)).toHaveCount(0);

  await wizard.getByRole('button', { name: 'Insert 2 rows' }).click();
  await expect(wizard.getByText('Confirmed. 2 rows inserted.')).toBeVisible();

  const imported = files.locator('tbody tr', { hasText: `data-${IMPORTED_DATA} or later` });
  await expect(imported).toHaveCount(2);
  await expect(imported.filter({ hasText: 'strategy/winpct_fixture.csv' })).toContainText('winpct');
  await expect(imported.filter({ hasText: 'strategy/winpct.csv' })).toContainText('winpct');
  // The seeded version's copy of the re-cut file is still there: a changed
  // file is a new row, never an overwrite of one something may pin.
  await expect(files.locator('tbody tr', { hasText: 'strategy/winpct.csv' })).toHaveCount(2);
});
