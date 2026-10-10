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
  await expect(page.getByRole('heading', { name: 'Crowdsourced Crossword Game Research' })).toBeVisible();
  // Signed out: the header offers an account, and no admin link.
  await expect(page.getByRole('link', { name: 'Sign in' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Admin', exact: true })).toHaveCount(0);

  await page.getByRole('link', { name: 'Browse jobs' }).click();
  await expect(page).toHaveURL(/\/jobs$/);
  await expect(page.getByRole('heading', { name: 'Jobs' })).toBeVisible();
  const row = page.locator('tr', { has: page.locator(`a[href="/jobs/${job.id}"]`) });
  await expect(row).toContainText('Game Pairs');
  // Progress in the job's own unit (the list said `units` until the
  // fourteenth audit).
  await expect(row).toContainText(/\d[\d,]* \/ [\d,]+ pairs/);

  // Listed, and titled on its page, by its name: a pairs job between two
  // configs is named for them ("static-equity vs static-score").
  await row.getByRole('link', { name: job.name, exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/jobs/${job.id}$`));
  await expect(page.getByRole('heading', { level: 1, name: job.name, exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Significance Test' })).toBeVisible();
  // The status on a row of its own, saying what it means for this job unless
  // it is active (the badge says that plainly), then the headline figures.
  const status = page.getByTestId('job-status');
  await expect(status).toContainText(/\b(active|inactive|completed)\b/);
  if (/\bactive\b/.test(await status.innerText())) {
    await expect(page.getByTestId('job-status-context')).toHaveCount(0);
  } else {
    await expect(page.getByTestId('job-status-context')).toContainText(/Paused|Finished/);
  }
  await expect(page.locator('.grid > .card > p:first-child')).toHaveText([
    'Allocation', 'Tasks completed', 'Active contributors', 'Estimated time left'
  ]);
  // Anonymous workers are shown by pseudonym, never by the UUID that is
  // their credential.
  const contributors = page.locator('.card', { has: page.getByRole('heading', { name: 'Contributors' }) });
  await expect(contributors.getByText(/^Anonymous · [0-9a-f]{16}$/).first()).toBeVisible();

  await page.getByRole('link', { name: 'Contributions' }).click();
  await expect(page).toHaveURL(/\/workers$/);
  await expect(page.getByRole('heading', { name: 'Contributions', level: 1 })).toBeVisible();
  // The ranking's own rows and headers: the site's totals and a
  // contributor's breakdown are tables too, under the same headers.
  const ranking = page.getByTestId('contributor-ranking');
  const header = (name: string) =>
    ranking.locator(':scope > table > thead').getByRole('columnheader', { name, exact: true });
  const leaders = ranking.locator(':scope > table > tbody > tr');
  await expect(leaders.first()).toContainText(/Anonymous · [0-9a-f]{16}/);
  // The site's totals -- movegens, compute time, tasks -- then the same by
  // job type, every type listed.
  const site = page.getByTestId('site-movegens');
  await expect(site.locator('dt')).toHaveText(['Movegens', 'Compute time', 'Tasks']);
  await expect(site.locator('dd').first()).toHaveText(/^[1-9][\d,]*$/);
  await expect(site.getByRole('rowheader')).toHaveText([
    'Opening Rack Analysis', 'Games', 'Game Pairs', 'Leave Generation'
  ]);
  // The seeded pairs job has results, so its type has movegens and tasks.
  const pairs = site.locator('tr[data-type="game_pairs"]');
  await expect(pairs.locator('[data-figure="movegens"]')).toHaveText(/^[1-9][\d,]*$/);
  await expect(pairs.locator('[data-figure="tasks"]')).toHaveText(/^[1-9][\d,]*$/);
  // A contributor's own, under their row when their name is chosen, and
  // folded away again. Held by name, not by place: the list reads itself
  // again every 30 seconds, and the busy workers can swap places in between.
  const label = (await leaders.first().getByRole('button').innerText()).replace(/^[▸▾]\s*/, '').trim();
  const first = page.getByRole('button', { name: label, exact: true });
  await expect(first).toHaveAttribute('aria-expanded', 'false');
  await first.click();
  await expect(first).toHaveAttribute('aria-expanded', 'true');
  const breakdown = page.getByTestId('contributor-movegens');
  await expect(breakdown.getByRole('rowheader')).toHaveText([
    'Opening Rack Analysis', 'Games', 'Game Pairs', 'Leave Generation'
  ]);
  // Whichever jobs it worked on, it did some work.
  await expect(breakdown.locator('[data-figure="movegens"]').filter({ hasText: /^[1-9][\d,]*$/ }).first()).toBeVisible();
  await expect(breakdown.locator('[data-figure="tasks"]').filter({ hasText: /^[1-9][\d,]*$/ }).first()).toBeVisible();
  await first.click();
  await expect(breakdown).toHaveCount(0);
  // Ranked by movegens unless another column is chosen.
  await expect(header('Movegens')).toHaveAttribute('aria-sort', 'descending');
  // Chosen: tasks completed, most first -- once the reordered page is in.
  // Exactly: a contributor's name is a button too.
  await page.getByRole('button', { name: 'Tasks', exact: true }).click();
  await expect(header('Tasks')).toHaveAttribute('aria-sort', 'descending');
  const counts = async () =>
    (await leaders.locator('td[data-column="tasks"]').allInnerTexts()).map((text) =>
      Number(text.replace(/,/g, ''))
    );
  await expect
    .poll(async () => {
      const seen = await counts();
      return seen.length > 0 && seen.join() === [...seen].sort((a, b) => b - a).join();
    })
    .toBe(true);
  expect((await counts()).every((count) => count > 0)).toBe(true);
});
