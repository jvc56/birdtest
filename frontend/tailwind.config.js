/** @type {import('tailwindcss').Config} */
export default {
  // Dark mode only: `dark` is always on the root element, so there is no
  // toggle and no second palette to keep consistent.
  darkMode: 'class',
  content: ['./src/**/*.{html,js,svelte,ts}'],
  theme: {
    extend: {
      colors: {
        border: 'hsl(217 19% 22%)',
        input: 'hsl(217 19% 22%)',
        ring: 'hsl(199 89% 48%)',
        background: 'hsl(222 24% 8%)',
        foreground: 'hsl(210 20% 92%)',
        muted: { DEFAULT: 'hsl(217 19% 14%)', foreground: 'hsl(215 14% 60%)' },
        card: { DEFAULT: 'hsl(222 22% 11%)', foreground: 'hsl(210 20% 92%)' },
        primary: { DEFAULT: 'hsl(199 89% 48%)', foreground: 'hsl(222 47% 8%)' },
        destructive: { DEFAULT: 'hsl(0 72% 51%)', foreground: 'hsl(210 20% 98%)' },
        success: { DEFAULT: 'hsl(142 71% 45%)', foreground: 'hsl(222 47% 8%)' },
        warning: { DEFAULT: 'hsl(38 92% 50%)', foreground: 'hsl(222 47% 8%)' }
      },
      borderRadius: { lg: '0.5rem', md: '0.375rem', sm: '0.25rem' }
    }
  },
  plugins: []
};
