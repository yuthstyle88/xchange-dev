// QA-F20 — flow: desk kiosk cancel through the entry bar
// (design docs/design/qa-e2e.md §4 row F20; docs/design/blueprint/kiosk-cancel.md §2, §3.4, §3.5 KC-2b).
// Written by ticket #4238 (QA-F20); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors & routes cited from source:
// - app/lib/features/kiosk/infrastructure/data_sources/kiosk_api.dart:49-90
//   POST /api/kiosk { action: 'quote' | 'confirm' | 'status', payload }
// - web/src/shared/ui/QueuePanel.tsx:379-424
//   article.xc-queue-card, h3 (queue number), .queue-order-actions button.queue-primary (POST /api/kiosk accept)
// - web/src/screens/desk/DeskPage.tsx:34-38
//   [data-desk-entry-bar]
// - web/src/screens/desk/entries/kiosk/entry.ts:28-34,47,54,74-94
//   POST /api/kiosk { action: 'get' }, button[data-action="cancel-kiosk-request"],
//   window.location.assign(`/desk?branch=${store.branch()}`)
// - web/src/shared/ui/CancelReasonDialog.tsx:61-95
//   dialog.xc-dialog, select[name="reason"], textarea[name="note"], button[data-action="confirm-cancel-request"]
import { expect, test } from '@playwright/test';
import {
  closeShift,
  ensureShiftClosed,
  hasAccount,
  openShift,
  posted,
  shot,
  signIn,
  users,
  type Role,
} from './helpers';

const REQUIRED_ROLES: readonly Role[] = ['teller', 'branchManager'];
const KIOSK_PRECONDITION_RE = /Counter is closed|Rate changed or counter closed|queueClosed|outside.*hours/i;

interface KioskQuoteAnswer {
  id: string;
  queue: string;
}

interface KioskConfirmAnswer {
  id: string;
  queue: string;
  status: string;
  cancelToken?: string;
}

interface KioskIdentityAnswer {
  id: string;
  queue: string;
  status: string;
}

interface BranchServiceStatusEnvelope {
  data?: {
    queueOpen?: boolean;
  };
}

// POST /api/kiosk applies the same-origin rule (kiosk.rs:245, extractors/origin.rs:4-6): X-Xchange-Request: 1 and
// an Origin equal to the host. Without them every call answers 400 "Request rejected." and the test would only skip.
const kioskHeaders = { 'x-xchange-request': '1', origin: new URL(users.baseUrl).origin };

