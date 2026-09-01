// The build is a static SPA served by Nginx: there is no Node server to render
// on, and every page's data comes from the API at runtime.
export const ssr = false;
export const prerender = false;
export const trailingSlash = 'never';
