// QA-F14a — customer rate alert create and delete (design docs/design/qa-e2e.md §3, §4; docs/design/blueprint/rate-alerts.md §4.4, §9.1 C11).
// Written by ticket QA-F14a (#4109); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test } from '@playwright/test';
import { posted, shot, signIn, users } from './helpers';

const MARKER_TARGET = '9999.00';
// The target shows as "9,999.00 <ccy>"; match it exactly so 19,999 or 9,999.50 (not this spec's) never match.
const MARKER_SHOWN = /^9,999\.00(\s|$)/;
// The alert this run created; afterEach deletes it through the UI if the test did not, and goes red if that fails.
let pendingAlertId = '';

test.describe('QA-F14a flow: customer rate alert create and delete', () => {
  test.afterEach(async ({ page }) => {
    if (!pendingAlertId) return;
    const id = pendingAlertId;
    pendingAlertId = '';
    await page.goto('/app/rate-alerts');
    const left = page.locator(`article.xc-alert-card[data-alert-id="${id}"]`);
    await expect(page.locator('.xc-alerts-empty, .xc-alerts-list').first()).toBeVisible({ timeout: 20_000 });
    if ((await left.count()) === 0) return;
    const res = await posted(page, /^\/api\/app\/rate-alerts\/[^\/]+\/delete$/, () => left.locator('.xc-btn-delete').click());
    expect(res.ok, `cleanup delete of alert ${id}: ${res.text}`).toBe(true);
  });

  test('customer creates, verifies, and deletes a rate alert', async ({ page }) => {
    // Step 1: Sign in as customer and open /app/rate-alerts
    await signIn(page, 'customer', '/app/rate-alerts');

    // Step 2: Wait for screen to render and alerts list or empty state to appear
    await expect(page.locator('.xc-rate-alerts-screen')).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('.xc-alerts-empty, .xc-alerts-list').first()).toBeVisible({ timeout: 20_000 });

    // Step 3: Clean up any leftover test alert matching MARKER_TARGET from an earlier run
    const markerCards = page.locator('article.xc-alert-card').filter({ has: page.locator('.xc-alert-target-amount', { hasText: MARKER_SHOWN }) });
    const leftover = markerCards;
    while ((await leftover.count()) > 0) {
      const leftId = await leftover.first().getAttribute('data-alert-id');
      const first = page.locator(`article.xc-alert-card[data-alert-id="${leftId}"]`);
      await posted(page, /^\/api\/app\/rate-alerts\/[^\/]+\/delete$/, () =>
        first.locator('.xc-btn-delete').click(),
      );
      await expect(first).not.toBeAttached({ timeout: 10_000 });
    }
    await expect(leftover).toHaveCount(0);

    // Step 4: Fill create rate alert form
    const branchSelect = page.locator('#alert-branch');
    await expect(branchSelect.locator('option')).not.toHaveCount(0, { timeout: 10_000 });
    const targetBranch = users.branch || 'MAIN';
    if ((await branchSelect.locator(`option[value="${targetBranch}"]`).count()) > 0) {
      await branchSelect.selectOption(targetBranch);
    }
    const selectedBranch = await branchSelect.inputValue();

    const currency = (users.currency || 'USD').toUpperCase();
    await page.locator('#alert-currency').fill(currency);
    await page.locator('#alert-condition').selectOption('ABOVE');
    await page.locator('#alert-target').fill(MARKER_TARGET);

    // Step 5: Submit form and handle potential alertSettingsMissing on unseeded host
    const createRes = await posted(page, /^\/api\/app\/rate-alerts$/, () =>
      page.locator('.xc-create-alert-form button[type="submit"]').click(),
    );

    if (!createRes.ok && createRes.body.includes('alertSettingsMissing')) {
      console.log('[F14a] skip: rate_alert_settings not seeded on host (alertSettingsMissing)');
      test.skip(true, 'rate_alert_settings not seeded on host: alertSettingsMissing');
      return;
    }
    expect(createRes.ok, `POST /api/app/rate-alerts failed: ${createRes.text}`).toBe(true);

    let createdId: string | undefined;
    try {
      const parsed = JSON.parse(createRes.body);
      if (typeof parsed?.id === 'string') {
        createdId = parsed.id;
        pendingAlertId = parsed.id;
      }
    } catch {
      // fallback to marker filter
    }

    // Step 6: Verify newly created alert card in list
    const card = createdId
      ? page.locator(`article.xc-alert-card[data-alert-id="${createdId}"]`)
      : markerCards.first();

    await expect(card).toBeVisible({ timeout: 15_000 });
    await expect(card.locator('.xc-alert-currency')).toContainText(currency);
    if (selectedBranch) {
      await expect(card.locator('.xc-alert-branch-badge')).toContainText(selectedBranch);
    }
    await expect(card.locator('.xc-alert-target-amount')).toContainText('9,999');

    await shot(page, 'app', 'rate-alert-created');

    // Step 7: Delete the alert
    const deleteBtn = card.locator('.xc-btn-delete');
    const deleteRes = await posted(
      page,
      /^\/api\/app\/rate-alerts\/[^\/]+\/delete$/,
      () => deleteBtn.click(),
    );
    expect(deleteRes.ok, `POST /api/app/rate-alerts/{id}/delete failed: ${deleteRes.text}`).toBe(true);
    await expect(card).not.toBeAttached({ timeout: 15_000 });
    pendingAlertId = '';

    // Step 8: Reload page to verify persistence of deletion
    await page.reload();
    await expect(page.locator('.xc-rate-alerts-screen')).toBeVisible({ timeout: 20_000 });
    await expect(page.locator('.xc-alerts-empty, .xc-alerts-list').first()).toBeVisible({ timeout: 20_000 });

    const remainingMarkerCard = createdId
      ? page.locator(`article.xc-alert-card[data-alert-id="${createdId}"]`)
      : markerCards;
    await expect(remainingMarkerCard).toHaveCount(0);

    await shot(page, 'app', 'rate-alert-deleted');
  });
});
