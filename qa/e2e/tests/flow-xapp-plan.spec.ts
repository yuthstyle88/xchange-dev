// QA-F14b — flow: save and delete a customer journey plan (/app, /app/journey)
// (design docs/design/qa-e2e.md §4 row F14b; #4070 §3b).
// Written by ticket #4110 (QA-F14b); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/screens/:
// - customer/home/HomeScreen.tsx:516,536,554,585,595 #amount-input, calculate, .xc-home-estimate-card, save, .xc-home-alert-success
// - customer/journey/JourneyScreen.tsx:396 [data-testid="draft-card"]
// - customer/journey/PlanDelete.tsx:58 [data-action="delete-plan"]
import { expect, test } from '@playwright/test';
import { hasAccount, posted, shot, signIn, users } from './helpers';

interface PlannerDraftAnswer {
  id?: string;
  data?: { id?: string };
}

// The plan this run created; afterEach deletes it by id if the test did not, and goes red if that fails.
let createdDraftId = '';

test.describe('QA-F14b flow: save and delete customer journey plan', () => {
  test.afterEach(async ({ page }) => {
    if (!createdDraftId) return;
    const id = createdDraftId;
    createdDraftId = '';
    const res = await page.request.post(`/api/app/planner-drafts/${encodeURIComponent(id)}/delete`, {
      headers: { 'x-xchange-request': '1', 'idempotency-key': crypto.randomUUID(), 'content-type': 'application/json' },
      data: { id },
    });
    expect(res.ok() || res.status() === 404, `cleanup delete of plan ${id}: ${res.status()}`).toBeTruthy();
  });

  test('customer saves a planner draft on /app and deletes it on /app/journey', async ({ page }) => {
    if (!hasAccount('customer')) {
      const reason = 'no customer account in users.json: run scripts/reseed.sh';
      console.log(`[F14b] skip: ${reason}`);
      test.skip(true, reason);
    }

    const currency = users.currency ?? 'USD';
    const quantity = String((Date.now() % 800) + 101);

    // Step 1: /app : BUY users.currency , a quantity -> estimate card shows
    await signIn(page, 'customer', '/app');
    await page.goto('/app');

    const buyBtn = page.locator('.xc-home-planner-section .xc-home-toggle-btn').first();
    await buyBtn.click();

    const currOption = page.locator(`select#currency-select option[value="${currency}"]`);
    await expect(currOption, `rate for ${currency} loaded in planner`).toHaveCount(1, { timeout: 20_000 });
    await page.locator('select#currency-select').selectOption(currency);

    await page.locator('input#amount-input').fill(quantity);

    const calcBtn = page.locator('.xc-home-calc-actions button');
    await expect(calcBtn, 'calculate button ready').toBeEnabled({ timeout: 15_000 });
    const previewRes = await posted(page, /^\/api\/v3\/compute\/planner_preview$/, () => calcBtn.click());
    if (!previewRes.ok && previewRes.body.includes('rateUnavailable')) {
      const reason = `no published ${currency} rate on the branch (planner_preview rateUnavailable)`;
      console.log(`[F14b] skip: ${reason}`);
      test.skip(true, reason);
    }

    const estimateCard = page.locator('.xc-home-estimate-card');
    await expect(estimateCard, 'the estimate card shows').toBeVisible({ timeout: 20_000 });
    await shot(page, 'app', 'plan-1-estimate');

    // Step 2: save -> planSaved shows an id
    const saveBtn = estimateCard.locator('.xc-home-save-actions button');
    const saveRes = await posted(page, /^\/api\/app\/planner-drafts$/, () => saveBtn.click());
    console.log(`[F14b] POST /api/app/planner-drafts ${saveRes.text}`);
    expect(saveRes.ok, `POST /api/app/planner-drafts: ${saveRes.text}`).toBeTruthy();

    const draftAnswer = JSON.parse(saveRes.body) as PlannerDraftAnswer;
    const draftId = String(draftAnswer.id ?? draftAnswer.data?.id ?? '');
    expect(draftId, `draftId from POST /api/app/planner-drafts: ${saveRes.text}`).not.toBe('');
    createdDraftId = draftId;

    const planSavedAlert = page.locator('.xc-home-alert.xc-home-alert-success[role="status"]');
    await expect(planSavedAlert, 'planSaved shows an id').toContainText(draftId, { timeout: 20_000 });
    await shot(page, 'app', 'plan-2-saved');

    // Step 3: /app/journey -> a draft-card with that amount
    await page.goto('/app/journey');
    // Match the amount element exactly: the card also shows a Thai-era expiry date (e.g. 2569) that a bare quantity could hit.
    const amountText = new RegExp(`^${quantity}(\\.0+)?\\s+${currency}$`);
    const draftCard = page.locator('[data-testid="draft-card"]').filter({ has: page.locator('.xc-journey-draft-amount', { hasText: amountText }) }).first();
    await expect(draftCard, `a draft-card with amount ${quantity}`).toBeVisible({ timeout: 20_000 });
    await shot(page, 'app', 'plan-3-journey');

    // Step 4: its delete-plan -> gone after a reload
    const deleteBtn = draftCard.locator('button[data-action="delete-plan"]');
    const deleteRes = await posted(
      page,
      new RegExp(`^/api/app/planner-drafts/${encodeURIComponent(draftId)}/delete$`),
      () => deleteBtn.click(),
    );
    console.log(`[F14b] POST /api/app/planner-drafts/${draftId}/delete ${deleteRes.text}`);
    expect(deleteRes.ok, `POST /api/app/planner-drafts/${draftId}/delete: ${deleteRes.text}`).toBeTruthy();
    createdDraftId = '';

    await page.reload();
    await expect(draftCard, `draft-card with quantity ${quantity} gone after reload`).toBeHidden({ timeout: 20_000 });
    await shot(page, 'app', 'plan-4-deleted');
  });
});
