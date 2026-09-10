import { defineConfig } from '@playwright/test';
const port = Number(process.env.E2E_PRODUCTION_PORT ?? 5194);

export default defineConfig({
  testDir: './e2e',
  testMatch: '*-production.spec.ts',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    viewport: { width: 1120, height: 720 },
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'webkit', use: { browserName: 'webkit' } },
    { name: 'chromium', use: { browserName: 'chromium' } },
  ],
  webServer: {
    command: `pnpm build && pnpm exec vite preview --host 127.0.0.1 --port ${port} --strictPort`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
    env: { VITE_XTRACE_FIXTURE: 'F1', VITE_GALLERY: '1' },
  },
});
