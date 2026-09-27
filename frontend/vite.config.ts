import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    port: 5173,
    // The dev server proxies the API, so the frontend needs no API base-URL
    // configuration of its own: it always talks to same-origin /api.
    proxy: {
      '/api': {
        // 127.0.0.1, not localhost: the stack publishes its ports on IPv4
        // loopback only, and where localhost resolves to ::1 first Node 18
        // does not fall back.
        target: process.env.BIRDTEST_API ?? 'http://127.0.0.1:8080',
        changeOrigin: true
      }
    }
  },
  // Tier 1F (TESTING.md): pure TypeScript, no browser. The globals a module
  // touches — fetch, document.cookie, EventSource — are stubbed per test, so
  // the plain Node environment is enough and no DOM library is needed.
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node',
    pool: 'threads',
    poolOptions: { threads: { maxThreads: 2, minThreads: 1 } }
  }
});
