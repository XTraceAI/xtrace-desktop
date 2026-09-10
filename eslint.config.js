import js from '@eslint/js';
import { defineConfig } from 'eslint/config';
import hooks from 'eslint-plugin-react-hooks';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import { galleryBoundary } from './apps/desktop/ui/scripts/gallery-boundary.mjs';

export default defineConfig(
  { ignores: ['**/target/**', '**/node_modules/**', '**/dist/**', '**/gen/**'] },
  {
    files: ['**/*.{js,mjs,ts,tsx}'],
    extends: [js.configs.recommended, tseslint.configs.recommended],
    languageOptions: { globals: { ...globals.browser, ...globals.node } },
  },
  {
    files: ['apps/desktop/ui/src/**/*.{ts,tsx}'],
    plugins: { 'react-hooks': hooks, gallery: { rules: { boundary: galleryBoundary } } },
    rules: {
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'error',
      'gallery/boundary': 'error',
    },
  },
);
