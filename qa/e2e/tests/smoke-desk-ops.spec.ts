// QA-S8 — smoke: desk and operations pages (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-S8; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { test } from '@playwright/test';
import { STAFF_LANGS, signIn, smoke } from './helpers';

test.describe('QA-S8 smoke: desk and operations pages', () => {
  // key: web/src/screens/desk/bookings/DeskBookingsScreen.tsx:606
  test('/desk/bookings', async ({ page }) => {
    await signIn(page, 'teller', '/desk/bookings');
    await smoke(page, {
      area: 'desk-ops',
      screen: 'desk-bookings',
      path: '/desk/bookings',
      role: 'teller',
      key: '.xc-desk-bookings',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/desk/queue/counter/CounterBar.tsx:66
  test('/desk/queue', async ({ page }) => {
    await signIn(page, 'teller', '/desk/queue');
    await smoke(page, {
      area: 'desk-ops',
      screen: 'desk-queue',
      path: '/desk/queue',
      role: 'teller',
      key: '.desk-actions',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/desk-transaction/TransactionScreen.tsx:173
  test('/desk/transactions/00000000-0000-0000-0000-000000000000', async ({ page }) => {
    await signIn(page, 'teller', '/desk/transactions/00000000-0000-0000-0000-000000000000');
    await smoke(page, {
      area: 'desk-ops',
      screen: 'desk-transaction',
      path: '/desk/transactions/00000000-0000-0000-0000-000000000000',
      role: 'teller',
      key: '.xc-dtx[data-state="not-found"]',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/kiosk-queue/KioskQueueScreen.tsx:11
  test('/kiosk-queue.html', async ({ page }) => {
    await signIn(page, 'teller', '/kiosk-queue.html');
    await smoke(page, {
      area: 'desk-ops',
      screen: 'kiosk-queue',
      path: '/kiosk-queue.html',
      role: 'teller',
      key: '.queue-page',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/receipt-sample/ReceiptSample.tsx:24
  test('/receipt-sample', async ({ page }) => {
    await signIn(page, 'admin', '/receipt-sample');
    await smoke(page, {
      area: 'desk-ops',
      screen: 'receipt-sample',
      path: '/receipt-sample',
      role: 'admin',
      key: '.receipt',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/devices/DevicesScreen.tsx:494
  test('/devices', async ({ page }) => {
    await signIn(page, 'admin', '/devices');
    await smoke(page, {
      area: 'desk-ops',
      screen: 'devices',
      path: '/devices',
      role: 'admin',
      key: '.xc-devices',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/documents/DocumentsScreen.tsx:47
  test('/documents', async ({ page }) => {
    await signIn(page, 'admin', '/documents');
    await smoke(page, {
      area: 'desk-ops',
      screen: 'documents',
      path: '/documents',
      role: 'admin',
      key: '.documents-screen',
      langs: STAFF_LANGS,
      allowConsole: ['documentsNotFound404'],
    });
  });

  // key: web/src/screens/operations/verification/VerificationScreen.tsx:681
  test('/operations/verification', async ({ page }) => {
    await signIn(page, 'admin', '/operations/verification');
    await smoke(page, {
      area: 'desk-ops',
      screen: 'verification',
      path: '/operations/verification',
      role: 'admin',
      key: 'data-testid=verification-screen',
      langs: STAFF_LANGS,
    });
  });

  // key: web/src/screens/kiosk-currencies/KioskCurrenciesScreen.tsx:326
  test('/kiosk-currencies.html', async ({ page }) => {
    await signIn(page, 'admin', '/kiosk-currencies.html');
    await smoke(page, {
      area: 'desk-ops',
      screen: 'kiosk-currencies',
      path: '/kiosk-currencies.html',
      role: 'admin',
      key: '#bank-reference',
      langs: STAFF_LANGS,
    });
  });
});
