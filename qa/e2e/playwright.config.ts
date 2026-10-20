import { defineConfig } from '@playwright/test';
import { loadUsers } from './tests/helpers';

// The host and accounts come from users.json (gitignored; users.example.json when it is missing, for --list only).
const users = loadUsers();

// Customer screens are phone-sized; staff, desk and admin screens are desktop-sized (design qa-e2e.md §1).
const PHONE = /(smoke-public|smoke-app-[ab]|flow-booking)\.spec\.ts$/;

export default defineConfig({
  testDir: './tests',
  outputDir: './test-results',
  // One worker, files in name order: flow-cash (F3, funds the safe and opens a shift) runs before flow-queue (F2).
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 90_000,
  reporter: [['list'], ['html', { outputFolder: 'playwright-report', open: 'never' }]],
  use: {
    baseURL: users.baseUrl,
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'phone', testMatch: PHONE, use: { viewport: { width: 430, height: 932 } } },
    { name: 'desktop', testMatch: /\.spec\.ts$/, testIgnore: PHONE, use: { viewport: { width: 1280, height: 800 } } },
  ],
});
