import { defineConfig, devices } from '@playwright/test';

const port = Number(process.env.E2E_PORT ?? 5174);
const viewport = { width: 1440, height: 900 };

export default defineConfig({
  testDir: './e2e',
  testIgnore: [
    'shell.spec.ts',
    'shell-production.spec.ts',
    'gallery.spec.ts',
    'gallery-production.spec.ts',
  ],
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    viewport,
    trace: 'on-first-retry',
  },
  projects: [
    { name: 'webkit', use: { ...devices['Desktop Safari'], viewport } },
    { name: 'chromium', use: { ...devices['Desktop Chrome'], viewport } },
  ],
  webServer: {
    command: `pnpm dev --port ${port}`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
    env: { VITE_GALLERY: '', VITE_XTRACE_FIXTURE: '' },
  },
});
