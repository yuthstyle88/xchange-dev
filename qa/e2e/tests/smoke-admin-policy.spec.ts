// QA-S6 — smoke: admin policy menus (design docs/design/qa-e2e.md §3, §4). Written by ticket QA-S6; this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { test } from '@playwright/test';
import { hasAccount, signIn, smoke, STAFF_LANGS } from './helpers';

test.describe('QA-S6 smoke: admin policy menus', () => {
  // key: web/src/screens/admin/alert-settings/AlertSettings.tsx:148 (menu.ts:12 needs AlertSettingsSave)
  test('/#alert-settings', async ({ page }) => {
    const role = hasAccount('executive') ? 'executive' : 'admin';
    await signIn(page, role, '/#alert-settings');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'alert-settings',
      path: '/#alert-settings',
      role,
      key: '.alert-settings-container',
      langs: STAFF_LANGS,
      needs: 'AlertSettingsSave',
    });
  });

  // key: web/src/screens/admin/cash-policy/CashPolicy.tsx:173 (menu.ts:13 needs CashTransferPolicySet)
  test('/#cash-policy', async ({ page }) => {
    const role = hasAccount('executive') ? 'executive' : 'admin';
    await signIn(page, role, '/#cash-policy');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'cash-policy',
      path: '/#cash-policy',
      role,
      key: '.admin-cash-policy-screen',
      langs: STAFF_LANGS,
      needs: 'CashTransferPolicySet',
    });
  });

  // key: web/src/screens/admin/rate-alert-settings/RateAlertSettings.tsx:211 (menu.ts:13 needs RateAlertSettingsSave)
  test('/#rate-alert-settings', async ({ page }) => {
    const role = hasAccount('executive') ? 'executive' : 'admin';
    await signIn(page, role, '/#rate-alert-settings');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'rate-alert-settings',
      path: '/#rate-alert-settings',
      role,
      key: '.rate-alert-settings',
      langs: STAFF_LANGS,
      needs: 'RateAlertSettingsSave',
    });
  });

  // key: web/src/screens/admin/campaigns/Campaigns.tsx:658 (menu.ts:13 needs MarketingCampaigns; marketing holds it)
  test('/#campaigns', async ({ page }) => {
    const role = hasAccount('marketing') ? 'marketing' : 'admin';
    await signIn(page, role, '/#campaigns');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'campaigns',
      path: '/#campaigns',
      role,
      key: '.xc-campaigns-screen',
      langs: STAFF_LANGS,
      needs: 'MarketingCampaigns',
    });
  });

  // key: web/src/screens/admin/campaign-performance/CampaignPerformance.tsx:95 (menu.ts:13 needs MarketingPerformance; marketing holds it)
  test('/#campaign-performance', async ({ page }) => {
    const role = hasAccount('marketing') ? 'marketing' : 'admin';
    await signIn(page, role, '/#campaign-performance');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'campaign-performance',
      path: '/#campaign-performance',
      role,
      key: '.xc-cp-screen',
      langs: STAFF_LANGS,
      needs: 'MarketingPerformance',
    });
  });

  // key: web/src/screens/admin/members/Members.tsx:174 (menu.ts:12 needs MarketingMembers; marketing holds it)
  test('/#members', async ({ page }) => {
    const role = hasAccount('marketing') ? 'marketing' : 'admin';
    await signIn(page, role, '/#members');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'members',
      path: '/#members',
      role,
      key: '#segment-name',
      langs: STAFF_LANGS,
      needs: 'MarketingMembers',
    });
  });

  // key: web/src/screens/admin/loyalty/Loyalty.tsx:217 (menu.ts:13 needs LoyaltyRulesManage; 503 loyaltyUnavailable when unconfigured)
  test('/#loyalty', async ({ page }) => {
    const role = hasAccount('executive') ? 'executive' : 'admin';
    await signIn(page, role, '/#loyalty');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'loyalty',
      path: '/#loyalty',
      role,
      key: 'section[data-form="point-draft"]',
      langs: STAFF_LANGS,
      needs: 'LoyaltyRulesManage',
      allowConsole: ['loyaltyUnavailable503'],
    });
  });

  // key: web/src/screens/admin/cash-limits/CashLimits.tsx:149 (menu.ts:13 needs CashLimitsManage)
  test('/#cash-limits', async ({ page }) => {
    const role = hasAccount('executive') ? 'executive' : 'admin';
    await signIn(page, role, '/#cash-limits');
    await smoke(page, {
      area: 'admin-policy',
      screen: 'cash-limits',
      path: '/#cash-limits',
      role,
      key: '.cash-limits',
      langs: STAFF_LANGS,
      needs: 'CashLimitsManage',
    });
  });
});
