import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './e2e',
  testMatch: 'gallery.spec.ts',
  forbidOnly: Boolean(process.env.CI),
  timeout: 180000,
  use: {
    baseURL: 'http://127.0.0.1:5183',
    viewport: { width: 2880, height: 1120 },
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'webkit', use: { browserName: 'webkit' } },
    { name: 'chromium', use: { browserName: 'chromium' } },
  ],
  webServer: {
    command: 'pnpm dev --port 5183',
    url: 'http://127.0.0.1:5183',
    reuseExistingServer: false,
    env: { VITE_GALLERY: '1', VITE_XTRACE_FIXTURE: '' },
  },
});
