// J1 walk-in day journey: open shift, walk-ins take numbers, called, BUY then SELL at the desk, close balanced
// (ticket #5795; owner order relayed by claude/608dff; design docs/design/qa-e2e.md; docs/design/walkin-queue.md).
// Written by ticket #5795; this file is in its File scope.
// Import only from './helpers' and './desk-exchange' (#5795). Never run against a host from a worker's machine (E-3): check with --list and tsc.
//
// Selectors cited from web/src/:
// - shared/ui/AccountBar.tsx:153,161,170                 .account-bar, span, button (t('shell.account.branch'))
// - screens/desk/queue/counter/CounterBar.tsx:67-74     button[name="issueWalkIn"] (POST /api/queue/tickets)
// - screens/desk/queue/board/Board.tsx:43,69-71         li[data-ticket="<id>"] (ticket row, displayNumber, CALLED)
// - screens/desk/queue/actions/TicketActions.tsx:58-59,105-113 button[name="call"] (POST /api/queue/tickets/{id}/call)
// - screens/desk/steps/receipt/step.tsx:32-34           section.desk-form strong (/^XC-\d{8}-\d{6}$/)
import { expect, test, type Page } from '@playwright/test';
import { runDeskExchange } from './desk-exchange';
import {
  closeQueueIfOpened,
  closeShift,
  cutOff,
  ensureQueueOpen,
  hasAccount,
  openShift,
  posted,
  signIn,
  startService,
  users,
  type Role,
} from './helpers';

const EXCHANGE_AMOUNT = '1';

interface MeViewAnswer {
  data?: {
    branch?: string;
    branchName?: string;
  };
}

interface StaffMeViewAnswer {
  data?: {
    branchId?: string;
    branchName?: string;
  };
}

interface MyShiftBalance {
  code: string;
  opening: string;
  expected: string;
}

interface MyShiftAnswer {
  data?: {
    id: string;
    status: string;
    openedAt: string;
    balances: MyShiftBalance[];
  } | null;
}

/** Asserts the page operates under users.branch via /api/v3/views/me, staff_me, and .account-bar on screen. */
async function assertActingBranch(page: Page, label: string): Promise<void> {
  const meRes = await page.request.get('/api/v3/views/me');
  expect(meRes.ok(), `[${label}] GET /api/v3/views/me: ${meRes.status()}`).toBeTruthy();
  const meBody = (await meRes.json().catch(() => ({}))) as MeViewAnswer;
  const meBranch = meBody.data?.branch;
  const branchName = meBody.data?.branchName ?? '';
  expect(
    meBranch,
    `assertActingBranch [${label}] me.branch mismatch: got "${meBranch}", expected "${users.branch}"`,
  ).toBe(users.branch);

  const staffRes = await page.request.get('/api/v3/views/staff_me');
  expect(staffRes.ok(), `[${label}] GET /api/v3/views/staff_me: ${staffRes.status()}`).toBeTruthy();
  const staffBody = (await staffRes.json().catch(() => ({}))) as StaffMeViewAnswer;
  const staffBranchId = staffBody.data?.branchId;
  expect(
    staffBranchId,
    `assertActingBranch [${label}] staff_me.branchId mismatch: got "${staffBranchId}", expected "${users.branch}"`,
  ).toBe(users.branch);

  const accountBar = page.locator('.account-bar');
  await expect(accountBar, `assertActingBranch [${label}] .account-bar visible`).toBeVisible({ timeout: 20_000 });
  const escapedName = branchName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  await expect(
    accountBar,
    `assertActingBranch [${label}] .account-bar shows branch "${branchName}"`,
  ).toContainText(new RegExp(`(สาขา|Branch|分店)\\s+${escapedName}`));
}

