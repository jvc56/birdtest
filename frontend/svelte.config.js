import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
export default {
  preprocess: vitePreprocess(),
  kit: {
    // The build is a static bundle served by Nginx alongside the Axum
    // container, so every route falls back to index.html and the app resolves
    // it client-side.
    adapter: adapter({ fallback: 'index.html', strict: false })
  }
};
