import { test, expect, devices, type Page } from '@playwright/test';
import { seededJob } from '../lib/api';

// A phone, in Chromium: Pixel 5 is 393 CSS pixels wide.
test.use({ ...devices['Pixel 5'] });

/** Nothing on the page is wider than the screen, so nothing scrolls sideways. */
async function expectNoSidewaysScroll(page: Page) {
  const { scrollWidth, innerWidth } = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    innerWidth: window.innerWidth
  }));
  expect(scrollWidth, 'page is wider than the viewport').toBeLessThanOrEqual(innerWidth);
}

/**
 * E-10: the site at phone width. One journey rather than all of them: a
 * visitor on a phone opens the site, goes to the job list, and reads a job's
 * page -- the densest public page -- without anything spilling sideways.
 */
test('E-10: a visitor on a phone reads the job list and a job page', async ({ page, request }) => {
  const job = await seededJob(request);
  expect(page.viewportSize()!.width).toBeLessThan(400);

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Crowdsourced word game analysis' })).toBeVisible();
  await expectNoSidewaysScroll(page);

  await page.getByRole('link', { name: 'Browse jobs' }).tap();
  await expect(page).toHaveURL(/\/jobs$/);
  await expect(page.getByRole('heading', { name: 'Jobs' })).toBeVisible();
  await expectNoSidewaysScroll(page);

  await page.locator(`a[href="/jobs/${job.id}"]`).tap();
  await expect(page.getByRole('heading', { name: 'Game pairs' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'SPRT' })).toBeVisible();
  await expectNoSidewaysScroll(page);

  // The four headline figures stack in one column rather than squeezing four
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
  for (const name of ['Jobs', 'Ratings', 'Contributors', 'Users', 'Sign in']) {
    const link = page.getByRole('banner').getByRole('link', { name, exact: true });
    await expect(link).toBeInViewport();
  }
});
