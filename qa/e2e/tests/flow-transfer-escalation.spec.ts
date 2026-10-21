// Flow F12 (docs/design/qa-e2e.md §4 F12, ticket #4030):
//   Escalated cash-transfer approval above the THB policy limit -> branch manager refused (ABOVE_LIMIT),
//   admin (EXECUTIVE) approves -> finance dispatches and receives EXTERNAL_IN, then nets the safe back to
//   where it started with a matching EXTERNAL_OUT approved by admin, dispatched and received.
//   Never saves a transfer policy (E-4). Never run against a host from a worker's machine (E-3).
import { expect, test, type Browser, type Page } from '@playwright/test';
import { hasAccount, posted, shot, signIn, users, type Role } from './helpers';

const REQUIRED: Role[] = ['finance', 'branchManager', 'admin'];

const transferRow = (page: Page, id: string) => page.locator(`tr[data-transfer-id="${id}"]`);

async function asRole(browser: Browser, role: Role, next: string): Promise<Page> {
  const page = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
  await signIn(page, role, next);
  return page;
}

async function createAndSubmitTransfer(
  finance: Page,
  kind: 'EXTERNAL_IN' | 'EXTERNAL_OUT',
  qty: string,
  note: string,
): Promise<string> {
  await finance.getByTestId('transfer-new').click();
  const form = finance.getByTestId('transfer-form');
  await expect(form).toBeVisible({ timeout: 15_000 });
  await form.getByTestId('transfer-kind').selectOption(kind);
  await form.getByTestId('transfer-note').fill(note);
  const thbRows = form.locator('[data-denomination-row]').filter({ hasText: 'THB' });
  await expect(thbRows.first(), 'THB rows in the transfer form').toBeVisible({ timeout: 20_000 });
  const thbCount = await thbRows.count();
  for (let i = 0; i < thbCount; i++) {
    await thbRows.nth(i).locator('input[data-denomination-id]').fill(qty);
  }
  const submitted = finance.waitForResponse(
    (r) => r.request().method() === 'POST' && /\/api\/cash-transfers\/[^/]+\/submit$/.test(new URL(r.url()).pathname),
    { timeout: 15_000 },
  );
  const created = await posted(finance, /^\/api\/cash-transfers$/, () => form.getByTestId('transfer-save').click());
  console.log(`[f12:${kind}] POST /api/cash-transfers ${created.text}`);
  expect(created.ok, `${kind} transfer created: ${created.text}`).toBeTruthy();
  const id = String(JSON.parse(created.body).id ?? '');
  expect(id, `new ${kind} transfer id`).not.toBe('');
  const sub = await submitted;
  const subText = `${sub.status()} ${(await sub.text().catch(() => '')).slice(0, 300)}`;
  console.log(`[f12:${kind}] submit ${subText}`);
  expect(sub.ok(), `${kind} transfer submitted: ${subText}`).toBeTruthy();
  await finance.reload();
  await expect(transferRow(finance, id)).toBeVisible({ timeout: 20_000 });
  return id;
}

async function dispatchAndReceiveTransfer(finance: Page, id: string, qty: string): Promise<void> {
  const row = transferRow(finance, id);
  await finance.reload();
  await expect(row, `transfer ${id} is APPROVED`).toContainText('APPROVED', { timeout: 20_000 });
  const dispatched = await posted(finance, /\/api\/cash-transfers\/[^/]+\/dispatch$/, () =>
    row.getByTestId('transfer-dispatch').click(),
  );
  console.log(`[f12:${id}] dispatch ${dispatched.text}`);
  expect(dispatched.ok, `transfer ${id} dispatched: ${dispatched.text}`).toBeTruthy();
  await finance.reload();
  await expect(row, `transfer ${id} is DISPATCHED`).toContainText('DISPATCHED', { timeout: 20_000 });

  await row.getByTestId('transfer-receive').click();
  const recv = finance.getByTestId('transfer-receive-form');
  await expect(recv).toBeVisible({ timeout: 15_000 });
  const recvRows = recv.locator('[data-denomination-row]').filter({ hasText: 'THB' });
  await expect(recvRows.first(), 'THB rows in the receive form').toBeVisible({ timeout: 20_000 });
  const recvCount = await recvRows.count();
  for (let i = 0; i < recvCount; i++) {
    await recvRows.nth(i).locator('input[data-denomination-id]').fill(qty);
  }
  const received = await posted(finance, /\/api\/cash-transfers\/[^/]+\/receive$/, () =>
    recv.getByTestId('transfer-receive-save').click(),
  );
  console.log(`[f12:${id}] receive ${received.text}`);
  expect(received.ok, `transfer ${id} received: ${received.text}`).toBeTruthy();
  await finance.reload();
  await expect(row, `transfer ${id} is RECEIVED`).toContainText('RECEIVED', { timeout: 20_000 });
}

