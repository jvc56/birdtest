import { test, expect } from '@playwright/test';
import { AdminApi } from '../lib/api';
import { ADMIN_STATE } from '../lib/env';

/**
 * E-18: an admin changes an opening-rack job's consensus settings after
 * creating it, from the job's admin page, and the job's settings say so on
 * its public page. The job is left inactive, so nothing analyses its racks
 * meanwhile and the edit has no rack to restate.
 */
test.use({ storageState: ADMIN_STATE });

let api: AdminApi;
let jobId: string;

test.beforeAll(async () => {
  api = await AdminApi.open();
  const suffix = crypto.randomUUID().slice(0, 8);
  jobId = await api.inactiveJob({
    name: `e18 consensus ${suffix}`,
    job_type: 'opening_rack',
    player_config_id: await api.createSimConfig(`e18-sim-${suffix}`),
    racks_per_batch: 5,
    rack_size: 7
  });
});

test.afterAll(async () => {
  await api.dispose();
});

test("E-18: an opening-rack job's consensus can be changed after it is created", async ({ page }) => {
  await page.goto(`/admin/jobs/${jobId}`);
  const card = page.getByTestId('consensus-editor');
  await expect(card.getByRole('heading', { name: 'Consensus' })).toBeVisible();
  const min = card.getByLabel('Minimum Analyses Per Rack');
  const max = card.getByLabel('Maximum Analyses Per Rack');
  const pct = card.getByLabel('Consensus %');
  await expect(min).toHaveValue('1');
  await expect(max).toHaveValue('1');
  // One analysis per rack seeks no agreement: the share waits for a maximum.
  await expect(pct).toBeDisabled();
  await expect(card.getByRole('button', { name: 'Save' })).toBeDisabled();

  await min.fill('2');
  await max.fill('1');
  await expect(card.getByText('The most analyses must be at least the fewest')).toBeVisible();
  await max.fill('3');
  await pct.fill('80');
  await card.getByRole('button', { name: 'Save' }).click();
  await expect(card.getByTestId('consensus-notice')).toHaveText('Saved: 0 racks unsettled.');

  await page.goto(`/jobs/${jobId}`);
  const settings = page.getByTestId('job-settings');
  const row = (label: string) =>
    settings.getByRole('row').filter({ has: page.getByRole('cell', { name: label, exact: true }) });
  await expect(row('Minimum Analyses Per Rack').getByRole('cell')).toHaveText(['Minimum Analyses Per Rack', '2']);
  await expect(row('Maximum Analyses Per Rack').getByRole('cell')).toHaveText(['Maximum Analyses Per Rack', '3']);
  await expect(row('Consensus %').getByRole('cell')).toHaveText(['Consensus %', '80%']);
});
