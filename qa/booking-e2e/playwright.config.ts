import { defineConfig } from '@playwright/test';
import { readFileSync } from 'node:fs';

// The accounts and the host come from users.json (gitignored). Copy users.example.json to users.json and fill it.
const users = JSON.parse(readFileSync(new URL('./users.json', import.meta.url), 'utf8'));

export default defineConfig({
  testDir: './tests',
  outputDir: './test-results',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 90_000,
  reporter: [['list'], ['html', { outputFolder: 'playwright-report', open: 'never' }]],
  use: {
    baseURL: users.baseUrl,
    viewport: { width: 430, height: 932 },
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
});
