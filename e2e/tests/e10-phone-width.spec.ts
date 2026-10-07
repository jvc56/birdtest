import { test, expect, devices, type Locator, type Page } from '@playwright/test';
import { seededJob } from '../lib/api';

// A phone, in Chromium: Pixel 5 is 393 CSS pixels wide.
test.use({ ...devices['Pixel 5'] });

/** The widest names a ranking can hold: 32 wide characters, a tombstone, a pseudonym. */
const LONGEST = 'W'.repeat(32);
const TOMBSTONE = 'deleted-0b6f7c6e-3f1d-4c55-9e3a-2f4b8d6a1c90';

/**
 * Nothing on the page is wider than the screen, so nothing scrolls sideways.
 * Measured against the device's width, not `innerWidth`: a phone's browser
 * widens its layout viewport to fit content that overflows, so `innerWidth`
 * grew with the page and the comparison always passed -- while the header's
 * links ran 140 pixels off a Pixel 5 (thirty-first audit).
 */
async function expectNoSidewaysScroll(page: Page) {
  const screen = page.viewportSize()!.width;
  const { scrollWidth, innerWidth } = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    innerWidth: window.innerWidth
  }));
  expect(innerWidth, 'the layout viewport was widened to fit the page').toBeLessThanOrEqual(screen);
  expect(scrollWidth, 'page is wider than the screen').toBeLessThanOrEqual(screen);
}

/**
 * A table's box is no wider than itself: the table wraps rather than scrolls,
 * so the column a list is ranked by stays in view. The page's first table
 * card unless given one.
 */
async function expectTableFits(page: Page, box?: Locator) {
  const table = box ?? page.locator('.card', { has: page.locator('table') }).first();
  const { scrollWidth, clientWidth } = await table.evaluate((el) => ({
    scrollWidth: el.scrollWidth,
    clientWidth: el.clientWidth
  }));
  expect(scrollWidth, 'the ranking is wider than its box').toBeLessThanOrEqual(clientWidth);
}

/**
 * E-10: the site at phone width. One journey rather than all of them: a
 * visitor on a phone opens the site, goes to the job list, reads a job's
 * page -- the densest public page -- and the two rankings, without anything
 * spilling sideways.
 */
