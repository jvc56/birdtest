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
 * card unless given one -- or any box, a stacked list's.
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
 * A column header is within the screen's width -- not pushed off its right
 * edge -- wherever it is down the page: on a phone the site's totals card
 * stacks above the ranking and pushes it below the fold, which is not what
 * this checks.
 */
async function expectAcrossScreen(page: Page, header: Locator) {
  await expect(header).toBeVisible();
  const box = (await header.boundingBox())!;
  expect(box.x, 'the column starts off the screen').toBeGreaterThanOrEqual(0);
  expect(box.x + box.width, 'the column runs off the screen').toBeLessThanOrEqual(page.viewportSize()!.width);
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
  // The header is two rows: the sign-in links share the brand's, and the
  // page's links have the second. A third row of its own pushed the heading
  // half a screen down.
  const banner = page.getByRole('banner');
  const brand = (await banner.getByRole('link', { name: 'birdtest', exact: true }).boundingBox())!;
  const signIn = (await banner.getByRole('link', { name: 'Sign in', exact: true }).boundingBox())!;
  expect(Math.abs(signIn.y + signIn.height / 2 - (brand.y + brand.height / 2))).toBeLessThan(brand.height);
  // contribute.txt's settings stack on a phone, every description on screen:
  // as a table, the last column ran off the side.
  const contributeSettings = page.getByTestId('contribute-settings');
  await expect(contributeSettings.getByText('idlewait', { exact: true })).toBeVisible();
  await expectTableFits(page, contributeSettings);
  for (const description of await contributeSettings.locator('dd').all()) {
    const box = (await description.boundingBox())!;
    expect(box.x + box.width).toBeLessThanOrEqual(page.viewportSize()!.width);
  }

  await page.getByRole('link', { name: 'Browse jobs' }).tap();
  await expect(page).toHaveURL(/\/jobs$/);
  await expect(page.getByRole('heading', { name: 'Jobs' })).toBeVisible();
  await expectNoSidewaysScroll(page);

  await page.locator(`a[href="/jobs/${job.id}"]`).tap();
  await expect(page.getByRole('heading', { level: 1, name: job.name, exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Significance Test' })).toBeVisible();
  await expectNoSidewaysScroll(page);

  // The four headline figures stack in one column rather than squeezing
  // across: each is as wide as the one above it and sits below it.
  const cards = page.locator('.grid > .card');
  await expect(cards).toHaveCount(4);
  const boxes = await Promise.all((await cards.all()).map((card) => card.boundingBox()));
  for (let i = 1; i < boxes.length; i++) {
    expect(boxes[i]!.x).toBeCloseTo(boxes[0]!.x, 0);
    expect(boxes[i]!.width).toBeCloseTo(boxes[0]!.width, 0);
    expect(boxes[i]!.y).toBeGreaterThan(boxes[i - 1]!.y);
  }
  // And the header's links are all on screen and reachable.
  for (const name of ['Jobs', 'Ratings', 'Contributions', 'Users', 'Sign in']) {
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
  await page.getByRole('banner').getByRole('link', { name: 'Contributions', exact: true }).tap();
  // The ranking's own headers: the site's totals and a contributor's
  // breakdown are tables under the same names.
  const ranking = page.getByTestId('contributor-ranking');
  const header = (name: string) =>
    ranking.locator(':scope > table > thead').getByRole('columnheader', { name, exact: true });
  await expect(header('Movegens')).toBeVisible();
  await expect(page.getByRole('cell', { name: /^Anonymous · [0-9a-f]{16}$/ }).first()).toBeVisible();
  await expectNoSidewaysScroll(page);
  await expectTableFits(page, ranking);
  await expectAcrossScreen(page, header('Movegens'));

  // And with the widest names either ranking can hold, which the seed has not
  // registered: both lists answered as the server would with them.
  const now = new Date().toISOString();
  await page.route(/\/api\/workers\?/, (route) =>
    route.fulfill({
      json: {
        items: [
          // Longer than any fleet will run: every column at its widest.
          { user_id: '00000000-0000-4000-8000-000000000001', username: LONGEST, anon_id: null,
            compute_seconds: 9.9e10, movegens: 9.2e18, tasks_completed: 123456789012, last_seen_at: now },
          { user_id: null, username: TOMBSTONE, anon_id: null, compute_seconds: 1,
            movegens: 1, tasks_completed: 1, last_seen_at: now },
          { user_id: null, username: null, anon_id: 'f'.repeat(16), compute_seconds: 1,
            movegens: 1, tasks_completed: 1, last_seen_at: now }
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
  // Every type at a BIGINT's widest, the site's and the contributor's, and a
  // century of compute and a trillion tasks beside.
  const share = { movegens: 9.2e18, compute_seconds: 3.2e9, tasks: 999999999999 };
  const widest = { opening_rack: share, games: share, game_pairs: share, leave_generation: share };
  await page.route(/\/api\/workers\/(user\/[^/]+\/)?movegens$/, (route) => route.fulfill({ json: widest }));
  await page.reload();
  await expect(page.getByRole('cell', { name: LONGEST })).toBeVisible();
  const site = page.getByTestId('site-movegens');
  await expect(site.locator('dd').first()).toHaveText('36,800,000,000,000,000,000');
  await expect(site.locator('[data-figure="movegens"]')).toHaveText(Array(4).fill('9,200,000,000,000,000,000'));
  await expectNoSidewaysScroll(page);
  // The site's table wraps inside its card rather than widening it.
  await expectTableFits(page, site);
  await expectTableFits(page, ranking);
  await expectAcrossScreen(page, header('Movegens'));
  // A contributor's work by job type opens under their row, inside the
  // ranking's box, and the page still does not scroll sideways.
  await page.getByRole('button', { name: LONGEST }).tap();
  const breakdown = page.getByTestId('contributor-movegens');
  await expect(breakdown.locator('[data-figure="movegens"]')).toHaveText(Array(4).fill('9,200,000,000,000,000,000'));
  await expectNoSidewaysScroll(page);
  await expectTableFits(page, ranking);
  for (const cell of await breakdown.locator('[data-figure]').all()) {
    const box = (await cell.boundingBox())!;
    expect(box.x + box.width).toBeLessThanOrEqual(page.viewportSize()!.width);
  }
  // Ranked by another column, that column is the one shown beside the name.
  await page.getByRole('button', { name: 'Compute time' }).tap();
  await expect(header('Compute time')).toHaveAttribute('aria-sort', 'descending');
  await expect(header('Movegens')).toBeHidden();
  await expectNoSidewaysScroll(page);
  await expectTableFits(page, ranking);
  await expectAcrossScreen(page, header('Compute time'));

  await page.getByRole('banner').getByRole('link', { name: 'Users', exact: true }).tap();
  await expect(page.getByRole('cell', { name: new RegExp(`^${LONGEST}`) })).toBeVisible();
  await expectNoSidewaysScroll(page);
  await expectTableFits(page);
  await expectAcrossScreen(page, page.getByRole('columnheader', { name: 'Tasks completed' }));
});

/**
 * E-10b: a rating pool's cross table at phone width. Six configs with long
 * names make it several screens wide: it scrolls sideways inside its card
 * while the page does not, and the config names, in a sticky first column,
 * stay in view as it does. The pool is served from a route: what is under
 * test is the layout, not a fit.
 */
test('E-10b: a rating pool cross table scrolls inside its card on a phone', async ({ page }) => {
  const ids = Array.from({ length: 6 }, (_, i) => `00000000-0000-4000-8000-00000000000${i}`);
  const name = (i: number) => `simmer-CSW24-${i + 1}ply-equity-${'x'.repeat(12)}`;
  const ratings = ids.map((id, i) => ({
    player_config_id: id,
    name: name(i),
    rating: 2000 - 60 * i,
    stderr: i === 0 ? 0 : 25,
    pairs_played: 500,
    connected_to_anchor: true,
    is_anchor: i === 0
  }));
  // Every head-to-head from both sides, the better config scoring 3 points a
  // rung more.
  const head_to_heads = ids.flatMap((row, i) =>
    ids
      .map((col, j) => ({ col, j }))
      .filter(({ j }) => j !== i)
      .map(({ col, j }) => ({
        row,
        col,
        pairs: 100,
        actual: 0.5 + 0.03 * (j - i),
        predicted: 0.5 + 0.025 * (j - i),
        stderr: 0.03,
        spread: 4.5 * (j - i)
      }))
  );
  await page.route(/\/api\/rating-pools\/[^/?]+$/, (route) =>
    route.fulfill({
      json: {
        id: 'pool',
        name: 'phone pool',
        variant: 'classic',
        letter_distribution: 'english',
        layout: 'standard15',
        anchor_player_config_id: ids[0],
        anchor_rating: 2000,
        members: ratings.map((r) => ({ player_config_id: r.player_config_id, name: r.name })),
        run: {
          id: 'run',
          computed_at: new Date().toISOString(),
          trigger: 'evidence',
          iterations: 5,
          converged: true,
          pairs_used: 1500,
          jobs_used: 15
        },
        ratings,
        head_to_heads
      }
    })
  );

  await page.goto('/ratings/00000000-0000-4000-8000-0000000000aa');
  const cross = page.getByTestId('cross-table');
  await expect(cross.locator('tbody tr')).toHaveCount(6);
  await expectNoSidewaysScroll(page);

  // The table's own box scrolls: it is wider than the box that holds it.
  const box = cross.locator('xpath=..');
  const { scrollWidth, clientWidth } = await box.evaluate((el) => ({
    scrollWidth: el.scrollWidth,
    clientWidth: el.clientWidth
  }));
  expect(scrollWidth, 'the cross table fits a phone, so this proves nothing').toBeGreaterThan(clientWidth);

  // Scrolled to its far end -- the ratings column -- the names are still there.
  await cross.scrollIntoViewIfNeeded();
  await box.evaluate((el) => (el.scrollLeft = el.scrollWidth));
  const firstName = cross.locator('tbody th').first();
  await expect(firstName).toBeInViewport();
  await expect(cross.locator('tbody tr').first().locator('td').last()).toBeInViewport();
  const [nameBox, scrolled] = [(await firstName.boundingBox())!, (await box.boundingBox())!];
  expect(Math.abs(nameBox.x - scrolled.x)).toBeLessThan(2);
  await expectNoSidewaysScroll(page);
});
