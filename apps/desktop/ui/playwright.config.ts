import { defineConfig, devices } from '@playwright/test';

const port = Number(process.env.E2E_PORT ?? 5174);

export default defineConfig({
  testDir: './e2e',
  testIgnore: ['*-production.spec.ts', 'gallery.spec.ts'],
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: [['list'], ['html', { open: 'never' }]],
  use: { baseURL: `http://127.0.0.1:${port}`, trace: 'on-first-retry' },
  projects: [
    { name: 'webkit', use: { ...devices['Desktop Safari'] } },
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
  ],
  webServer: {
    command: `pnpm dev --port ${port}`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
    env: { VITE_XTRACE_FIXTURE: 'F1', VITE_GALLERY: '' },
  },
});
