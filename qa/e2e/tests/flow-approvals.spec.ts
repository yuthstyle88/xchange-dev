// QA-F5 — flow: approvals list and a pending request, read only (design docs/design/qa-e2e.md §3, §4; docs/design/blueprint/rates.md §5).
// Written by ticket QA-F5 (#3876); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test } from '@playwright/test';
import { shot, signIn, watchConsole } from './helpers';

// Status filters rendered in web/src/screens/admin/approvals/Approvals.tsx:24, :320-334.
const STATUS_FILTERS = ['PENDING', 'APPROVED', 'REJECTED'] as const;

// Type filters rendered in web/src/screens/admin/approvals/Approvals.tsx:40-50, :335-358 ('' is "All" at :340).
const TYPE_FILTERS = [
  'RATE_PUBLICATION',
  'RATE_FEED_PUBLICATION',
  'CAMPAIGN_VERSION',
  'LOYALTY_RULE_VERSION',
  'EXCHANGE_TRANSACTION',
  'TRANSACTION_REVERSAL',
  'COUNT_DIFFERENCE',
  'NEGATIVE_STOCK',
  'CASH_TRANSFER',
  '',
] as const;

test.describe('QA-F5 flow: approvals list and a pending request, read only', () => {
  test('admin opens the approvals list and its filters', async ({ page }) => {
    const log = watchConsole(page);
    const decideCalls: string[] = [];

    // Read only: decide goes through POST /api/approvals/${requestId}/decide (web/src/screens/admin/parts/decide.ts:21).
    page.on('request', (req) => {
      const path = new URL(req.url()).pathname;
      if (req.method() === 'POST' && /^\/api\/approvals\/[^/]+\/decide$/.test(path)) {
        decideCalls.push(`${req.method()} ${path}`);
      }
    });

    // Step 1: signIn(page, 'admin', '/'), then page.goto('/#approvals').
    await signIn(page, 'admin', '/');
    await page.goto('/#approvals');

    // Screen root and filter bars (web/src/screens/admin/approvals/Approvals.tsx:319-358).
    const screen = page.locator('.admin-approvals-screen');
    await expect(screen).toBeVisible({ timeout: 20_000 });
    await expect(screen.locator('[data-status-switch]')).toBeVisible();
    await expect(screen.locator('[data-type-switch]')).toBeVisible();

    // Queue list (.approvals-queue at Approvals.tsx:376) or empty state (.empty at Approvals.tsx:369).
    const listOrEmpty = screen.locator('.approvals-queue, .empty').first();
    await expect(listOrEmpty).toBeVisible({ timeout: 20_000 });
    await expect(screen.locator('[role="alert"].error')).toHaveCount(0);
    await shot(page, 'flow-approvals', '1-initial');

    // Step 2: use each filter the screen offers, and assert the list re-renders without an error.
    for (const status of STATUS_FILTERS) {
      const btn = screen.locator(`[data-status-switch] button[data-status-filter="${status}"]`);
      const loaded = page
        .waitForResponse((r) => r.url().includes('approval_requests'), { timeout: 15_000 })
        .catch(() => null);
      await btn.click();
      await loaded;
      await expect(btn).toHaveAttribute('aria-pressed', 'true');
      await expect(listOrEmpty).toBeVisible({ timeout: 20_000 });
      await expect(screen.locator('[role="alert"].error')).toHaveCount(0);
    }

    // Return to PENDING (Approvals.tsx:267, :327) and cycle through every type filter (Approvals.tsx:340, :351).
    const pendingBtn = screen.locator('[data-status-switch] button[data-status-filter="PENDING"]');
    const backToPending = page
      .waitForResponse((r) => r.url().includes('approval_requests'), { timeout: 15_000 })
      .catch(() => null);
    await pendingBtn.click();
    await backToPending;
    await expect(pendingBtn).toHaveAttribute('aria-pressed', 'true');

    for (const type of TYPE_FILTERS) {
      const btn = screen.locator(`[data-type-switch] button[data-type-filter="${type}"]`);
      const loaded = page
        .waitForResponse((r) => r.url().includes('approval_requests'), { timeout: 15_000 })
        .catch(() => null);
      await btn.click();
      await loaded;
      await expect(btn).toHaveAttribute('aria-pressed', 'true');
      await expect(listOrEmpty).toBeVisible({ timeout: 20_000 });
      await expect(screen.locator('[role="alert"].error')).toHaveCount(0);
    }

    // Step 3: on PENDING + All (''), check whether a pending request exists (Approvals.tsx:152-249).
    const cards = screen.locator('[data-request]');
    const count = await cards.count();
    if (count === 0) {
      await expect(screen.locator('.empty')).toBeVisible();
      console.log('no requests');
    } else {
      const first = cards.first();
      await first.scrollIntoViewIfNeeded();
      // RequestCard renders [data-request] (:152), [data-request-type] (:155), .approval-proposer (:161),
      // and for a PENDING request its .approval-decision-form (:213) — read only, never click approve/reject (:229-244).
      const typeEl = first.locator('[data-request-type]');
      await expect(typeEl).toBeVisible();
      await expect(first.locator('.approval-proposer').first()).toBeVisible();
      await expect(first.locator('.approval-decision-form')).toBeVisible();
      const reqId = (await first.getAttribute('data-request')) ?? '';
      const reqType = (await typeEl.getAttribute('data-request-type')) ?? '';
      console.log(`pending request id=${reqId} type=${reqType}`);
      await shot(page, 'flow-approvals', '2-pending-detail');
    }

    expect(decideCalls, 'read-only flow must never send POST /api/approvals/*/decide').toEqual([]);
    expect(log.errors, 'console errors on /#approvals').toEqual([]);
  });
});
