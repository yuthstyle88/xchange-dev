// QA-S6 — smoke: admin policy menus (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-S6; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { test } from '@playwright/test';
import { signIn, smoke, STAFF_LANGS } from './helpers';

test.describe('QA-S6 smoke: admin policy menus', () => {
  // key: web/src/screens/admin/alert-settings/AlertSettings.tsx:148 (menu.ts:12 needs AlertSettingsSave)
  test('/#alert-settings', async ({ page }) => {
    await signIn(page, 'admin', '/#alert-settings');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'alert-settings',
      path: '/#alert-settings',
      role: 'admin',
      key: '.alert-settings-container',
      langs: STAFF_LANGS,
      needs: 'AlertSettingsSave',
    });
  });

  // key: web/src/screens/admin/cash-policy/CashPolicy.tsx:173 (menu.ts:13 needs CashTransferPolicySet)
  test('/#cash-policy', async ({ page }) => {
    await signIn(page, 'admin', '/#cash-policy');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'cash-policy',
      path: '/#cash-policy',
      role: 'admin',
      key: '.admin-cash-policy-screen',
      langs: STAFF_LANGS,
      needs: 'CashTransferPolicySet',
    });
  });

  // key: web/src/screens/admin/rate-alert-settings/RateAlertSettings.tsx:211 (menu.ts:13 needs RateAlertSettingsSave)
  test('/#rate-alert-settings', async ({ page }) => {
    await signIn(page, 'admin', '/#rate-alert-settings');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'rate-alert-settings',
      path: '/#rate-alert-settings',
      role: 'admin',
      key: '.rate-alert-settings',
      langs: STAFF_LANGS,
      needs: 'RateAlertSettingsSave',
    });
  });

  // key: web/src/screens/admin/campaigns/Campaigns.tsx:658 (menu.ts:13 needs MarketingCampaigns)
  test('/#campaigns', async ({ page }) => {
    await signIn(page, 'admin', '/#campaigns');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'campaigns',
      path: '/#campaigns',
      role: 'admin',
      key: '.xc-campaigns-screen',
      langs: STAFF_LANGS,
      needs: 'MarketingCampaigns',
    });
  });

  // key: web/src/screens/admin/campaign-performance/CampaignPerformance.tsx:95 (menu.ts:13 needs MarketingPerformance)
  test('/#campaign-performance', async ({ page }) => {
    await signIn(page, 'admin', '/#campaign-performance');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'campaign-performance',
      path: '/#campaign-performance',
      role: 'admin',
      key: '.xc-cp-screen',
      langs: STAFF_LANGS,
      needs: 'MarketingPerformance',
    });
  });

  // key: web/src/screens/admin/members/Members.tsx:174 (menu.ts:12 needs MarketingMembers)
  test('/#members', async ({ page }) => {
    await signIn(page, 'admin', '/#members');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'members',
      path: '/#members',
      role: 'admin',
      key: '#segment-name',
      langs: STAFF_LANGS,
      needs: 'MarketingMembers',
    });
  });

  // key: web/src/screens/admin/loyalty/Loyalty.tsx:217 (menu.ts:13 needs LoyaltyRulesManage; 503 loyaltyUnavailable when unconfigured)
  test('/#loyalty', async ({ page }) => {
    await signIn(page, 'admin', '/#loyalty');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'loyalty',
      path: '/#loyalty',
      role: 'admin',
      key: 'section[data-form="point-draft"]',
      langs: STAFF_LANGS,
      needs: 'LoyaltyRulesManage',
      allowConsole: ['loyaltyUnavailable503'],
    });
  });

  // key: web/src/screens/admin/cash-limits/CashLimits.tsx:149 (menu.ts:13 needs CashLimitsManage)
  test('/#cash-limits', async ({ page }) => {
    await signIn(page, 'admin', '/#cash-limits');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'cash-limits',
      path: '/#cash-limits',
      role: 'admin',
      key: '.cash-limits',
      langs: STAFF_LANGS,
      needs: 'CashLimitsManage',
    });
  });
});
