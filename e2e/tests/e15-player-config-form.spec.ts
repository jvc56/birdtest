import { test, expect } from '@playwright/test';
import { ADMIN_STATE } from '../lib/env';

test.use({ storageState: ADMIN_STATE });

/**
 * E-15: the player-config form holds back a stopping percentage the server
 * would refuse. It must be strictly between 0 and 100, and the field's
 * `min` and `max` let both ends through, so the refusal came back from the
 * server after the submit.
 */
test('E-15: the player-config form refuses a stopping % of 0 or 100', async ({ page }) => {
  await page.goto('/admin/player-configs/new');
  await page.getByLabel('Name').fill(`e15-${crypto.randomUUID().slice(0, 8)}`);
  await page.getByLabel('Simming player').check();
  const stopping = page.getByLabel('Stopping % (-sc)');
  const create = page.getByRole('button', { name: 'Create' });
  const valid = () => stopping.evaluate((el) => (el as HTMLInputElement).checkValidity());

  for (const bad of ['100', '0']) {
    await stopping.fill(bad);
    expect(await valid(), bad).toBe(false);
    await create.click();
    await expect(page).toHaveURL(/\/admin\/player-configs\/new$/);
  }
  await stopping.fill('99.5');
  expect(await valid()).toBe(true);
});
