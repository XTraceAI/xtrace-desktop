import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './e2e',
  testMatch: 'table-overflow.spec.ts',
  forbidOnly: !!process.env.CI,
  use: {
    baseURL: 'http://127.0.0.1:5180',
    viewport: { width: 1120, height: 900 },
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'webkit', use: { browserName: 'webkit' } }],
  webServer: {
    command: 'pnpm dev --port 5180',
    url: 'http://127.0.0.1:5180',
    reuseExistingServer: false,
  },
});
