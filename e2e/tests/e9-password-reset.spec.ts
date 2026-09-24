import { test, expect } from '@playwright/test';
import { confirmedUser } from '../lib/api';
import { uniqueUser } from '../lib/env';
import { linkIn, RESET_SUBJECT, waitForMail } from '../lib/mail';

/**
 * E-9: a user who has forgotten their password asks for a reset, follows the
 * emailed link, chooses a new one, and signs in with it; the old one no longer
 * works. The link is read from the outbox file addressed to this user.
 */
test('E-9: a user resets a forgotten password from the emailed link', async ({ page }) => {
  const user = uniqueUser();
  await confirmedUser(user);
  const newPassword = `Pw-${crypto.randomUUID()}`;

  await page.goto('/login');
  await page.getByRole('link', { name: 'Forgot password?' }).click();
  await expect(page).toHaveURL(/\/reset-password$/);
  await page.getByLabel('Email').fill(user.email);
  await page.getByRole('button', { name: 'Send reset link' }).click();
  await expect(page.getByText('If that address has a confirmed account, a reset link is on its way.')).toBeVisible();

  const mail = await waitForMail(user.email, RESET_SUBJECT);
  await page.goto(linkIn(mail, '/reset-password/confirm'));
  await expect(page.getByRole('heading', { name: 'Choose a new password' })).toBeVisible();
  await page.getByLabel('New password').fill(newPassword);
  await page.getByRole('button', { name: 'Set new password' }).click();
  await expect(page).toHaveURL(/\/login$/);

  // The old password is refused...
  await page.getByLabel('Username').fill(user.username);
  await page.getByLabel('Password').fill(user.password);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.locator('.field-error')).toBeVisible();
  await expect(page).toHaveURL(/\/login$/);

  // ...and the new one lets them in.
  await page.getByLabel('Password').fill(newPassword);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page).toHaveURL(/\/account$/);
  await expect(page.getByText(user.email)).toBeVisible();

  // A reset link works once.
  await page.goto(linkIn(mail, '/reset-password/confirm'));
  await page.getByLabel('New password').fill(`Pw-${crypto.randomUUID()}`);
  await page.getByRole('button', { name: 'Set new password' }).click();
  await expect(page.locator('.field-error')).toBeVisible();
  await expect(page).toHaveURL(/\/reset-password\/confirm\?token=/);
});
