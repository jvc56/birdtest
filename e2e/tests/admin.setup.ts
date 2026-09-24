import { test as setup, expect } from '@playwright/test';
import { ADMIN_STATE, env } from '../lib/env';

/**
 * The admin scripts/seed.py registered, confirmed and promoted, signed in
 * once through the login page. Every admin journey starts from this session
 * rather than logging in again: the login endpoint allows ten attempts a
 * minute per address, and the whole suite shares one.
 */
setup('the seeded admin signs in', async ({ page }) => {
  await page.goto('/login');
  await page.getByLabel('Username').fill(env.admin.username);
  await page.getByLabel('Password').fill(env.admin.password);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page).toHaveURL(/\/account$/);
  await expect(page.getByRole('link', { name: 'Admin', exact: true })).toBeVisible();
  await page.context().storageState({ path: ADMIN_STATE });
});
