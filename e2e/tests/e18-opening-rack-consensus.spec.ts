import { test, expect } from '@playwright/test';
import { AdminApi } from '../lib/api';
import { ADMIN_STATE, SEEDED_DATA } from '../lib/env';

/**
 * E-18: an admin changes an opening-rack job's consensus settings after
 * creating it, from the job's admin page, and the job's settings say so on
 * its public page. Analysing a rack more than once needs MAGPIE 0.1.5, so
 * the job's floor is raised first, in its Controls. The job is left
 * inactive, so nothing analyses its racks meanwhile and the edit has no
 * rack to restate.
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
    rack_size: 7,
    min_magpie_version: '0.1.2'
  });
});

test.afterAll(async () => {
  await api.dispose();
});

test("E-18: an opening-rack job's consensus can be changed after it is created", async ({ page }) => {
  await page.goto(`/admin/jobs/${jobId}`);
  const card = page.getByTestId('consensus-editor');
  await expect(card.getByRole('heading', { name: 'Consensus' })).toBeVisible();
  const max = card.getByLabel('Max Passes Per Rack');
  const margin = card.getByLabel('Win By');
  const top = card.getByLabel('Reanalyse Top Moves');
  await expect(max).toHaveValue('1');
  await expect(margin).toHaveValue('2');
  await expect(top).toHaveValue('10');
  // One analysis per rack seeks no agreement and reanalyses nothing.
  await expect(margin).toBeDisabled();
  await expect(top).toBeDisabled();
  await expect(card.getByRole('button', { name: 'Save' })).toBeDisabled();

  await max.fill('3');
  await top.fill('1');
  await expect(card.getByText('The moves reanalysed must be a whole number from 2 to 100.')).toBeVisible();
  await top.fill('5');
  // Below the floor that reads a reanalysis's moves: raised in the Controls first.
  await expect(card.getByText(/needs MAGPIE 0\.1\.5 or later/)).toBeVisible();
  await expect(card.getByRole('button', { name: 'Save' })).toBeDisabled();
  await page.getByTestId('job-min-magpie').getByLabel('Oldest MAGPIE').fill('0.1.5');
  await page.getByTestId('job-min-magpie').getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText(/need this MAGPIE version or newer/)).toBeVisible();

  await expect(card.getByText(/needs MAGPIE 0\.1\.5 or later/)).toBeHidden();
  await max.fill('3');
  await margin.fill('2');
  await top.fill('5');
  await card.getByRole('button', { name: 'Save' }).click();
  await expect(card.getByTestId('consensus-notice')).toHaveText('Saved: 0 racks unsettled.');

  await page.goto(`/jobs/${jobId}`);
  const settings = page.getByTestId('job-settings');
  const row = (label: string) =>
    settings.getByRole('row').filter({ has: page.getByRole('cell', { name: label, exact: true }) });
  await expect(row('Max Passes Per Rack').getByRole('cell')).toHaveText(['Max Passes Per Rack', '3']);
  await expect(row('Win By').getByRole('cell')).toHaveText(['Win By', '2']);
  await expect(row('Reanalyse Top Moves').getByRole('cell')).toHaveText(['Reanalyse Top Moves', '5']);
  await expect(row('Oldest MAGPIE').getByRole('cell')).toHaveText(['Oldest MAGPIE', '0.1.5']);
});

test("E-18: an opening-rack job made on the form has the Racks Per Task it was given", async ({ page }) => {
  const suffix = crypto.randomUUID().slice(0, 8);
  const configId = await api.createSimConfig(`e18-form-${suffix}`);
  await page.goto('/admin/jobs/new');
  await page.getByLabel('Job Title').fill(`e18 form ${suffix}`);
  await page.getByLabel('Job Type').selectOption({ label: 'Opening Rack Analysis' });
  const letterdist = page.getByLabel('Letter Distribution');
  const fixtureBag = letterdist.locator('option', { hasText: `english_fixture (${SEEDED_DATA},` });
  await letterdist.selectOption((await fixtureBag.getAttribute('value'))!);
  const layout = page.getByLabel('Board', { exact: true });
  const board = layout.locator('option', { hasText: `standard15 (${SEEDED_DATA},` });
  await layout.selectOption((await board.getAttribute('value'))!);
  await page.getByLabel('Player', { exact: true }).selectOption(configId);
  // The server's default until changed.
  const racks = page.getByLabel('Racks Per Task');
  await expect(racks).toHaveValue('500');
  await racks.fill('7');
  await page.getByRole('button', { name: 'Create job' }).click();

  await expect(page).toHaveURL(/\/admin\/jobs\/[0-9a-f-]{36}$/);
  const created = page.url().split('/').pop()!;
  await page.goto(`/jobs/${created}`);
  const row = page
    .getByTestId('job-settings')
    .getByRole('row')
    .filter({ has: page.getByRole('cell', { name: 'Racks Per Task', exact: true }) });
  await expect(row.getByRole('cell')).toHaveText(['Racks Per Task', '7']);
});
