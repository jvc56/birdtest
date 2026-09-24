import { defineConfig, devices } from '@playwright/test';
import { env } from './lib/env';

/**
 * Tier 5 (TESTING.md section 5). Expects the stack e2e/run.sh brings up; run
 * the suite through that script, which also seeds the admin these journeys
 * sign in as and tears everything down afterwards.
 *
 * One worker, no retries. The journeys share one stack and one allocation
 * budget, and a journey that only passes on its second try is a failure this
 * tier exists to report.
 */
export default defineConfig({
  testDir: './tests',
  // Per test, and far below the ten-minute ceiling: the slowest journey waits
  // for fake workers to finish three small jobs.
  timeout: 5 * 60_000,
  expect: { timeout: 15_000 },
  globalTimeout: 15 * 60_000,
  workers: 1,
  fullyParallel: false,
  retries: 0,
  forbidOnly: !!process.env.CI,
  reporter: [['list'], ['html', { open: 'never', outputFolder: 'playwright-report' }]],
  use: {
    baseURL: env.baseURL,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure'
  },
  projects: [
    { name: 'setup', testMatch: /.*\.setup\.ts/ },
    {
      name: 'journeys',
      dependencies: ['setup'],
      testMatch: /.*\.spec\.ts/,
      use: { ...devices['Desktop Chrome'] }
    }
  ]
});
