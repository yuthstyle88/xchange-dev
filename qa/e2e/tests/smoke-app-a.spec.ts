// QA-S2 — smoke: customer app A (/app, /app/branches, /app/journey, /app/assistant, /app/history, /app/book) (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-S2; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { test } from '@playwright/test';
import { CUSTOMER_LANGS, signIn, smoke } from './helpers';

test.describe('QA-S2 smoke: customer app A (/app, /app/branches, /app/journey, /app/assistant, /app/history, /app/book)', () => {
  // key: web/src/screens/customer/home/HomeScreen.tsx:355
  test('/app', async ({ page }) => {
    await signIn(page, 'customer', '/app');
    await smoke(page, {
      area: 'app-a',
      screen: 'home',
      path: '/app',
      role: 'customer',
      key: '.xc-customer-home',
      langs: CUSTOMER_LANGS,
    });
  });

  // key: web/src/screens/customer/branches/BranchesScreen.tsx:121
  test('/app/branches', async ({ page }) => {
    await signIn(page, 'customer', '/app/branches');
    await smoke(page, {
      area: 'app-a',
      screen: 'branches',
      path: '/app/branches',
      role: 'customer',
      key: '.xc-branches-screen',
      langs: CUSTOMER_LANGS,
    });
  });

  // key: web/src/screens/customer/journey/JourneyScreen.tsx:245
  test('/app/journey', async ({ page }) => {
    await signIn(page, 'customer', '/app/journey');
    await smoke(page, {
      area: 'app-a',
      screen: 'journey',
      path: '/app/journey',
      role: 'customer',
      key: '.xc-journey-screen',
      langs: CUSTOMER_LANGS,
    });
  });

  // key: web/src/screens/customer/assistant/AssistantScreen.tsx:232
  test('/app/assistant', async ({ page }) => {
    await signIn(page, 'customer', '/app/assistant');
    await smoke(page, {
      area: 'app-a',
      screen: 'assistant',
      path: '/app/assistant',
      role: 'customer',
      key: '.assistant-container',
      langs: CUSTOMER_LANGS,
    });
  });

  // key: web/src/screens/customer/history/HistoryScreen.tsx:67
  test('/app/history', async ({ page }) => {
    await signIn(page, 'customer', '/app/history');
    await smoke(page, {
      area: 'app-a',
      screen: 'history',
      path: '/app/history',
      role: 'customer',
      key: '.xc-history-screen',
      langs: CUSTOMER_LANGS,
    });
  });

  // key: web/src/screens/customer/book/Book.tsx:394
  test('/app/book', async ({ page }) => {
    await signIn(page, 'customer', '/app/book');
    await smoke(page, {
      area: 'app-a',
      screen: 'book',
      path: '/app/book',
      role: 'customer',
      key: 'data-testid=book-screen',
      langs: CUSTOMER_LANGS,
    });
  });
});
