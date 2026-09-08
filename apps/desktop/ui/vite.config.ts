import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [tailwindcss()],
  clearScreen: false,
  server: { host: '127.0.0.1', port: 5173, strictPort: true },
  build: { target: 'safari17' },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.{ts,tsx}'],
  },
});
