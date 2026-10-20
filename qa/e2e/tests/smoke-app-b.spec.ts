// QA-S3 — smoke: customer app B (/app/bookings, /app/member, /app/notifications, /app/profile, /app/rate-alerts, /app/verification) (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-S3; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { test } from '@playwright/test';
import { CUSTOMER_LANGS, signIn, smoke } from './helpers';

test.describe('QA-S3 smoke: customer app B (/app/bookings, /app/member, /app/notifications, /app/profile, /app/rate-alerts, /app/verification)', () => {
  // key: web/src/screens/customer/bookings/Bookings.tsx:299
  test('/app/bookings', async ({ page }) => {
    await signIn(page, 'customer', '/app/bookings');
    await smoke(page, {
      area: 'app-b',
      screen: 'bookings',
      path: '/app/bookings',
      role: 'customer',
      key: '.xc-bookings-screen',
      langs: CUSTOMER_LANGS,
    });
  });

  // key: web/src/screens/customer/member/Member.tsx:56
  test('/app/member', async ({ page }) => {
    await signIn(page, 'customer', '/app/member');
    await smoke(page, {
      area: 'app-b',
      screen: 'member',
      path: '/app/member',
      role: 'customer',
      key: '.xc-member-screen',
      langs: CUSTOMER_LANGS,
    });
  });

  // key: web/src/screens/customer/notifications/Notifications.tsx:43
  test('/app/notifications', async ({ page }) => {
    await signIn(page, 'customer', '/app/notifications');
    await smoke(page, {
      area: 'app-b',
      screen: 'notifications',
      path: '/app/notifications',
      role: 'customer',
      key: '.xc-notifications-screen',
      langs: CUSTOMER_LANGS,
    });
  });

  // key: web/src/screens/customer/profile/Profile.tsx:176
  test('/app/profile', async ({ page }) => {
    await signIn(page, 'customer', '/app/profile');
    await smoke(page, {
      area: 'app-b',
      screen: 'profile',
      path: '/app/profile',
      role: 'customer',
      key: '.xc-profile',
      langs: CUSTOMER_LANGS,
    });
  });

  // key: web/src/screens/customer/rate-alerts/RateAlerts.tsx:386
  test('/app/rate-alerts', async ({ page }) => {
    await signIn(page, 'customer', '/app/rate-alerts');
    await smoke(page, {
      area: 'app-b',
      screen: 'rate-alerts',
      path: '/app/rate-alerts',
      role: 'customer',
      key: '.xc-rate-alerts-screen',
      langs: CUSTOMER_LANGS,
    });
  });

  // key: web/src/screens/customer/verification/VerificationScreen.tsx:255
  test('/app/verification', async ({ page }) => {
    await signIn(page, 'customer', '/app/verification');
    await smoke(page, {
      area: 'app-b',
      screen: 'verification',
      path: '/app/verification',
      role: 'customer',
      key: '.xc-verification-screen',
      langs: CUSTOMER_LANGS,
    });
  });
});
