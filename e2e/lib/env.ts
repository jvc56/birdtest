import path from 'node:path';

/**
 * What e2e/run.sh exports. The defaults are run.sh's own values, so a
 * `npx playwright test` against a stack left up with `run.sh --keep` works
 * with only E2E_OUTBOX_DIR set.
 */
export const env = {
  baseURL: process.env.E2E_BASE_URL ?? 'http://localhost:5280',
  outbox: process.env.E2E_OUTBOX_DIR ?? '',
  admin: {
    username: process.env.E2E_ADMIN_USER ?? 'e2e-admin',
    email: process.env.E2E_ADMIN_EMAIL ?? 'e2e-admin@example.invalid',
    password: process.env.E2E_ADMIN_PASSWORD ?? 'e2e-Admin-passphrase-7431!'
  }
};

/** The seeded admin's signed-in browser state, written by admin.setup.ts. */
export const ADMIN_STATE = path.join(__dirname, '..', '.auth', 'admin.json');

/** The fixture tarball the suite seeds from, and the one E-3 imports. */
export const SEEDED_DATA = '20260101';
export const IMPORTED_DATA = '20260201';

/** A fresh identity per journey, so journeys never share an inbox or a name. */
export function uniqueUser(prefix = 'e2e') {
  const id = crypto.randomUUID();
  return {
    username: `${prefix}-${id.slice(0, 8)}`,
    email: `${prefix}-${id}@example.invalid`,
    // Random and unrelated to the name: the server scores a password against
    // the username and address too.
    password: `Pw-${crypto.randomUUID()}`
  };
}
