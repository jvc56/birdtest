import { test, expect } from '@playwright/test';
import { seededJob, waitForResults } from '../lib/api';

/**
 * E-1: an anonymous visitor browses the landing page, the job list, a job's
 * page and the contributor leaderboard -- the public face of the site, with
 * no session at all.
 */
test('E-1: an anonymous visitor browses the landing page, jobs, a job and the leaderboard', async ({
  page,
  request
}) => {
  const job = await seededJob(request);
  // The leaderboard lists workers that have finished a task, so wait for the
  // fake workers' first result rather than racing them.
  await waitForResults(request, job.id);

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Crowdsourced word game analysis' })).toBeVisible();
  // Signed out: the header offers an account, and no admin link.
  await expect(page.getByRole('link', { name: 'Sign in' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Admin', exact: true })).toHaveCount(0);

  await page.getByRole('link', { name: 'Browse jobs' }).click();
  await expect(page).toHaveURL(/\/jobs$/);
  await expect(page.getByRole('heading', { name: 'Jobs' })).toBeVisible();
  const row = page.locator('tr', { has: page.locator(`a[href="/jobs/${job.id}"]`) });
  await expect(row).toContainText('Game pairs');
  await expect(row).toContainText('units');

  await row.getByRole('link', { name: 'Game pairs' }).click();
  await expect(page).toHaveURL(new RegExp(`/jobs/${job.id}$`));
  await expect(page.getByRole('heading', { name: 'Game pairs' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'SPRT' })).toBeVisible();
  // Anonymous workers are shown by pseudonym, never by the UUID that is
  // their credential.
  const contributors = page.locator('.card', { has: page.getByRole('heading', { name: 'Contributors' }) });
  await expect(contributors.getByText(/^Anonymous · [0-9a-f]{8}$/).first()).toBeVisible();

  await page.getByRole('link', { name: 'Contributors' }).click();
  await expect(page).toHaveURL(/\/workers$/);
  await expect(page.getByRole('heading', { name: 'Contributors' })).toBeVisible();
  const leaders = page.locator('tbody tr');
  await expect(leaders.first()).toContainText(/Anonymous · [0-9a-f]{8}/);
  // Ranked by tasks completed, most first.
  const counts = (await leaders.locator('td:last-child').allInnerTexts()).map((text) =>
    Number(text.replace(/,/g, ''))
  );
  expect(counts.length).toBeGreaterThan(0);
  expect(counts.every((count) => count > 0)).toBe(true);
  expect([...counts].sort((a, b) => b - a)).toEqual(counts);
});
