// Playwright settings for the map's browser checks: Chromium only, one worker, the built page
// served by route handlers (see serve.ts), so no server process and no network.
import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  testMatch: '*.spec.ts',
  workers: 1,
  timeout: 120_000,
  reporter: 'list',
  outputDir: '../test-results',
  use: {
    browserName: 'chromium',
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
  },
});