test('E-10: a visitor on a phone reads the job list, a job page and the rankings', async ({ page, request }) => {
  const job = await seededJob(request);
  expect(page.viewportSize()!.width).toBeLessThan(400);

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Crowdsourced Crossword Game Research' })).toBeVisible();
  await expectNoSidewaysScroll(page);

  await page.getByRole('link', { name: 'Browse jobs' }).tap();
  await expect(page).toHaveURL(/\/jobs$/);
  await expect(page.getByRole('heading', { name: 'Jobs' })).toBeVisible();
  await expectNoSidewaysScroll(page);

  await page.locator(`a[href="/jobs/${job.id}"]`).tap();
  await expect(page.getByRole('heading', { name: 'Game Pairs' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Significance Test' })).toBeVisible();
  await expectNoSidewaysScroll(page);

  // The three headline figures stack in one column rather than squeezing
  // three across: each is as wide as the one above it and sits below it.
  const cards = page.locator('.grid > .card');
  await expect(cards).toHaveCount(3);
  const boxes = await Promise.all((await cards.all()).map((card) => card.boundingBox()));
  for (let i = 1; i < boxes.length; i++) {
    expect(boxes[i]!.x).toBeCloseTo(boxes[0]!.x, 0);
    expect(boxes[i]!.width).toBeCloseTo(boxes[0]!.width, 0);
    expect(boxes[i]!.y).toBeGreaterThan(boxes[i - 1]!.y);
  }
  // And the header's links are all on screen and reachable.
  for (const name of ['Jobs', 'Ratings', 'Contributors', 'Users', 'Sign in']) {
    const link = page.getByRole('banner').getByRole('link', { name, exact: true });
    await expect(link).toBeInViewport();
  }
  // Every setting, the players' side by side: the tables wrap, or scroll
  // inside their card, but never widen the page.
  const jobSettings = page.locator('.card', { has: page.getByRole('heading', { name: 'Job settings' }) });
  await expect(jobSettings.getByText('Oldest MAGPIE')).toBeVisible();
  const settings = page.locator('.card', { has: page.getByRole('heading', { name: 'Player settings' }) });
  await settings.getByRole('button', { name: 'All settings' }).tap();
  await expect(settings.getByText('Movegen Margin')).toBeVisible();
  await expectNoSidewaysScroll(page);

  // The same page with the widest name as its creator and as a contributor:
  // "Created by" widened the page, and the contributors' count left its box.
  await page.route(new RegExp(`/api/jobs/${job.id}$`), async (route) => {
    const response = await route.fetch();
    const body = await response.json();
    body.job.created_by = LONGEST;
    body.workers = [
      { username: LONGEST, anon_id: null, tasks_completed: 123456789012, compute_seconds: 9.9e10 },
      ...(body.workers ?? [])
    ];
    await route.fulfill({ response, json: body });
  });
  // Its live pushes would put the real payload back.
  await page.route(/\/stream$/, (route) => route.abort());
  await page.reload();
  await expect(page.getByText(`Created by ${LONGEST}`)).toBeVisible();
  await expectNoSidewaysScroll(page);
  const contributors = page.locator('div.overflow-x-auto', {
    has: page.getByRole('columnheader', { name: 'Contributor' })
  });
  await expect(contributors.getByRole('cell', { name: LONGEST })).toBeVisible();
  await expectTableFits(page, contributors);
  await page.unrouteAll();

  // The contributors' ranking fits its box, the column it is ranked by on
  // screen: a pseudonym's sixteen characters pushed it out (thirty-second audit).
  // With a row in it: an empty table fits on any page.
  await page.getByRole('banner').getByRole('link', { name: 'Contributors', exact: true }).tap();
  await expect(page.getByRole('columnheader', { name: 'Compute time' })).toBeVisible();
  await expect(page.getByRole('cell', { name: /^Anonymous · [0-9a-f]{16}$/ }).first()).toBeVisible();
  await expectNoSidewaysScroll(page);
  await expectTableFits(page);
  await expect(page.getByRole('columnheader', { name: 'Compute time' })).toBeInViewport();

  // And with the widest names either ranking can hold, which the seed has not
  // registered: both lists answered as the server would with them.
  const now = new Date().toISOString();
  await page.route(/\/api\/workers\?/, (route) =>
    route.fulfill({
      json: {
        items: [
          // Longer than any fleet will run: the compute column at its widest.
          { user_id: null, username: LONGEST, anon_id: null, compute_seconds: 9.9e10,
            games_played: 123456789012, racks_analyzed: 123456789012,
            tasks_completed: 123456789012, last_seen_at: now },
          { user_id: null, username: TOMBSTONE, anon_id: null, compute_seconds: 1,
            games_played: 1, racks_analyzed: 1, tasks_completed: 1, last_seen_at: now },
          { user_id: null, username: null, anon_id: 'f'.repeat(16), compute_seconds: 1,
            games_played: 1, racks_analyzed: 1, tasks_completed: 1, last_seen_at: now }
        ],
        total: 3,
        page: 0,
        per_page: 50
      }
    })
  );
  await page.route(/\/api\/users\?/, (route) =>
    route.fulfill({
      json: {
        items: [
          { username: LONGEST, is_admin: true, created_at: now, tasks_completed: 123456789012 },
          { username: TOMBSTONE, is_admin: false, created_at: now, tasks_completed: 1 }
        ],
        total: 2,
        page: 0,
        per_page: 50
      }
    })
  );
  await page.reload();
  await expect(page.getByRole('cell', { name: LONGEST })).toBeVisible();
  await expectNoSidewaysScroll(page);
  await expectTableFits(page);
  await expect(page.getByRole('columnheader', { name: 'Compute time' })).toBeInViewport();
  // Ranked by another column, that column is the one shown beside the name.
  await page.getByRole('button', { name: 'Games' }).tap();
  await expect(page.getByRole('columnheader', { name: 'Games' })).toHaveAttribute('aria-sort', 'descending');
  await expect(page.getByRole('columnheader', { name: 'Compute time' })).toBeHidden();
  await expectNoSidewaysScroll(page);
  await expectTableFits(page);
  await expect(page.getByRole('columnheader', { name: 'Games' })).toBeInViewport();

  await page.getByRole('banner').getByRole('link', { name: 'Users', exact: true }).tap();
  await expect(page.getByRole('cell', { name: new RegExp(`^${LONGEST}`) })).toBeVisible();
  await expectNoSidewaysScroll(page);
  await expectTableFits(page);
  await expect(page.getByRole('columnheader', { name: 'Tasks completed' })).toBeInViewport();
});