/** Reads the teller's active shift from GET /api/v3/views/my_shift. */
async function readMyShift(page: Page): Promise<NonNullable<MyShiftAnswer['data']> | null> {
  const res = await page.request.get(`/api/v3/views/my_shift?branch=${encodeURIComponent(users.branch)}`);
  expect(res.ok(), `GET /api/v3/views/my_shift: ${res.status()}`).toBeTruthy();
  const body = (await res.json().catch(() => ({}))) as MyShiftAnswer;
  return body.data ?? null;
}

test.describe.serial('J1 walk-in day', () => {
  test('J1 walk-in day: open shift, two walk-ins take a number, are called, BUY then SELL at the desk, close shift balanced', async ({ browser }) => {
    test.setTimeout(300_000);

    const requiredRoles: readonly Role[] = ['teller', 'branchManager'];
    for (const role of requiredRoles) {
      if (!hasAccount(role)) {
        const reason = `no ${role} account in users.json: run scripts/reseed.sh`;
        console.log(`[J1] skip: ${reason}`);
        test.skip(true, reason);
      }
    }

    const teller = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    let openedQueue = false;

    try {
      // Step 1: Role guard done above; signIn both to /staff and assertActingBranch on both.
      await signIn(teller, 'teller', '/staff');
      await signIn(manager, 'branchManager', '/staff');
      await assertActingBranch(teller, 'teller on /staff');
      await assertActingBranch(manager, 'manager on /staff');

      // Step 2: Open shift, start service, ensure queue open.
      await openShift(teller, manager);
      let started: 'started' | 'alreadyOpen' | undefined;

      try {
        started = await startService(manager);
        openedQueue = await ensureQueueOpen(manager);

        const currency = users.currency ?? 'USD';

        // Step 3: Walk-in A takes a number on /desk/queue.
        await teller.goto('/desk/queue');
        await assertActingBranch(teller, 'teller on /desk/queue before walk-in A write');

        const issueBtn = teller.locator('button[name="issueWalkIn"]');
        await expect(issueBtn, 'teller issueWalkIn button (CounterBar.tsx:67-74)').toBeVisible({ timeout: 20_000 });
        const issuedA = await posted(teller, /^\/api\/queue\/tickets$/, () => issueBtn.click());
        console.log(`[J1] POST /api/queue/tickets (A) ${issuedA.text}`);
        expect(issuedA.ok, `issue walk-in A: ${issuedA.text}`).toBeTruthy();
        const ticketA = JSON.parse(issuedA.body) as { id: string; displayNumber: string };
        expect(ticketA.id, 'issued ticket A id').toBeTruthy();
        expect(ticketA.displayNumber, 'issued ticket A displayNumber').toBeTruthy();

        // Step 4: Call A on the board row li[data-ticket="<ticketA.id>"].
        const rowA = teller.locator(`li[data-ticket="${ticketA.id}"]`);
        await expect(rowA, `ticket A ${ticketA.displayNumber} row on board (Board.tsx:43)`).toBeVisible({ timeout: 20_000 });
        const callBtnA = rowA.locator('button[name="call"]');
        await expect(callBtnA, 'call button for ticket A (TicketActions.tsx:58-59,105-113)').toBeVisible({ timeout: 20_000 });
        const calledA = await posted(teller, /\/call$/, () => callBtnA.click());
        console.log(`[J1] POST /api/queue/tickets/${ticketA.id}/call ${calledA.text}`);
        expect(calledA.ok, `call ticket A: ${calledA.text}`).toBeTruthy();
        await expect(rowA, `board shows ticket A ${ticketA.displayNumber} as CALLED (Board.tsx:69-71)`).toContainText('CALLED', {
          timeout: 20_000,
        });

        // Record initial currency balance before step 5 (BUY)
        const shiftBeforeBuy = await readMyShift(teller);
        const initialCurrencyBalance = shiftBeforeBuy?.balances.find((b) => b.code === currency)?.expected ?? '0';

        // Step 5: BUY for A: assertActingBranch(teller), set up waits, runDeskExchange(..., 'BUY').
        await assertActingBranch(teller, 'teller before BUY sale');

        // These waits are awaited only after runDeskExchange returns; 120 s (inside the 300 s test) keeps a slow but healthy
        // sale from rejecting them unawaited, which Playwright reports as a failure (the #5789 review finding).
        const servePromiseA = teller.waitForResponse((res) => {
          try {
            const url = new URL(res.url());
            return res.request().method() === 'POST' && /^\/api\/queue\/tickets\/serve$/.test(url.pathname);
          } catch {
            return false;
          }
        }, { timeout: 120_000 });

        const createTxPromiseA = teller.waitForRequest((req) => {
          try {
            const url = new URL(req.url());
            return req.method() === 'POST' && /^\/api\/transactions$/.test(url.pathname);
          } catch {
            return false;
          }
        }, { timeout: 120_000 });

        const buyReceipt = await runDeskExchange(teller, manager, 'BUY', (opened) => {
          if (opened) openedQueue = true;
        });
        expect(buyReceipt, 'BUY receipt documentRef matches pattern (steps/receipt/step.tsx:32-34)').toMatch(
          /^XC-\d{8}-\d{6}$/,
        );

        const serveResA = await servePromiseA;
        expect(serveResA.ok(), `POST /api/queue/tickets/serve status: ${serveResA.status()}`).toBeTruthy();
        const serveBodyA = (await serveResA.json().catch(() => ({}))) as { id?: string };
        expect(serveBodyA.id, `serve response id matches ticket A id (${ticketA.id})`).toBe(ticketA.id);

        const createReqA = await createTxPromiseA;
        const postDataA = createReqA.postDataJSON() as { queueTicketId?: string };
        expect(postDataA.queueTicketId, `create transaction queueTicketId matches ticket A id (${ticketA.id})`).toBe(
          ticketA.id,
        );

        // Step 6: A's ticket finished by the sale: on /desk/queue, li[data-ticket="<ticketA.id>"] has count 0.
        await teller.goto('/desk/queue');
        await expect(
          teller.locator(`li[data-ticket="${ticketA.id}"]`),
          `finished ticket A ${ticketA.displayNumber} leaves the board (W8; flow-queue.spec.ts:160-162)`,
        ).toHaveCount(0, { timeout: 20_000 });

        // Step 7: Walk-in B: repeat steps 3-6 with a new ticket and runDeskExchange(..., 'SELL').
        // Before it, GET my_shift must show the teller holding >= EXCHANGE_AMOUNT of the currency.
        const shiftBeforeSell = await readMyShift(teller);
        const buyBalanceRow = shiftBeforeSell?.balances.find((b) => b.code === currency);
        const buyBalance = buyBalanceRow?.expected ?? '0';
        console.log(`[J1] balance before SELL (after BUY) ${currency}: ${buyBalance}`);
        expect(
          Number(buyBalance),
          `expected positive ${currency} balance before SELL: ${buyBalance}`,
        ).toBeGreaterThanOrEqual(Number(EXCHANGE_AMOUNT));

        // Walk-in B takes a number on /desk/queue
        await assertActingBranch(teller, 'teller on /desk/queue before walk-in B write');
        const issuedB = await posted(teller, /^\/api\/queue\/tickets$/, () => issueBtn.click());
        console.log(`[J1] POST /api/queue/tickets (B) ${issuedB.text}`);
        expect(issuedB.ok, `issue walk-in B: ${issuedB.text}`).toBeTruthy();
        const ticketB = JSON.parse(issuedB.body) as { id: string; displayNumber: string };
        expect(ticketB.id, 'issued ticket B id').toBeTruthy();
        expect(ticketB.displayNumber, 'issued ticket B displayNumber').toBeTruthy();

        // Call B
        const rowB = teller.locator(`li[data-ticket="${ticketB.id}"]`);
        await expect(rowB, `ticket B ${ticketB.displayNumber} row on board (Board.tsx:43)`).toBeVisible({ timeout: 20_000 });
        const callBtnB = rowB.locator('button[name="call"]');
        await expect(callBtnB, 'call button for ticket B (TicketActions.tsx:58-59,105-113)').toBeVisible({ timeout: 20_000 });
        const calledB = await posted(teller, /\/call$/, () => callBtnB.click());
        console.log(`[J1] POST /api/queue/tickets/${ticketB.id}/call ${calledB.text}`);
        expect(calledB.ok, `call ticket B: ${calledB.text}`).toBeTruthy();
        await expect(rowB, `board shows ticket B ${ticketB.displayNumber} as CALLED (Board.tsx:69-71)`).toContainText('CALLED', {
          timeout: 20_000,
        });

        // SELL for B
        await assertActingBranch(teller, 'teller before SELL sale');

        const servePromiseB = teller.waitForResponse((res) => {
          try {
            const url = new URL(res.url());
            return res.request().method() === 'POST' && /^\/api\/queue\/tickets\/serve$/.test(url.pathname);
          } catch {
            return false;
          }
        }, { timeout: 120_000 });

        const createTxPromiseB = teller.waitForRequest((req) => {
          try {
            const url = new URL(req.url());
            return req.method() === 'POST' && /^\/api\/transactions$/.test(url.pathname);
          } catch {
            return false;
          }
        }, { timeout: 120_000 });

        const sellReceipt = await runDeskExchange(teller, manager, 'SELL', (opened) => {
          if (opened) openedQueue = true;
        });
        expect(sellReceipt, 'SELL receipt documentRef matches pattern (steps/receipt/step.tsx:32-34)').toMatch(
          /^XC-\d{8}-\d{6}$/,
        );

        const serveResB = await servePromiseB;
        expect(serveResB.ok(), `POST /api/queue/tickets/serve status: ${serveResB.status()}`).toBeTruthy();
        const serveBodyB = (await serveResB.json().catch(() => ({}))) as { id?: string };
        expect(serveBodyB.id, `serve response id matches ticket B id (${ticketB.id})`).toBe(ticketB.id);

        const createReqB = await createTxPromiseB;
        const postDataB = createReqB.postDataJSON() as { queueTicketId?: string };
        expect(postDataB.queueTicketId, `create transaction queueTicketId matches ticket B id (${ticketB.id})`).toBe(
          ticketB.id,
        );

        // B's ticket finished by the sale
        await teller.goto('/desk/queue');
        await expect(
          teller.locator(`li[data-ticket="${ticketB.id}"]`),
          `finished ticket B ${ticketB.displayNumber} leaves the board (W8; flow-queue.spec.ts:160-162)`,
        ).toHaveCount(0, { timeout: 20_000 });

        // After SELL: currency balance is back to its value before step 5
        const shiftAfterSell = await readMyShift(teller);
        const sellBalanceRow = shiftAfterSell?.balances.find((b) => b.code === currency);
        const sellBalance = sellBalanceRow?.expected ?? '0';
        console.log(`[J1] balance after SELL ${currency}: ${sellBalance}`);
        expect(
          Number(sellBalance),
          `${currency} balance back to value before step 5 (${initialCurrencyBalance}) after SELL: ${sellBalance}`,
        ).toBe(Number(initialCurrencyBalance));
      } finally {
        // Step 8: Close and balance in finally
        await closeQueueIfOpened(manager, openedQueue).catch(() => {});
        if (started === 'started') await cutOff(manager);
        await teller.goto('/staff');
        await manager.goto('/staff');
        await closeShift(teller, manager);
        const closedShift = await readMyShift(teller);
        expect(closedShift, 'GET my_shift is null after closeShift').toBeNull();
      }
    } finally {
      await teller.close().catch(() => {});
      await manager.close().catch(() => {});
    }
  });
});
