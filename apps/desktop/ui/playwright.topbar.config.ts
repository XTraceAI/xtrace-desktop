import { defineConfig } from '@playwright/test';
import base from './playwright.config';

export default defineConfig({
  ...base,
  testMatch: 'topbar.spec.ts',
  use: { ...base.use, baseURL: 'http://127.0.0.1:5177' },
  webServer: {
    command: 'pnpm dev --port 5177',
    url: 'http://127.0.0.1:5177',
    reuseExistingServer: false,
  },
});
