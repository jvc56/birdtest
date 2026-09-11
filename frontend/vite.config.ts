import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    port: 5173,
    // The dev server proxies the API, so the frontend needs no API base-URL
    // configuration of its own: it always talks to same-origin /api.
    proxy: {
      '/api': {
        target: process.env.BIRDTEST_API ?? 'http://localhost:8080',
        changeOrigin: true
      }
    }
  }
});
