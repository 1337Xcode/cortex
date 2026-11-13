// Playwright settings for the map's checks against a real Cortex daemon: Chromium only, one
// worker, no retries, the scripted repository and its daemon set up once before the checks and
// stopped and deleted after them. The spec is named `*.real.ts` so that the checks against the
// stand-in daemon (`npm run e2e`, every `*.spec.ts` under e2e/) never pick it up.
import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  testMatch: '*.real.ts',
  workers: 1,
  retries: 0,
  timeout: 120_000,
  reporter: 'list',
  outputDir: '../../test-results',
  globalSetup: './setup.ts',
  globalTeardown: './teardown.ts',
  use: {
    browserName: 'chromium',
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
  },
});
