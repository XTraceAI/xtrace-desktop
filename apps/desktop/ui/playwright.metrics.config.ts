import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  testMatch: 'metrics.spec.ts',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  use: {
    baseURL: 'http://127.0.0.1:5179',
    viewport: { width: 1120, height: 720 },
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'webkit', use: { browserName: 'webkit' } }],
  webServer: {
    command: 'pnpm dev --port 5179',
    url: 'http://127.0.0.1:5179',
    reuseExistingServer: false,
  },
});
