import { chromium } from '@playwright/test';
import { ROLE_HOME, ROLES, hasAccount, loginWithForm, saveStorageState, users } from './helpers';

export default async function globalSetup(): Promise<void> {
  const activeRoles = ROLES.filter((role) => hasAccount(role));
  if (activeRoles.length === 0) return;

  const browser = await chromium.launch();
  try {
    for (const role of activeRoles) {
      const page = await browser.newPage({ baseURL: users.baseUrl });
      try {
        await loginWithForm(page, role, ROLE_HOME[role]);
        await saveStorageState(page, role);
      } finally {
        await page.close();
      }
    }
  } finally {
    await browser.close();
  }
}
