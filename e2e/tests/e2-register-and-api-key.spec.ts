import { test, expect } from '@playwright/test';
import { uniqueUser } from '../lib/env';
import { CONFIRM_SUBJECT, linkIn, waitForMail } from '../lib/mail';

/**
 * E-2: register, confirm the address from the emailed link, sign in, generate
 * an API key, see it exactly once, and deactivate it. The code comes out of
 * the outbox file addressed to this journey's own new address.
 */
test('E-2: a new user registers, confirms, signs in, and makes and deactivates an API key', async ({
  page,
  request
}) => {
  const user = uniqueUser();

  await page.goto('/register');
  await page.getByLabel('Username').fill(user.username);
  await page.getByLabel('Email').fill(user.email);
  await page.getByLabel('Password').fill(user.password);
  await page.getByRole('button', { name: 'Register' }).click();
  await expect(page).toHaveURL(/\/register\/check-email$/);
  await expect(page.getByRole('heading', { name: 'Check your email' })).toBeVisible();

  // Not confirmed yet, so signing in is refused.
  await page.goto('/login');
  await page.getByLabel('Username').fill(user.username);
  await page.getByLabel('Password').fill(user.password);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page.locator('.field-error')).toContainText(/confirm your email/i);

  const mail = await waitForMail(user.email, CONFIRM_SUBJECT);
  expect(mail.to).toBe(user.email);
  await page.goto(linkIn(mail, '/confirm-email'));
  await expect(page.getByRole('heading', { name: 'Email confirmed' })).toBeVisible();
  // The confirmation page moves on to sign-in by itself.
  await expect(page).toHaveURL(/\/login$/);

  await page.getByLabel('Username').fill(user.username);
  await page.getByLabel('Password').fill(user.password);
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page).toHaveURL(/\/account$/);
  await expect(page.getByRole('heading', { name: 'Account' })).toBeVisible();
  await expect(page.getByText(user.email)).toBeVisible();
  await expect(page.getByText('contributor', { exact: true })).toBeVisible();

  // Generate a key: shown once, with the warning that it will not be again.
  await page.getByPlaceholder('Label (optional)').fill('laptop');
  await page.getByRole('button', { name: 'Generate key' }).click();
  await expect(page.getByText('Copy this key now — it is not shown again.')).toBeVisible();
  const key = (await page.locator('code.break-all').innerText()).trim();
  expect(key.length).toBeGreaterThanOrEqual(32);
  const row = page.locator('tbody tr', { hasText: 'laptop' });
  await expect(row).toContainText('active');

  // ...and never again: not after a reload, and not from the list endpoint.
  await page.reload();
  await expect(row).toContainText('active');
  await expect(page.getByText('Copy this key now')).toHaveCount(0);
  expect(await page.content()).not.toContain(key);
  const listed = await page.evaluate(() => fetch('/api/me/api-keys').then((r) => r.text()));
  expect(listed).toContain('laptop');
  expect(listed).not.toContain(key);

  // Deactivate it, and a worker presenting it is no longer let in.
  await row.getByRole('button', { name: 'Deactivate' }).click();
  await expect(row.locator('td').nth(3)).toHaveText('inactive');
  await expect(row.getByRole('button', { name: 'Activate', exact: true })).toBeVisible();
  const claim = await request.post('/api/worker/task', {
    headers: { Authorization: `Bearer ${key}` },
    data: { magpie_version: '99.0.0', unsupported_jobs: [] }
  });
  expect(claim.status()).toBe(401);
  expect((await claim.json()).message).toBe('unknown or inactive API key');
});
