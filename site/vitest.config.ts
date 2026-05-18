import { defineConfig } from 'vitest/config';
import { resolve } from 'node:path';

export default defineConfig({
  resolve: {
    alias: {
      '@/': resolve(__dirname, './src/'),
      '@': resolve(__dirname, './src'),
    },
  },
  test: {
    globals: true,
    passWithNoTests: true,
    include: [
      'src/**/*.{test,spec}.{ts,tsx}',
      'tests/**/*.{test,spec}.{ts,tsx}',
    ],
  },
});
