// QA-S1 — smoke: public pages (/login, /app/sign-in, /app/sign-up, /rates) (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-S1; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { test } from '@playwright/test';
import { CUSTOMER_LANGS, STAFF_LANGS, smoke } from './helpers';

test.describe('QA-S1 smoke: public pages (/login, /app/sign-in, /app/sign-up, /rates)', () => {
  // key: web/src/screens/login/LoginScreen.tsx:126
  test('/login', async ({ page }) => {
    await smoke(page, {
      area: 'public',
      screen: 'login',
      path: '/login',
      role: 'anonymous',
      key: 'button[type="submit"]',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/customer/sign-in/SignIn.tsx:88
  test('/app/sign-in', async ({ page }) => {
    await smoke(page, {
      area: 'public',
      screen: 'sign-in',
      path: '/app/sign-in',
      role: 'anonymous',
      key: '.xc-sign-in',
      langs: CUSTOMER_LANGS,
    });
  });

  // key: web/src/screens/customer/sign-in/SignUp.tsx:65
  test('/app/sign-up', async ({ page }) => {
    await smoke(page, {
      area: 'public',
      screen: 'sign-up',
      path: '/app/sign-up',
      role: 'anonymous',
      key: '.xc-sign-in',
      langs: CUSTOMER_LANGS,
    });
  });

  // key: web/src/screens/customer/rate-display/RateDisplay.tsx:57
  test('/rates', async ({ page }) => {
    await smoke(page, {
      area: 'public',
      screen: 'rates',
      path: '/rates',
      role: 'anonymous',
      key: '.xc-rate-display',
      langs: CUSTOMER_LANGS,
    });
  });
});
