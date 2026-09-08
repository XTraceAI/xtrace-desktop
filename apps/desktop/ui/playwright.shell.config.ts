import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  testMatch: 'shell.spec.ts',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  use: {
    baseURL: 'http://127.0.0.1:5181',
    viewport: { width: 1120, height: 720 },
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'webkit', use: { browserName: 'webkit' } },
    { name: 'chromium', use: { browserName: 'chromium' } },
  ],
  webServer: {
    command: 'pnpm dev --port 5181',
    url: 'http://127.0.0.1:5181',
    reuseExistingServer: false,
    env: { VITE_XTRACE_FIXTURE: 'F1' },
  },
});