async function findPendingTransferRequestId(admin: Page, transferId: string): Promise<string> {
  const res = await admin.request.get(
    `/api/v3/views/approval_requests?branch=${encodeURIComponent(users.branch)}&status=PENDING&type=CASH_TRANSFER`,
  );
  expect(res.ok(), `admin reads approval_requests: ${res.status()}`).toBeTruthy();
  const body = (await res.json()) as {
    data?: { requests?: Array<{ id: string; objectId: string; status: string }> };
  };
  const match = body.data?.requests?.find((r) => r.objectId === transferId && r.status === 'PENDING');
  expect(match?.id, `pending CASH_TRANSFER approval request for transfer ${transferId}`).toBeTruthy();
  return match!.id;
}

async function approveOnAdminScreen(admin: Page, reqId: string): Promise<void> {
  await admin.goto('/#approvals');
  const filterBtn = admin.locator('[data-type-filter="CASH_TRANSFER"]');
  await expect(filterBtn).toBeVisible({ timeout: 20_000 });
  await filterBtn.click();
  const card = admin
    .locator(`[data-request="${reqId}"]`)
    .filter({ has: admin.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }) })
    .first();
  await expect(card, `pending CASH_TRANSFER card ${reqId} on admin /#approvals`).toBeVisible({ timeout: 20_000 });
  const decided = await posted(admin, /^\/api\/approvals\/[^/]+\/decide$/, () =>
    card.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }).click(),
  );
  console.log(`[f12:admin] decide ${reqId} -> ${decided.text}`);
  expect(decided.ok, `admin approved ${reqId}: ${decided.text}`).toBeTruthy();
}

