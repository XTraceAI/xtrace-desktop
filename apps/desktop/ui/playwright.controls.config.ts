import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './e2e',
  testMatch: 'controls.spec.ts',
  forbidOnly: !!process.env.CI,
  use: {
    baseURL: 'http://127.0.0.1:5178',
    viewport: { width: 900, height: 900 },
    contextOptions: { reducedMotion: 'reduce' },
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'webkit', use: { browserName: 'webkit' } }],
  webServer: {
    command: 'pnpm dev --port 5178',
    url: 'http://127.0.0.1:5178',
    reuseExistingServer: false,
  },
});
