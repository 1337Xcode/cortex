// Vite and Vitest settings for the map. The page is served under `/<token>/`, so every asset
// URL must be relative: `base: './'` keeps the token in front of each request. Tailwind builds the
// utility classes the components use from the stylesheet's own theme (see src/index.css).
import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  base: './',
  plugins: [react(), tailwindcss()],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
});
