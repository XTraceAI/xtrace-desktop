import path from 'node:path';
import { defineConfig } from '@playwright/test';

if (!['capture', 'compare'].includes(process.env.XTRACE_PARITY_MODE))
  throw new Error('Use pnpm parity or pnpm parity:baselines to validate the snapshot contract.');

export default defineConfig({
  testDir: './e2e/parity',
  testMatch: 'parity.spec.ts',
  forbidOnly: true,
  workers: 1,
  retries: 0,
  timeout: 30000,
  updateSnapshots: process.env.XTRACE_PARITY_MODE === 'capture' ? 'all' : 'none',
  snapshotPathTemplate: `${process.env.XTRACE_PARITY_BASELINES}/{arg}{ext}`,
  outputDir: path.join(process.env.XTRACE_PARITY_OUTPUT, 'test-results'),
  reporter: [
    ['list'],
    ['json', { outputFile: path.join(process.env.XTRACE_PARITY_OUTPUT, 'results.json') }],
  ],
  expect: {
    toHaveScreenshot: {
      maxDiffPixelRatio: 0.002,
      threshold: 0.1,
      animations: 'disabled',
      caret: 'hide',
      scale: 'device',
    },
  },
  use: {
    browserName: 'webkit',
    viewport: { width: 2880, height: 1120 },
    deviceScaleFactor: 2,
    locale: 'en-US',
    timezoneId: 'UTC',
    colorScheme: 'light',
    reducedMotion: 'reduce',
    baseURL: 'http://127.0.0.1:5184',
  },
  webServer: {
    command: 'pnpm dev --port 5184 --strictPort',
    url: 'http://127.0.0.1:5184',
    reuseExistingServer: false,
    env: { VITE_GALLERY: '1', VITE_XTRACE_FIXTURE: '' },
  },
});
