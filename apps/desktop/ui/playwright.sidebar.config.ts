import { defineConfig } from '@playwright/test';
import base from './playwright.config';

export default defineConfig({
  ...base,
  testMatch: 'sidebar.spec.ts',
  use: { ...base.use, baseURL: 'http://127.0.0.1:5176' },
  webServer: {
    command: 'pnpm dev --port 5176',
    url: 'http://127.0.0.1:5176',
    reuseExistingServer: false,
  },
});
