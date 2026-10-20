import { chromium } from '@playwright/test';
import { ROLE_HOME, ROLES, hasAccount, loginWithForm, saveStorageState, users } from './helpers';

export default async function globalSetup(): Promise<void> {
  const activeRoles = ROLES.filter((role) => hasAccount(role));
  if (activeRoles.length === 0) return;

  const failed: string[] = [];
  const browser = await chromium.launch();
  try {
    for (const role of activeRoles) {
      const page = await browser.newPage({ baseURL: users.baseUrl });
      try {
        await loginWithForm(page, role, ROLE_HOME[role]);
        await saveStorageState(page, role);
      } catch (err) {
        failed.push(role);
        const message = err instanceof Error ? err.message : String(err);
        console.log(`[global-setup] ${role}: ${message}`);
      } finally {
        await page.close();
      }
    }
  } finally {
    await browser.close();
  }
  if (failed.length > 0) {
    console.log(`[global-setup] sign-in failed for ${failed.join(', ')}`);
  }
}
