import { defineConfig } from 'astro/config';
import { fileURLToPath } from 'node:url';
import react from '@astrojs/react';
import tailwind from '@astrojs/tailwind';
import sitemap from '@astrojs/sitemap';
import icon from 'astro-icon';

export default defineConfig({
  site: 'https://1337xcode.github.io/cortex',
  base: process.env.ASTRO_BASE ?? '/cortex',
  trailingSlash: 'always',
  integrations: [
    react(),
    tailwind(),
    sitemap({
      serialize(item) {
        // Add lastmod date to all sitemap entries
        item.lastmod = new Date().toISOString().split('T')[0];
        return item;
      },
    }),
    icon({ include: { lucide: ['*'] } }),
  ],
  markdown: {
    shikiConfig: { theme: 'github-dark-default' },
  },
  vite: {
    resolve: {
      alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
    },
    build: { rollupOptions: { output: { manualChunks: { 'three': ['three', '3d-force-graph'] } } } },
  },
});
