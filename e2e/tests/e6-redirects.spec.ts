import { test, expect } from '@playwright/test';
import { confirmedUser } from '../lib/api';
import { uniqueUser } from '../lib/env';

/**
 * E-6: the pages a visitor may not see send them somewhere they may. A
 * signed-in non-admin asking for /admin lands on the home page; a visitor
 * with no session asking for /account lands on sign-in, which then returns
 * them to /account.
 */
test('E-6: a non-admin is sent away from /admin, and an anonymous visitor from /account', async ({
  page
}) => {
  await page.goto('/account');
  await expect(page).toHaveURL(/\/login\?next=%2Faccount$/);
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible();

  const user = uniqueUser();
  await confirmedUser(user);
  await page.getByLabel('Username').fill(user.username);
  await page.getByLabel('Password').fill(user.password);
  await page.getByRole('button', { name: 'Sign in' }).click();
  // Back to where the guard stopped them.
  await expect(page).toHaveURL(/\/account$/);
  await expect(page.getByText('contributor', { exact: true })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Admin', exact: true })).toHaveCount(0);

  for (const path of ['/admin', '/admin/jobs/new', '/admin/input-data']) {
    await page.goto(path);
    await expect(page).toHaveURL(/\/$/);
    await expect(page.getByRole('heading', { name: 'Crowdsourced word game analysis' })).toBeVisible();
  }
  // The admin API says no as well; the redirect is not the only thing between
  // a contributor and the admin endpoints.
  const status = await page.evaluate(() => fetch('/api/admin/input-data').then((r) => r.status));
  expect(status).toBe(403);
});
