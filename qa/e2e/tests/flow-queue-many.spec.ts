// QA-F8 — flow: queue with several tickets (order of calls, skip = no-show, cancel)
// (design docs/design/qa-e2e.md §4 F8; docs/design/blueprint/queue-tickets.md §3, §4.3, §4.4).
// Written by ticket #4025 (QA-F8); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors and routes cited from web/src/screens/desk/queue/ and api/:
// - CounterBar.tsx:67-82,114-139  button[name="issueWalkIn"|"callNext"|"openQueue"|"closeQueue"]
// - Board.tsx:37-43,63-71         section (waiting / nowServing) > ul > li[data-ticket="<id>"] (board/i18n.ts:5,7,11-12,19,21,25-26)
// - TicketActions.tsx:55-65,100-141 button[name="skip"|"start"|"finish"|"cancel"], input[name="cancelReason"], button[name="cancelConfirm"]
// - queue-commands.ts:36-43,54-64 POST /api/queue/tickets, /api/queue/call-next, /api/queue/tickets/{id}/skip|start|finish|cancel
// - queue_board.rs:76-94          GET /api/v3/views/queue_board?branch=… (lists only WAITING, CALLED, SERVING)
import { expect, test } from '@playwright/test';
import { closeShift, ensureShiftClosed, openShift, posted, shot, signIn, users } from './helpers';

interface QueueTicketAnswer {
  id: string;
  displayNumber: string;
  status: string;
}

interface BranchServiceStatusEnvelope {
  data?: {
    queueOpen?: boolean;
  };
}

interface QueueBoardEnvelope {
  data?: {
    waiting?: Array<{
      id: string;
      displayNumber: string;
      serviceType: string;
      sequence: number;
    }>;
  };
}

