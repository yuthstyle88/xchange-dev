// QA-F2 — flow: queue walk-in to finish (design docs/design/qa-e2e.md §3, §4; docs/design/blueprint/queue-tickets.md §3, §4.3, §4.4).
// Written by ticket #3873 (QA-F2); this file is its whole File scope.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/screens/desk/queue/:
// - CounterBar.tsx:67-74   button[name="issueWalkIn"]  (POST /api/queue/tickets, queue-commands.ts:36,54)
// - CounterBar.tsx:75-82   button[name="callNext"]     (POST /api/queue/call-next, queue-commands.ts:37,55)
// - CounterBar.tsx:114-126 button[name="openQueue"]    (POST /api/queue/open, queue-commands.ts:49,66)
// - CounterBar.tsx:127-139 button[name="closeQueue"]   (POST /api/queue/close, queue-commands.ts:50,66)
// - Board.tsx:43,69-71     li[data-ticket="<id>"]      (ticket row with displayNumber and status text; board/i18n.ts:11-12,25-26)
// - TicketActions.tsx:58-63,100-107 button[name="start"] / button[name="finish"] (POST /api/queue/tickets/{id}/start|finish, queue-commands.ts:56-62)
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

test.describe.serial('QA-F2 flow: queue walk-in to finish', () => {
  test('teller serves a walk-in from issue to finish', async ({ browser }) => {
    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const teller = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });

    try {
      // 1. Sign both in on /staff (skips via signIn when branchManager or teller is missing in users.json),
      //    close any leftover open shift, then open the teller's shift so the teller has an ACTIVE counter (queue-tickets.md Q7).
      await signIn(manager, 'branchManager', '/staff');
      await signIn(teller, 'teller', '/staff');
      await ensureShiftClosed(teller, manager);
      await openShift(teller, manager);

      let openedQueue = false;
      try {
        // 2. Branch manager opens /desk/queue; if the branch queue is closed, open it (POST /api/queue/open)
        //    and remember to close it again in finally.
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
        console.log(`[queue] branch=${users.branch} initial queueOpen=${wasQueueOpen}`);

        if (!wasQueueOpen) {
          const openRes = await posted(manager, /^\/api\/queue\/open$/, () => openQueueBtn.click());
          console.log(`[queue] POST /api/queue/open ${openRes.text}`);
          expect(openRes.ok, `open queue accepted: ${openRes.text}`).toBeTruthy();
          openedQueue = true;
        }

        // 3. Teller on /desk/queue: issue walk-in -> call next -> start -> finish.
        await teller.goto('/desk/queue');
        const issueBtn = teller.locator('button[name="issueWalkIn"]');
        const callNextBtn = teller.locator('button[name="callNext"]');
        await expect(issueBtn, 'teller issueWalkIn button (CounterBar.tsx:67-74)').toBeVisible({ timeout: 20_000 });
        await expect(callNextBtn, 'teller callNext button enabled with open shift (CounterBar.tsx:75-82)').toBeEnabled({
          timeout: 20_000,
        });
        await shot(teller, 'desk', 'queue-1-initial');

        const issued = await posted(teller, /^\/api\/queue\/tickets$/, () => issueBtn.click());
        console.log(`[queue] POST /api/queue/tickets ${issued.text}`);
        expect(issued.ok, `issue walk-in accepted: ${issued.text}`).toBeTruthy();
        const issuedTicket = JSON.parse(issued.body) as QueueTicketAnswer;
        console.log(`[queue] issued ticket: id=${issuedTicket.id} number=${issuedTicket.displayNumber}`);
        expect(issuedTicket.id, 'issued ticket id').toBeTruthy();
        expect(issuedTicket.displayNumber, 'issued ticket displayNumber').toBeTruthy();
        await shot(teller, 'desk', 'queue-2-issued');

        const called = await posted(teller, /^\/api\/queue\/call-next$/, () => callNextBtn.click());
        console.log(`[queue] POST /api/queue/call-next ${called.text}`);
        expect(called.ok, `call-next accepted: ${called.text}`).toBeTruthy();
        const calledTicket = JSON.parse(called.body) as QueueTicketAnswer;
        console.log(
          `[queue] called ticket: id=${calledTicket.id} number=${calledTicket.displayNumber} (issued number=${issuedTicket.displayNumber})`,
        );
        expect(calledTicket.id, 'called ticket id').toBeTruthy();
        expect(calledTicket.displayNumber, 'called ticket displayNumber').toBeTruthy();

        // Board row for the ticket returned by call-next (Board.tsx:69-71; board/i18n.ts:11,25).
        const row = teller.locator(`li[data-ticket="${calledTicket.id}"]`);
        await expect(row, `called ticket ${calledTicket.displayNumber} row on board`).toBeVisible({ timeout: 20_000 });
        await expect(row, `board shows ticket number ${calledTicket.displayNumber}`).toContainText(
          calledTicket.displayNumber,
        );
        await expect(row, `board shows ${calledTicket.displayNumber} as CALLED`).toContainText('CALLED', {
          timeout: 20_000,
        });
        await shot(teller, 'desk', 'queue-3-called');

        // Start serving (TicketActions.tsx:59,100-107; board/i18n.ts:12,26 'SERVING' | 'Serving').
        const startBtn = row.locator('button[name="start"]');
        await expect(startBtn, 'start button on CALLED ticket').toBeVisible({ timeout: 20_000 });
        const started = await posted(
          teller,
          new RegExp(`^/api/queue/tickets/${encodeURIComponent(calledTicket.id)}/start$`),
          () => startBtn.click(),
        );
        console.log(`[queue] POST /api/queue/tickets/${calledTicket.id}/start ${started.text}`);
        expect(started.ok, `start ticket accepted: ${started.text}`).toBeTruthy();
        await expect(row, `board shows ${calledTicket.displayNumber} as SERVING`).toContainText(/SERVING|Serving/, {
          timeout: 20_000,
        });
        await shot(teller, 'desk', 'queue-4-serving');

        // Finish serving (TicketActions.tsx:62,100-107): the completed ticket leaves the board.
        const finishBtn = row.locator('button[name="finish"]');
        await expect(finishBtn, 'finish button on SERVING ticket').toBeVisible({ timeout: 20_000 });
        const finished = await posted(
          teller,
          new RegExp(`^/api/queue/tickets/${encodeURIComponent(calledTicket.id)}/finish$`),
          () => finishBtn.click(),
        );
        console.log(`[queue] POST /api/queue/tickets/${calledTicket.id}/finish ${finished.text}`);
        expect(finished.ok, `finish ticket accepted: ${finished.text}`).toBeTruthy();
        await expect(row, `finished ticket ${calledTicket.displayNumber} leaves the board`).toHaveCount(0, {
          timeout: 20_000,
        });
        await shot(teller, 'desk', 'queue-5-finished');
      } finally {
        // 4. Always close the teller's shift and restore the queue state if step 2 opened it.
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
            console.log(`[queue] POST /api/queue/close ${closeRes.text}`);
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
