// QA-F19 — flow: kiosk cancel (staff from the queue panel; customer with cancelToken, wrong token refused)
// (design docs/design/qa-e2e.md §4 row F19; docs/design/kiosk-cancel.md §3.3, §3.4, §5.2).
// Written by ticket #4237 (QA-F19); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors and routes cited from web/src/shared/ui/ and api/src/presentation/http/kiosk.rs:
// - QueuePanel.tsx:379-383,453-462 article.xc-queue-card (h3 queue number), button[data-action="cancel-request"]
// - CancelReasonDialog.tsx:61-96   dialog.xc-dialog, select[name="reason"], textarea[name="note"], button[data-action="confirm-cancel-request"]
// - kiosk-cancel.ts:12-25          POST /api/kiosk { action: 'cancel', payload: { id, branchId, reason, note } }
// - kiosk.rs:382-476,660-715       POST /api/kiosk actions: quote, confirm, status, cancel, customerCancel
import { expect, type Page, test } from '@playwright/test';
import { posted, shot, signIn, users } from './helpers';

interface KioskQuoteAnswer {
  id: string;
  queue: string;
  status: string;
}

interface KioskConfirmAnswer {
  id: string;
  queue: string;
  orderNo: string;
  transactionNo: string | null;
  status: string;
  cancelToken?: string;
}

interface KioskIdentityAnswer {
  id: string;
  queue: string;
  orderNo: string;
  transactionNo: string | null;
  status: string;
}

const kioskHeaders = {
  'x-xchange-request': '1',
  origin: new URL(users.baseUrl).origin,
  'content-type': 'application/json',
};
const KIOSK_PRECONDITION_RE = /Counter is closed|Rate changed or counter closed|queueClosed|outside.*hours/i;

async function createWaitingKioskRequest(
  page: Page,
): Promise<{ id: string; queue: string; cancelToken: string }> {
  const currency = users.currency ?? 'USD';
  const quoteRes = await page.request.post('/api/kiosk', {
    headers: kioskHeaders,
    data: {
      action: 'quote',
      payload: {
        type: 'BUY',
        currency,
        amount: '100',
        branchId: users.branch,
      },
    },
  });
  const quoteText = await quoteRes.text().catch(() => '');
  if (!quoteRes.ok()) {
    if (quoteRes.status() < 500 && KIOSK_PRECONDITION_RE.test(quoteText)) {
      console.log(`[F19] skip on quote refusal: ${quoteRes.status()} ${quoteText}`);
      test.skip(true, `${quoteRes.status()} ${quoteText}`);
    }
    expect(quoteRes.ok(), `POST /api/kiosk quote: ${quoteRes.status()} ${quoteText}`).toBeTruthy();
  }
  const quote = JSON.parse(quoteText) as KioskQuoteAnswer;
  expect(quote.id, 'kiosk quote id').toBeTruthy();

  // KioskMachine::next(Confirm) requires age >= 3,000 ms (kiosk_machine.rs:163).
  await page.waitForTimeout(3_100);

  const confirmRes = await page.request.post('/api/kiosk', {
    headers: kioskHeaders,
    data: {
      action: 'confirm',
      payload: {
        id: quote.id,
        branchId: users.branch,
      },
    },
  });
  const confirmText = await confirmRes.text().catch(() => '');
  if (!confirmRes.ok()) {
    if (confirmRes.status() < 500 && KIOSK_PRECONDITION_RE.test(confirmText)) {
      console.log(`[F19] skip on confirm refusal: ${confirmRes.status()} ${confirmText}`);
      test.skip(true, `${confirmRes.status()} ${confirmText}`);
    }
    expect(confirmRes.ok(), `POST /api/kiosk confirm: ${confirmRes.status()} ${confirmText}`).toBeTruthy();
  }
  const confirmed = JSON.parse(confirmText) as KioskConfirmAnswer;
  expect(confirmed.id, 'confirmed request id').toBe(quote.id);
  expect(confirmed.status, 'confirmed request status').toBe('WAITING');
  expect(confirmed.queue, 'confirmed request queue number').toBeTruthy();
  expect(confirmed.cancelToken, 'confirmed request cancelToken').toBeTruthy();

  return {
    id: confirmed.id,
    queue: confirmed.queue,
    cancelToken: confirmed.cancelToken!,
  };
}

async function readKioskStatus(page: Page, id: string): Promise<KioskIdentityAnswer> {
  const res = await page.request.post('/api/kiosk', {
    headers: kioskHeaders,
    data: {
      action: 'status',
      payload: {
        id,
        branchId: users.branch,
      },
    },
  });
  const text = await res.text().catch(() => '');
  expect(res.ok(), `POST /api/kiosk status for ${id}: ${res.status()} ${text}`).toBeTruthy();
  return JSON.parse(text) as KioskIdentityAnswer;
}