test.describe.serial('F20 desk kiosk cancel', () => {
  test('teller accepts a kiosk request, cancels it from the desk entry bar, and reopening it is refused', async ({
    browser,
  }) => {
    const missing = REQUIRED_ROLES.filter((role) => !hasAccount(role));
    if (missing.length > 0) {
      const reason = `missing account in users.json for: ${missing.join(', ')} (run scripts/reseed.sh)`;
      console.log(`[F20] skip: ${reason}`);
      test.skip(true, reason);
    }

    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const teller = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });

    let shiftOpened = false;
    let openedQueue = false;
    let requestId = '';
    let queueNo = '';
    let cancelled = false;

    try {
      // Open the teller's shift and ensure the branch queue is open so confirm can issue a walk-in ticket
      // and accept has an ACTIVE counter (queue-tickets.md Q7, §4.5).
      await signIn(manager, 'branchManager', '/staff');
      await signIn(teller, 'teller', '/staff');
      await ensureShiftClosed(teller, manager);
      await openShift(teller, manager);
      shiftOpened = true;

      await manager.goto('/desk/queue');
      const openQueueBtn = manager.locator('button[name="openQueue"]');
      await expect(openQueueBtn, 'branch manager openQueue button (CounterBar.tsx:114-126)').toBeVisible({
        timeout: 20_000,
      });

      const statusViewRes = await manager.request.get(
        `/api/v3/views/branch_service_status?branch=${encodeURIComponent(users.branch)}`,
      );
      expect(
        statusViewRes.ok(),
        `GET /api/v3/views/branch_service_status: ${statusViewRes.status()} ${await statusViewRes.text().catch(() => '')}`,
      ).toBeTruthy();
      const statusViewBody = (await statusViewRes.json()) as BranchServiceStatusEnvelope;
      if (!statusViewBody.data?.queueOpen) {
        const openRes = await posted(manager, /^\/api\/queue\/open$/, () => openQueueBtn.click());
        console.log(`[F20] POST /api/queue/open ${openRes.text}`);
        expect(openRes.ok, `open queue accepted: ${openRes.text}`).toBeTruthy();
        openedQueue = true;
      }

      // Step 1: Create a WAITING kiosk request on MAIN via POST /api/kiosk quote then confirm (Public, as in F19).
      const quoteRes = await teller.request.post('/api/kiosk', {
        headers: kioskHeaders,
        data: {
          action: 'quote',
          payload: {
            type: 'BUY',
            currency: users.currency ?? 'USD',
            amount: users.quantity ?? '100',
            paymentMethod: 'Cash',
            branchId: users.branch,
          },
        },
      });
      const quoteText = await quoteRes.text();
      console.log(`[F20] POST /api/kiosk quote: ${quoteRes.status()} ${quoteText.slice(0, 300)}`);
      if (!quoteRes.ok()) {
        if (quoteRes.status() < 500 && KIOSK_PRECONDITION_RE.test(quoteText)) {
          test.skip(true, `POST /api/kiosk quote refused (${quoteRes.status()}): ${quoteText}`);
        }
        expect(quoteRes.ok(), `POST /api/kiosk quote: ${quoteRes.status()} ${quoteText}`).toBeTruthy();
      }
      const quoted = JSON.parse(quoteText) as KioskQuoteAnswer;
      expect(quoted.id, 'quoted kiosk request id').toBeTruthy();

      // KioskMachine::next(Confirm) refuses a quote younger than 3,000 ms (kiosk_machine.rs:165-167).
      await teller.waitForTimeout(3_100);

      const confirmRes = await teller.request.post('/api/kiosk', {
        headers: kioskHeaders,
        data: {
          action: 'confirm',
          payload: {
            id: quoted.id,
            branchId: users.branch,
          },
        },
      });
      const confirmText = await confirmRes.text();
      console.log(`[F20] POST /api/kiosk confirm: ${confirmRes.status()} ${confirmText.slice(0, 300)}`);
      if (!confirmRes.ok()) {
        if (confirmRes.status() < 500 && KIOSK_PRECONDITION_RE.test(confirmText)) {
          test.skip(true, `POST /api/kiosk confirm refused (${confirmRes.status()}): ${confirmText}`);
        }
        expect(confirmRes.ok(), `POST /api/kiosk confirm: ${confirmRes.status()} ${confirmText}`).toBeTruthy();
      }
      const confirmed = JSON.parse(confirmText) as KioskConfirmAnswer;
      requestId = String(confirmed.id ?? '').trim();
      queueNo = String(confirmed.queue ?? '').trim();
      expect(requestId, 'confirmed kiosk request id').toBe(quoted.id);
      expect(queueNo, 'confirmed kiosk queue number').not.toBe('');
      expect(confirmed.status, 'confirmed kiosk status').toBe('WAITING');
      expect(confirmed.cancelToken, 'confirm mints cancelToken once').toBeTruthy();

      // Step 2: The teller on /kiosk-queue.html finds the row and accepts it, then opens /desk?branch=MAIN&kiosk=<id>.
      await teller.goto(`/kiosk-queue.html?branch=${encodeURIComponent(users.branch)}`);
      const card = teller
        .locator('article.xc-queue-card')
        .filter({ has: teller.locator('h3', { hasText: queueNo }) })
        .first();
      await expect(card, `waiting kiosk card ${queueNo} on /kiosk-queue.html (QueuePanel.tsx:379-382)`).toBeVisible({
        timeout: 20_000,
      });

      const acceptBtn = card.locator('.queue-order-actions button.queue-primary');
      await expect(acceptBtn, 'accept button on waiting kiosk card (QueuePanel.tsx:419)').toBeVisible({
        timeout: 15_000,
      });
      const acceptRes = await posted(teller, /^\/api\/kiosk$/, () => acceptBtn.click());
      console.log(`[F20] POST /api/kiosk accept ${acceptRes.text}`);
      expect(acceptRes.ok, `POST /api/kiosk accept: ${acceptRes.text}`).toBeTruthy();

      const getLoaded = teller.waitForResponse(
        (r) =>
          r.request().method() === 'POST' &&
          new URL(r.url()).pathname === '/api/kiosk' &&
          (r.request().postData() ?? '').includes('"get"'),
        { timeout: 15_000 },
      );
      await teller.goto(`/desk?branch=${encodeURIComponent(users.branch)}&kiosk=${encodeURIComponent(requestId)}`);
      const firstGet = await getLoaded;
      expect(
        firstGet.ok(),
        `POST /api/kiosk get for ${requestId}: ${firstGet.status()} ${await firstGet.text().catch(() => '')}`,
      ).toBeTruthy();

      const entryBar = teller.locator('[data-desk-entry-bar]');
      await expect(entryBar, 'desk entry bar visible (DeskPage.tsx:34)').toBeVisible({ timeout: 15_000 });
      await expect(entryBar, `entry bar banner shows queue ${queueNo} (entry.ts:47)`).toContainText(queueNo);
      const cancelBtn = entryBar.locator('button[data-action="cancel-kiosk-request"]');
      await expect(cancelBtn, 'cancel-kiosk-request button on entry bar (entry.ts:54)').toBeVisible({
        timeout: 15_000,
      });
      await shot(teller, 'desk', 'f20-1-desk-entry-bar');

      // Step 3: Click [data-action="cancel-kiosk-request"] -> CancelReasonDialog opens -> pick CUSTOMER_CHANGED_MIND -> confirm.
      await cancelBtn.click();
      const dialog = teller.locator('dialog.xc-dialog');
      await expect(dialog, 'CancelReasonDialog open (CancelReasonDialog.tsx:61)').toBeVisible({ timeout: 15_000 });
      await dialog.locator('select[name="reason"]').selectOption('CUSTOMER_CHANGED_MIND');
      await dialog.locator('textarea[name="note"]').fill(`e2e-F20-${Date.now()}`);

      const cancelRes = await posted(teller, /^\/api\/kiosk$/, () =>
        dialog.locator('button[data-action="confirm-cancel-request"]').click(),
      );
      console.log(`[F20] POST /api/kiosk cancel ${cancelRes.text}`);
      expect(cancelRes.ok, `POST /api/kiosk cancel: ${cancelRes.text}`).toBeTruthy();
      const cancelBody = JSON.parse(cancelRes.body) as KioskIdentityAnswer;
      expect(cancelBody.status, 'cancel response status').toBe('CANCELLED');

      // The desk navigates to /desk?branch=MAIN with no kiosk= parameter (entry.ts:89).
      await teller.waitForURL(
        (url) =>
          url.pathname === '/desk' &&
          url.searchParams.get('branch') === users.branch &&
          !url.searchParams.has('kiosk'),
        { timeout: 15_000 },
      );
      await expect(teller.locator('[data-desk-entry-bar]'), 'entry bar hidden after cancel reload').toHaveCount(0);
      await shot(teller, 'desk', 'f20-2-desk-cancelled');

      const statusRes = await teller.request.post('/api/kiosk', {
        headers: kioskHeaders,
        data: {
          action: 'status',
          payload: { id: requestId, branchId: users.branch },
        },
      });
      expect(statusRes.ok(), `POST /api/kiosk status: ${statusRes.status()}`).toBeTruthy();
      const statusBody = (await statusRes.json()) as KioskIdentityAnswer;
      expect(statusBody.status, 'POST /api/kiosk status answers CANCELLED').toBe('CANCELLED');
      cancelled = true;

      // Step 4: Open /desk?branch=MAIN&kiosk=<id> again -> the server's get refuses with "คำขอนี้ถูกยกเลิกแล้ว".
      const refusedGetPromise = teller.waitForResponse(
        (r) =>
          r.request().method() === 'POST' &&
          new URL(r.url()).pathname === '/api/kiosk' &&
          (r.request().postData() ?? '').includes('"get"'),
        { timeout: 15_000 },
      );
      await teller.goto(`/desk?branch=${encodeURIComponent(users.branch)}&kiosk=${encodeURIComponent(requestId)}`);
      const refusedGet = await refusedGetPromise;
      const refusedText = await refusedGet.text();
      console.log(`[F20] POST /api/kiosk get on cancelled request: ${refusedGet.status()} ${refusedText}`);
      expect(refusedGet.ok(), `POST /api/kiosk get on cancelled request must be refused: ${refusedText}`).toBe(false);
      expect(refusedText, 'refused get answers คำขอนี้ถูกยกเลิกแล้ว').toContain('คำขอนี้ถูกยกเลิกแล้ว');
      await expect(
        teller.locator('[data-desk-entry-bar]'),
        'cancelled request does not mount desk entry bar',
      ).toHaveCount(0);
      await shot(teller, 'desk', 'f20-3-reopen-refused');
    } finally {
      try {
        if (requestId && !cancelled) {
          const cleanRes = await teller.request.post('/api/kiosk', {
            headers: kioskHeaders,
            data: {
              action: 'cancel',
              payload: {
                id: requestId,
                branchId: users.branch,
                reason: 'CUSTOMER_LEFT',
              },
            },
          });
          const cleanText = await cleanRes.text().catch(() => '');
          console.log(`[F20] finally: POST /api/kiosk cancel ${cleanRes.status()} ${cleanText}`);
          expect(cleanRes.ok(), `cleanup cancel ${requestId}: ${cleanRes.status()} ${cleanText}`).toBeTruthy();
        }
      } finally {
        try {
          if (shiftOpened) {
            await teller.goto('/staff');
            await manager.goto('/staff');
            await closeShift(teller, manager);
          }
        } finally {
          if (openedQueue) {
            await manager.goto('/desk/queue');
            const closeQueueBtn = manager.locator('button[name="closeQueue"]');
            await expect(closeQueueBtn, 'branch manager closeQueue button (CounterBar.tsx:127-139)').toBeVisible({
              timeout: 20_000,
            });
            const closeRes = await posted(manager, /^\/api\/queue\/close$/, () => closeQueueBtn.click());
            console.log(`[F20] POST /api/queue/close ${closeRes.text}`);
            expect(closeRes.ok, `close queue accepted: ${closeRes.text}`).toBeTruthy();
          }
        }
      }
      await teller.close().catch(() => {});
      await manager.close().catch(() => {});
    }
  });
});