test.describe.serial('QA-F8 flow: queue with several tickets (order, skip = no-show, cancel)', () => {
  test('teller issues three walk-ins, skips A (no-show), serves B to finish, sees C on second staff page, and cancels C', async ({
    browser,
  }) => {
    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const teller = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });

    try {
      // 1. Sign in teller + branch manager on /staff, open the teller's shift, and open the branch queue if closed.
      await signIn(manager, 'branchManager', '/staff');
      await signIn(teller, 'teller', '/staff');
      await ensureShiftClosed(teller, manager);
      await openShift(teller, manager);

      let openedQueue = false;
      try {
        await manager.goto('/desk/queue');
        const openQueueBtn = manager.locator('button[name="openQueue"]');
        await expect(openQueueBtn, 'branch manager openQueue button (CounterBar.tsx:114-126)').toBeVisible({
          timeout: 20_000,
        });

        const statusRes = await manager.request.get(
          `/api/v3/views/branch_service_status?branch=${encodeURIComponent(users.branch)}`,
        );
        expect(
          statusRes.ok(),
          `GET /api/v3/views/branch_service_status: ${statusRes.status()} ${await statusRes.text().catch(() => '')}`,
        ).toBeTruthy();
        const statusBody = (await statusRes.json()) as BranchServiceStatusEnvelope;
        const wasQueueOpen = Boolean(statusBody.data?.queueOpen);
        console.log(`[queue-many] branch=${users.branch} initial queueOpen=${wasQueueOpen}`);

        if (!wasQueueOpen) {
          const openRes = await posted(manager, /^\/api\/queue\/open$/, () => openQueueBtn.click());
          console.log(`[queue-many] POST /api/queue/open ${openRes.text}`);
          expect(openRes.ok, `open queue accepted: ${openRes.text}`).toBeTruthy();
          openedQueue = true;
        }

        // 2. Read GET /api/v3/views/queue_board?branch=… and assert waiting is empty
        //    (never call someone else's leftover ticket; list leftover display numbers on failure).
        const boardRes = await teller.request.get(
          `/api/v3/views/queue_board?branch=${encodeURIComponent(users.branch)}`,
        );
        expect(
          boardRes.ok(),
          `GET /api/v3/views/queue_board: ${boardRes.status()} ${await boardRes.text().catch(() => '')}`,
        ).toBeTruthy();
        const boardBody = (await boardRes.json()) as QueueBoardEnvelope;
        const leftoverNumbers = (boardBody.data?.waiting ?? []).map((w) => w.displayNumber);
        expect(
          leftoverNumbers,
          `waiting queue must be empty before test (leftover display numbers: ${leftoverNumbers.join(', ')})`,
        ).toEqual([]);

        // 3. Teller on /desk/queue: issue three walk-ins A, B, C.
        await teller.goto('/desk/queue');
        const issueBtn = teller.locator('button[name="issueWalkIn"]');
        const callNextBtn = teller.locator('button[name="callNext"]');
        await expect(issueBtn, 'teller issueWalkIn button (CounterBar.tsx:67-74)').toBeVisible({ timeout: 20_000 });
        await expect(callNextBtn, 'teller callNext button enabled with open shift (CounterBar.tsx:75-82)').toBeEnabled({
          timeout: 20_000,
        });

        const issueWalkIn = async (label: string): Promise<QueueTicketAnswer> => {
          await expect(issueBtn, `issueWalkIn enabled before issuing ${label}`).toBeEnabled({ timeout: 20_000 });
          const res = await posted(teller, /^\/api\/queue\/tickets$/, () => issueBtn.click());
          console.log(`[queue-many] issue ${label}: POST /api/queue/tickets ${res.text}`);
          expect(res.ok, `issue walk-in ${label} accepted: ${res.text}`).toBeTruthy();
          const ticket = JSON.parse(res.body) as QueueTicketAnswer;
          expect(ticket.id, `issued ticket ${label} id`).toBeTruthy();
          expect(ticket.displayNumber, `issued ticket ${label} displayNumber`).toBeTruthy();
          await expect(
            teller.locator(`li[data-ticket="${ticket.id}"]`),
            `issued ticket ${label} (${ticket.displayNumber}) row visible on board`,
          ).toBeVisible({ timeout: 20_000 });
          return ticket;
        };

        const ticketA = await issueWalkIn('A');
        const ticketB = await issueWalkIn('B');
        const ticketC = await issueWalkIn('C');
        await shot(teller, 'desk', 'queue-many-1-issued-three');

        // 4. Teller calls next -> called id = A; A's row is CALLED.
        await expect(callNextBtn, 'callNext enabled before calling A').toBeEnabled({ timeout: 20_000 });
        const calledARes = await posted(teller, /^\/api\/queue\/call-next$/, () => callNextBtn.click());
        console.log(`[queue-many] callNext (expect A): POST /api/queue/call-next ${calledARes.text}`);
        expect(calledARes.ok, `first call-next accepted: ${calledARes.text}`).toBeTruthy();
        const calledA = JSON.parse(calledARes.body) as QueueTicketAnswer;
        expect(calledA.id, `first call-next returns ticket A (${ticketA.displayNumber})`).toBe(ticketA.id);

        const rowA = teller.locator(`li[data-ticket="${ticketA.id}"]`);
        await expect(rowA, `ticket A (${ticketA.displayNumber}) row visible`).toBeVisible({ timeout: 20_000 });
        await expect(rowA, `ticket A row shows ${ticketA.displayNumber}`).toContainText(ticketA.displayNumber);
        await expect(rowA, `ticket A row shows CALLED`).toContainText('CALLED', { timeout: 20_000 });
        await shot(teller, 'desk', 'queue-many-2-called-a');

        // 5. Teller skips A (POST /api/queue/tickets/{id}/skip -> NO_SHOW, final): A's row leaves the board.
        const skipBtnA = rowA.locator('button[name="skip"]');
        await expect(skipBtnA, 'skip button on CALLED ticket A (TicketActions.tsx:59,100-107)').toBeVisible({
          timeout: 20_000,
        });
        const skippedARes = await posted(
          teller,
          new RegExp(`^/api/queue/tickets/${encodeURIComponent(ticketA.id)}/skip$`),
          () => skipBtnA.click(),
        );
        console.log(`[queue-many] skip A: POST /api/queue/tickets/${ticketA.id}/skip ${skippedARes.text}`);
        expect(skippedARes.ok, `skip ticket A accepted: ${skippedARes.text}`).toBeTruthy();
        await expect(rowA, `skipped ticket A (${ticketA.displayNumber}) is gone from the board`).toHaveCount(0, {
          timeout: 20_000,
        });
        await shot(teller, 'desk', 'queue-many-3-skipped-a');

        // 6. Teller calls next -> called id = B; start -> finish.
        await expect(callNextBtn, 'callNext enabled before calling B').toBeEnabled({ timeout: 20_000 });
        const calledBRes = await posted(teller, /^\/api\/queue\/call-next$/, () => callNextBtn.click());
        console.log(`[queue-many] callNext (expect B): POST /api/queue/call-next ${calledBRes.text}`);
        expect(calledBRes.ok, `second call-next accepted: ${calledBRes.text}`).toBeTruthy();
        const calledB = JSON.parse(calledBRes.body) as QueueTicketAnswer;
        expect(calledB.id, `second call-next returns ticket B (${ticketB.displayNumber})`).toBe(ticketB.id);

        const rowB = teller.locator(`li[data-ticket="${ticketB.id}"]`);
        await expect(rowB, `ticket B (${ticketB.displayNumber}) row shows CALLED`).toContainText('CALLED', {
          timeout: 20_000,
        });

        const startBtnB = rowB.locator('button[name="start"]');
        await expect(startBtnB, 'start button on CALLED ticket B').toBeVisible({ timeout: 20_000 });
        const startedBRes = await posted(
          teller,
          new RegExp(`^/api/queue/tickets/${encodeURIComponent(ticketB.id)}/start$`),
          () => startBtnB.click(),
        );
        console.log(`[queue-many] start B: POST /api/queue/tickets/${ticketB.id}/start ${startedBRes.text}`);
        expect(startedBRes.ok, `start ticket B accepted: ${startedBRes.text}`).toBeTruthy();
        await expect(rowB, `ticket B (${ticketB.displayNumber}) row shows SERVING`).toContainText(/SERVING|Serving/, {
          timeout: 20_000,
        });

        const finishBtnB = rowB.locator('button[name="finish"]');
        await expect(finishBtnB, 'finish button on SERVING ticket B').toBeVisible({ timeout: 20_000 });
        const finishedBRes = await posted(
          teller,
          new RegExp(`^/api/queue/tickets/${encodeURIComponent(ticketB.id)}/finish$`),
          () => finishBtnB.click(),
        );
        console.log(`[queue-many] finish B: POST /api/queue/tickets/${ticketB.id}/finish ${finishedBRes.text}`);
        expect(finishedBRes.ok, `finish ticket B accepted: ${finishedBRes.text}`).toBeTruthy();
        await expect(rowB, `finished ticket B (${ticketB.displayNumber}) leaves the board`).toHaveCount(0, {
          timeout: 20_000,
        });
        await shot(teller, 'desk', 'queue-many-4-finished-b');

        // 7. Teller calls next -> called id = C.
        await expect(callNextBtn, 'callNext enabled before calling C').toBeEnabled({ timeout: 20_000 });
        const calledCRes = await posted(teller, /^\/api\/queue\/call-next$/, () => callNextBtn.click());
        console.log(`[queue-many] callNext (expect C): POST /api/queue/call-next ${calledCRes.text}`);
        expect(calledCRes.ok, `third call-next accepted: ${calledCRes.text}`).toBeTruthy();
        const calledC = JSON.parse(calledCRes.body) as QueueTicketAnswer;
        expect(calledC.id, `third call-next returns ticket C (${ticketC.displayNumber})`).toBe(ticketC.id);

        const rowC = teller.locator(`li[data-ticket="${ticketC.id}"]`);
        await expect(rowC, `ticket C (${ticketC.displayNumber}) row shows CALLED`).toContainText('CALLED', {
          timeout: 20_000,
        });

        // 8. Manager on /desk/queue (second staff page): C's number shows under nowServing (Board.tsx:63-71).
        await manager.goto('/desk/queue');
        const managerNowServing = manager.locator('section', {
          has: manager.locator('h3', { hasText: /กำลังให้บริการ|Now serving/ }),
        });
        const managerRowC = managerNowServing.locator(`li[data-ticket="${ticketC.id}"]`);
        await expect(managerRowC, `second staff page shows ticket C under nowServing`).toBeVisible({ timeout: 20_000 });
        await expect(managerRowC, `second staff page shows C's number ${ticketC.displayNumber}`).toContainText(
          ticketC.displayNumber,
        );
        await expect(managerRowC, `second staff page shows C as CALLED`).toContainText('CALLED');
        await shot(manager, 'desk', 'queue-many-5-manager-now-serving-c');

        // 9. Teller cancels C with a reason, clicks cancelConfirm -> ok; C is gone from the board.
        const cancelBtnC = rowC.locator('button[name="cancel"]');
        await expect(cancelBtnC, 'cancel button on CALLED ticket C (TicketActions.tsx:111-121)').toBeVisible({
          timeout: 20_000,
        });
        await cancelBtnC.click();

        const reasonInputC = rowC.locator('input[name="cancelReason"]');
        await expect(reasonInputC, 'cancelReason input on ticket C (TicketActions.tsx:126-132)').toBeVisible({
          timeout: 20_000,
        });
        await reasonInputC.fill('Customer left the branch');

        const cancelConfirmBtnC = rowC.locator('button[name="cancelConfirm"]');
        await expect(cancelConfirmBtnC, 'cancelConfirm button on ticket C (TicketActions.tsx:134-141)').toBeVisible({
          timeout: 20_000,
        });
        const cancelledCRes = await posted(
          teller,
          new RegExp(`^/api/queue/tickets/${encodeURIComponent(ticketC.id)}/cancel$`),
          () => cancelConfirmBtnC.click(),
        );
        console.log(`[queue-many] cancel C: POST /api/queue/tickets/${ticketC.id}/cancel ${cancelledCRes.text}`);
        expect(cancelledCRes.ok, `cancel ticket C accepted: ${cancelledCRes.text}`).toBeTruthy();
        await expect(rowC, `cancelled ticket C (${ticketC.displayNumber}) is gone from the board`).toHaveCount(0, {
          timeout: 20_000,
        });
        await shot(teller, 'desk', 'queue-many-6-cancelled-c');
      } finally {
        // 10. Always close the teller's shift and close the queue only if step 1 opened it.
        try {
          await teller.goto('/staff');
          await manager.goto('/staff');
          await closeShift(teller, manager);
        } finally {
          if (openedQueue) {
            await manager.goto('/desk/queue');
            const closeQueueBtn = manager.locator('button[name="closeQueue"]');
            await expect(closeQueueBtn, 'branch manager closeQueue button (CounterBar.tsx:127-139)').toBeVisible({
              timeout: 20_000,
            });
            const closeRes = await posted(manager, /^\/api\/queue\/close$/, () => closeQueueBtn.click());
            console.log(`[queue-many] POST /api/queue/close ${closeRes.text}`);
            expect(closeRes.ok, `close queue accepted: ${closeRes.text}`).toBeTruthy();
          }
        }
      }
    } finally {
      await teller.close().catch(() => {});
      await manager.close().catch(() => {});
    }
  });
});