async function cleanupKioskRequest(
  page: Page,
  id: string | null,
  tellerSignedIn: boolean,
): Promise<void> {
  if (!id) return;
  const current = await readKioskStatus(page, id);
  if (current.status === 'CANCELLED') return;

  if (!tellerSignedIn) {
    await signIn(page, 'teller', '/staff');
  }
  const res = await page.request.post('/api/kiosk', {
    headers: kioskHeaders,
    data: {
      action: 'cancel',
      payload: {
        id,
        branchId: users.branch,
        reason: 'OTHER',
        note: 'QA-F19 cleanup',
      },
    },
  });
  const text = await res.text().catch(() => '');
  expect(res.ok(), `cleanup cancel of kiosk request ${id}: ${res.status()} ${text}`).toBeTruthy();
}

test.describe('QA-F19 kiosk cancel', () => {
  test('staff cancels a waiting kiosk request from the queue panel', async ({ page }) => {
    let createdId: string | null = null;
    let tellerSignedIn = false;

    try {
      const created = await createWaitingKioskRequest(page);
      createdId = created.id;

      await signIn(page, 'teller', '/kiosk-queue.html');
      tellerSignedIn = true;

      const row = page.locator('article.xc-queue-card', {
        has: page.locator('h3', { hasText: created.queue }),
      });
      await expect(row, `waiting row ${created.queue} visible on /kiosk-queue.html`).toBeVisible({
        timeout: 20_000,
      });

      const cancelBtn = row.locator('button[data-action="cancel-request"]');
      await expect(cancelBtn, 'cancel-request button on row (QueuePanel.tsx:454-461)').toBeEnabled({
        timeout: 15_000,
      });
      await cancelBtn.click();

      const dialog = page.locator('dialog.xc-dialog');
      await expect(dialog, 'CancelReasonDialog open (CancelReasonDialog.tsx:61)').toBeVisible({
        timeout: 10_000,
      });

      await dialog.locator('select[name="reason"]').selectOption('OTHER');
      await dialog.locator('textarea[name="note"]').fill('Customer left the branch');

      const confirmBtn = dialog.locator('button[data-action="confirm-cancel-request"]');
      await expect(confirmBtn, 'confirm-cancel-request button enabled').toBeEnabled();
      await shot(page, 'desk-ops', 'kiosk-cancel-1-dialog');

      const cancelRes = await posted(page, /^\/api\/kiosk$/, () => confirmBtn.click());
      console.log(`[F19] staff cancel POST /api/kiosk ${cancelRes.text}`);
      expect(cancelRes.ok, `staff cancel accepted: ${cancelRes.text}`).toBeTruthy();

      await expect(dialog, 'CancelReasonDialog closes after confirm').toHaveCount(0, {
        timeout: 10_000,
      });
      await expect(row, `cancelled row ${created.queue} leaves the queue panel`).toHaveCount(0, {
        timeout: 20_000,
      });
      await shot(page, 'desk-ops', 'kiosk-cancel-2-panel-updated');

      const statusAfter = await readKioskStatus(page, created.id);
      expect(statusAfter.status, `status of ${created.id} after staff cancel`).toBe('CANCELLED');
      createdId = null;
    } finally {
      await cleanupKioskRequest(page, createdId, tellerSignedIn);
    }
  });

  test('customer cancels a waiting kiosk request with cancelToken; wrong token is refused', async ({
    page,
  }) => {
    let createdId: string | null = null;

    try {
      const created = await createWaitingKioskRequest(page);
      createdId = created.id;

      const wrongRes = await page.request.post('/api/kiosk', {
        headers: kioskHeaders,
        data: {
          action: 'customerCancel',
          payload: {
            id: created.id,
            cancelToken: 'wrong-token-00000000',
            branchId: users.branch,
          },
        },
      });
      const wrongText = await wrongRes.text();
      console.log(`[F19] wrong cancelToken POST /api/kiosk: ${wrongRes.status()} ${wrongText}`);
      expect(wrongRes.status(), `wrong cancelToken refused with 400: ${wrongText}`).toBe(400);
      const wrongBody = JSON.parse(wrongText) as { error?: string };
      expect(wrongBody.error, 'wrong cancelToken refusal sentence').toBe(
        'ยกเลิกคิวที่ตู้ไม่ได้ กรุณาติดต่อพนักงาน',
      );

      const statusAfterWrong = await readKioskStatus(page, created.id);
      expect(statusAfterWrong.status, 'status stays WAITING after wrong cancelToken').toBe(
        'WAITING',
      );

      const okRes = await page.request.post('/api/kiosk', {
        headers: kioskHeaders,
        data: {
          action: 'customerCancel',
          payload: {
            id: created.id,
            cancelToken: created.cancelToken,
            branchId: users.branch,
          },
        },
      });
      const okText = await okRes.text();
      console.log(`[F19] matching cancelToken POST /api/kiosk: ${okRes.status()} ${okText}`);
      expect(okRes.status(), `matching cancelToken accepted with 200: ${okText}`).toBe(200);
      const okBody = JSON.parse(okText) as KioskIdentityAnswer;
      expect(okBody.id, 'customerCancel response id').toBe(created.id);
      expect(okBody.status, 'customerCancel response status').toBe('CANCELLED');

      const statusAfterOk = await readKioskStatus(page, created.id);
      expect(statusAfterOk.status, 'status is CANCELLED after customerCancel').toBe('CANCELLED');
      createdId = null;
    } finally {
      await cleanupKioskRequest(page, createdId, false);
    }
  });
});