test.describe.serial('F12 cash transfer escalation above limit', () => {
  test('above-limit EXTERNAL_IN refuses branch manager (ABOVE_LIMIT), admin approves, EXTERNAL_OUT restores the safe', async ({
    browser,
  }) => {
    const missing = REQUIRED.filter((r) => !hasAccount(r));
    test.skip(
      missing.length > 0,
      `missing password in users.json for: ${missing.map((r) => `${r} (${users[r]?.identifier ?? users[r]?.phone ?? 'unmapped'})`).join(', ')}`,
    );

    const admin = await asRole(browser, 'admin', '/');
    const finance = await asRole(browser, 'finance', '/staff');
    const manager = await asRole(browser, 'branchManager', '/');

    try {
      // Step 0 (admin): read cash_transfer_policy and pick THB lines over the limit (null policy -> any amount).
      const policyRes = await admin.request.get('/api/v3/views/cash_transfer_policy');
      expect(policyRes.ok(), `GET /api/v3/views/cash_transfer_policy (${policyRes.status()})`).toBeTruthy();
      const policyBody = (await policyRes.json()) as {
        data?: { limits?: Record<string, string> } | null;
      };
      const denomRes = await admin.request.get('/api/v3/views/denominations');
      expect(denomRes.ok(), `GET /api/v3/views/denominations (${denomRes.status()})`).toBeTruthy();
      const denomBody = (await denomRes.json()) as {
        data?: { rows?: Array<{ currency: string; faceValue: string }> };
      };
      const thbUnitSum =
        denomBody.data?.rows
          ?.filter((d) => d.currency === 'THB')
          .reduce((sum, d) => sum + Number(d.faceValue), 0) || 1688.75;
      const rawThbLimit = policyBody.data?.limits?.THB;
      const parsedLimit = rawThbLimit !== undefined ? Number(rawThbLimit) : Number.NaN;
      const qty =
        Number.isFinite(parsedLimit) && parsedLimit >= 0
          ? String(Math.floor(parsedLimit / thbUnitSum) + 10)
          : '10';
      const totalThb = Number(qty) * thbUnitSum;
      console.log(
        `[f12:policy] cash_transfer_policy THB limit=${rawThbLimit ?? 'null'}, unitSum=${thbUnitSum}, chosen qty=${qty} per THB denomination (total=${totalThb} THB)`,
      );

      // Step 1 (finance): EXTERNAL_IN with those lines; create + submit -> both ok.
      const inId = await createAndSubmitTransfer(
        finance,
        'EXTERNAL_IN',
        qty,
        'e2e F12: above-limit EXTERNAL_IN',
      );
      await shot(finance, 'cash', 'escalation-1-in-submitted');
      const inReqId = await findPendingTransferRequestId(admin, inId);

      // Step 2 (manager): on /#approvals, attempt to approve it -> refused (ABOVE_LIMIT); still PENDING.
      await manager.goto('/#approvals');
      const mgrFilter = manager.locator('[data-type-filter="CASH_TRANSFER"]');
      await expect(mgrFilter).toBeVisible({ timeout: 20_000 });
      await mgrFilter.click();
      await expect(manager.locator('.admin-approvals-screen .approvals-queue, .admin-approvals-screen .empty').first()).toBeVisible({
        timeout: 20_000,
      });
      const mgrCard = manager
        .locator(`[data-request="${inReqId}"]`)
        .filter({ has: manager.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }) })
        .first();
      // `.empty` renders while the queue is still loading (Approvals.tsx:369), so wait for the card itself;
      // the raw fetch below is only for a manager whose queue never lists it.
      if (await mgrCard.waitFor({ state: 'visible', timeout: 20_000 }).then(() => true, () => false)) {
        const refused = await posted(manager, /^\/api\/approvals\/[^/]+\/decide$/, () =>
          mgrCard.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }).click(),
        );
        const errAlert = manager.locator('.admin-approvals-screen [role="alert"].error').first();
        await errAlert.waitFor({ state: 'visible', timeout: 10_000 }).catch(() => {});
        const errText = ((await errAlert.textContent().catch(() => '')) ?? '').trim();
        console.log(`[f12:manager] branch=ui decide ${inReqId} -> ${refused.text} error=${errText}`);
        expect(refused.ok, `branch manager must be refused on above-limit transfer: ${refused.text}`).toBeFalsy();
        expect(refused.text).toMatch(/^403\b/);
        await expect(errAlert).toBeVisible();
        expect(errText).not.toBe('');
        if (refused.body) {
          expect(refused.body).toMatch(/ABOVE_LIMIT|only executive may approve transfers above limit/i);
        } else {
          console.log('[f12:manager] ui branch: body unavailable');
        }
      } else {
        const refused = await manager.evaluate(
          async ({ id, branch }) => {
            const res = await fetch(`/api/approvals/${id}/decide`, {
              method: 'POST',
              credentials: 'same-origin',
              headers: {
                'x-xchange-request': '1',
                'idempotency-key': crypto.randomUUID(),
                'content-type': 'application/json',
              },
              body: JSON.stringify({ id, outcome: 'APPROVED', reason: '', branch }),
            });
            return { status: res.status, body: await res.text() };
          },
          { id: inReqId, branch: users.branch },
        );
        console.log(`[f12:manager] branch=raw decide ${inReqId} -> ${refused.status} ${refused.body}`);
        expect(
          refused.status,
          `branch manager must be refused on above-limit transfer: ${refused.status} ${refused.body}`,
        ).toBe(403);
        expect(refused.body).toMatch(/ABOVE_LIMIT|only executive may approve transfers above limit/i);
      }
      await shot(manager, 'cash', 'escalation-2-manager-refused');

      // Still PENDING in approval_requests and still DRAFT on finance's transfer row.
      expect(await findPendingTransferRequestId(admin, inId)).toBe(inReqId);
      await finance.reload();
      await expect(transferRow(finance, inId), 'EXTERNAL_IN stays DRAFT while approval is still PENDING').toContainText(
        'DRAFT',
        { timeout: 20_000 },
      );

      // Step 3 (admin): approve on /#approvals -> ok; transfer is APPROVED.
      await approveOnAdminScreen(admin, inReqId);
      await shot(admin, 'cash', 'escalation-3-admin-approved-in');

      // Step 4 (finance): dispatch and receive the same quantities -> DISPATCHED, RECEIVED.
      await dispatchAndReceiveTransfer(finance, inId, qty);
      await shot(finance, 'cash', 'escalation-4-in-received');

      // Step 5 (finance): EXTERNAL_OUT with the same lines; create + submit -> ok.
      const outId = await createAndSubmitTransfer(
        finance,
        'EXTERNAL_OUT',
        qty,
        'e2e F12: above-limit EXTERNAL_OUT restore safe',
      );
      const outReqId = await findPendingTransferRequestId(admin, outId);

      // Step 6 (admin): approve EXTERNAL_OUT -> ok.
      await approveOnAdminScreen(admin, outReqId);

      // Step 7 (finance): dispatch and receive -> RECEIVED (safe is back where it started).
      await dispatchAndReceiveTransfer(finance, outId, qty);
      await shot(finance, 'cash', 'escalation-7-out-received');
    } finally {
      await Promise.all([admin.close(), finance.close(), manager.close()]);
    }
  });
});
