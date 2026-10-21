// Flow F12b (docs/design/qa-e2e.md §4 F12b, ticket #4072):
//   Branch-to-branch cash transfer (MAIN -> branch2 -> MAIN) within the THB policy limit:
//   Leg 1 (MAIN -> branch2): finance creates & submits -> branchManager approves -> finance dispatches -> branchManager2 receives at branch2.
//   Leg 2 (branch2 -> MAIN): finance2 creates & submits -> branchManager2 approves -> finance2 dispatches -> branchManager receives at MAIN.
//   Both safes end where they started. Never saves a transfer policy or creates a branch (E-4). Never run against a host from a worker's machine (E-3).
import { expect, test, type Browser, type Page } from '@playwright/test';
import { hasAccount, posted, shot, signIn, users, type Role } from './helpers';

const REQUIRED_ROLES: readonly Role[] = ['admin', 'finance', 'branchManager', 'finance2', 'branchManager2'];
const TRANSFER_QTY = '1';

const transferRow = (page: Page, id: string) => page.locator(`tr[data-transfer-id="${id}"]`);

async function asRole(browser: Browser, role: Role, next: string): Promise<Page> {
  const page = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
  await signIn(page, role, next);
  return page;
}

async function createAndSubmitBranchTransfer(
  creator: Page,
  toBranch: string,
  denomId: string,
  qty: string,
  note: string,
  label: string,
): Promise<string> {
  await creator.goto('/staff');
  await creator.getByTestId('transfer-new').click();
  const form = creator.getByTestId('transfer-form');
  await expect(form).toBeVisible({ timeout: 15_000 });
  await form.getByTestId('transfer-kind').selectOption('BRANCH');

  const toSelect = form.getByTestId('transfer-to-branch');
  await expect(toSelect.locator(`option[value="${toBranch}"]`), `destination option ${toBranch}`).toHaveCount(1, {
    timeout: 20_000,
  });
  await toSelect.selectOption(toBranch);
  await form.getByTestId('transfer-note').fill(note);

  const denomInput = form.locator(`input[data-denomination-id="${denomId}"]`);
  await expect(denomInput, `THB denomination input ${denomId}`).toBeVisible({ timeout: 20_000 });
  await denomInput.fill(qty);

  const submitted = creator.waitForResponse(
    (r) => r.request().method() === 'POST' && /\/api\/cash-transfers\/[^/]+\/submit$/.test(new URL(r.url()).pathname),
    { timeout: 15_000 },
  );
  const created = await posted(creator, /^\/api\/cash-transfers$/, () => form.getByTestId('transfer-save').click());
  console.log(`[f12b:${label}] POST /api/cash-transfers ${created.text}`);
  expect(created.ok, `${label} transfer created: ${created.text}`).toBeTruthy();
  const id = String(JSON.parse(created.body).id ?? '');
  expect(id, `${label} new transfer id`).not.toBe('');

  const sub = await submitted;
  const subText = `${sub.status()} ${(await sub.text().catch(() => '')).slice(0, 300)}`;
  console.log(`[f12b:${label}] submit ${subText}`);
  expect(sub.ok(), `${label} transfer submitted: ${subText}`).toBeTruthy();

  await creator.reload();
  await expect(transferRow(creator, id)).toBeVisible({ timeout: 20_000 });
  return id;
}

async function findPendingTransferRequestId(
  viewer: Page,
  branch: string,
  transferId: string,
): Promise<string> {
  const res = await viewer.request.get(
    `/api/v3/views/approval_requests?branch=${encodeURIComponent(branch)}&status=PENDING&type=CASH_TRANSFER`,
  );
  expect(res.ok(), `read approval_requests on ${branch}: ${res.status()}`).toBeTruthy();
  const body = (await res.json()) as {
    data?: { requests?: Array<{ id: string; objectId: string; status: string }> };
  };
  const match = body.data?.requests?.find((r) => r.objectId === transferId && r.status === 'PENDING');
  expect(match?.id, `pending CASH_TRANSFER approval request for transfer ${transferId} on ${branch}`).toBeTruthy();
  return match!.id;
}

async function approveTransferOnAdminScreen(
  approver: Page,
  reqId: string,
  label: string,
): Promise<void> {
  await approver.goto('/#approvals');
  const filterBtn = approver.locator('[data-type-filter="CASH_TRANSFER"]');
  await expect(filterBtn).toBeVisible({ timeout: 20_000 });
  await filterBtn.click();
  const card = approver
    .locator(`[data-request="${reqId}"]`)
    .filter({ has: approver.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }) })
    .first();
  await expect(card, `pending CASH_TRANSFER card ${reqId} (${label})`).toBeVisible({ timeout: 20_000 });
  const decided = await posted(approver, /^\/api\/approvals\/[^/]+\/decide$/, () =>
    card.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }).click(),
  );
  console.log(`[f12b:${label}] decide ${reqId} -> ${decided.text}`);
  expect(decided.ok, `${label} approved ${reqId}: ${decided.text}`).toBeTruthy();
}

async function dispatchTransfer(
  dispatcher: Page,
  id: string,
  label: string,
): Promise<void> {
  await dispatcher.goto('/staff');
  const row = transferRow(dispatcher, id);
  await expect(row, `${label} transfer ${id} is APPROVED`).toContainText('APPROVED', { timeout: 20_000 });
  const dispatched = await posted(dispatcher, /\/api\/cash-transfers\/[^/]+\/dispatch$/, () =>
    row.getByTestId('transfer-dispatch').click(),
  );
  console.log(`[f12b:${label}] dispatch ${id} -> ${dispatched.text}`);
  expect(dispatched.ok, `${label} transfer ${id} dispatched: ${dispatched.text}`).toBeTruthy();
  await dispatcher.reload();
  await expect(row, `${label} transfer ${id} is DISPATCHED`).toContainText('DISPATCHED', { timeout: 20_000 });
}

async function receiveTransfer(
  receiver: Page,
  id: string,
  denomId: string,
  qty: string,
  label: string,
): Promise<void> {
  await receiver.goto('/staff');
  const row = transferRow(receiver, id);
  await expect(row, `${label} transfer ${id} is DISPATCHED at destination`).toContainText('DISPATCHED', {
    timeout: 20_000,
  });
  await row.getByTestId('transfer-receive').click();
  const recv = receiver.getByTestId('transfer-receive-form');
  await expect(recv).toBeVisible({ timeout: 15_000 });
  const denomInput = recv.locator(`input[data-denomination-id="${denomId}"]`);
  await expect(denomInput, `THB denomination input ${denomId} in receive form`).toBeVisible({ timeout: 20_000 });
  await denomInput.fill(qty);

  const received = await posted(receiver, /\/api\/cash-transfers\/[^/]+\/receive$/, () =>
    recv.getByTestId('transfer-receive-save').click(),
  );
  console.log(`[f12b:${label}] receive ${id} -> ${received.text}`);
  expect(received.ok, `${label} transfer ${id} received: ${received.text}`).toBeTruthy();
  await receiver.reload();
  await expect(row, `${label} transfer ${id} is RECEIVED`).toContainText('RECEIVED', { timeout: 20_000 });
}

test.describe.serial('F12b branch-to-branch cash transfer', () => {
  test('MAIN -> branch2 and branch2 -> MAIN within the limit, approved by each source branch manager and received at the destination', async ({
    browser,
  }) => {
    if (!users.branch2) {
      const reason = 'no branch2 in users.json: run scripts/reseed.sh';
      console.log(`[f12b] skip: ${reason}`);
      test.skip(true, reason);
    }
    const missing = REQUIRED_ROLES.filter((r) => !hasAccount(r));
    if (missing.length > 0) {
      const reason = `missing account in users.json for: ${missing.join(', ')} (run scripts/reseed.sh)`;
      console.log(`[f12b] skip: ${reason}`);
      test.skip(true, reason);
    }
    const branch1 = users.branch;
    const branch2 = users.branch2!;

    const admin = await asRole(browser, 'admin', '/');
    const finance1 = await asRole(browser, 'finance', '/staff');
    const manager1 = await asRole(browser, 'branchManager', '/');
    const finance2 = await asRole(browser, 'finance2', '/staff');
    const manager2 = await asRole(browser, 'branchManager2', '/staff');

    try {
      // Step 0 (admin): read cash_transfer_policy and pick a THB line within the limit (e.g. 1 note of 100 THB).
      const policyRes = await admin.request.get('/api/v3/views/cash_transfer_policy');
      expect(policyRes.ok(), `GET /api/v3/views/cash_transfer_policy (${policyRes.status()})`).toBeTruthy();
      const policyBody = (await policyRes.json()) as {
        data?: { limits?: Record<string, string> } | null;
      };
      const rawThbLimit = policyBody.data?.limits?.THB;
      const parsedLimit = rawThbLimit !== undefined ? Number(rawThbLimit) : Number.NaN;
      expect(
        Number.isFinite(parsedLimit) && parsedLimit > 0,
        `cash_transfer_policy THB limit must be positive so a branch transfer is within the limit (got ${rawThbLimit ?? 'null'})`,
      ).toBeTruthy();

      const denomRes = await admin.request.get('/api/v3/views/denominations');
      expect(denomRes.ok(), `GET /api/v3/views/denominations (${denomRes.status()})`).toBeTruthy();
      const denomBody = (await denomRes.json()) as {
        data?: { rows?: Array<{ id: number; currency: string; faceValue: string }> };
      };
      const thbDenoms = (denomBody.data?.rows ?? []).filter((d) => d.currency === 'THB');
      const picked =
        thbDenoms.find((d) => Number(d.faceValue) === 100 && Number(d.faceValue) <= parsedLimit) ??
        thbDenoms.find((d) => Number(d.faceValue) <= parsedLimit);
      expect(picked, `a THB denomination within the limit ${parsedLimit}`).toBeTruthy();
      const denomId = String(picked!.id);
      const totalThb = Number(picked!.faceValue) * Number(TRANSFER_QTY);
      console.log(
        `[f12b:policy] cash_transfer_policy THB limit=${rawThbLimit}, chosen denomination id=${denomId} (THB ${picked!.faceValue} x ${TRANSFER_QTY} = ${totalThb} THB <= ${parsedLimit})`,
      );

      // Step 1 (finance on MAIN): BRANCH transfer MAIN -> branch2; create + submit -> both ok.
      const leg1Id = await createAndSubmitBranchTransfer(
        finance1,
        branch2,
        denomId,
        TRANSFER_QTY,
        `e2e F12b leg 1: ${branch1} -> ${branch2}`,
        'leg1-create',
      );
      await shot(finance1, 'cash', 'branch-transfer-1-leg1-submitted');
      const leg1ReqId = await findPendingTransferRequestId(manager1, branch1, leg1Id);

      // Step 2 (branchManager on MAIN): /#approvals, approve -> ok.
      await approveTransferOnAdminScreen(manager1, leg1ReqId, 'leg1-manager1-approve');
      await shot(manager1, 'cash', 'branch-transfer-2-leg1-approved');

      // Step 3 (finance on MAIN): dispatch -> DISPATCHED.
      await dispatchTransfer(finance1, leg1Id, 'leg1-finance1-dispatch');
      await shot(finance1, 'cash', 'branch-transfer-3-leg1-dispatched');

      // Step 4 (branchManager2 on branch2 session): receive the same quantities -> RECEIVED.
      await receiveTransfer(manager2, leg1Id, denomId, TRANSFER_QTY, 'leg1-manager2-receive');
      await shot(manager2, 'cash', 'branch-transfer-4-leg1-received');

      // Step 5 (finance2 on branch2): BRANCH transfer branch2 -> MAIN, the same lines; create + submit -> ok.
      const leg2Id = await createAndSubmitBranchTransfer(
        finance2,
        branch1,
        denomId,
        TRANSFER_QTY,
        `e2e F12b leg 2: ${branch2} -> ${branch1}`,
        'leg2-create',
      );
      await shot(finance2, 'cash', 'branch-transfer-5-leg2-submitted');
      const leg2ReqId = await findPendingTransferRequestId(manager2, branch2, leg2Id);

      // Step 6 (branchManager2 on branch2): /#approvals, approve -> ok.
      await approveTransferOnAdminScreen(manager2, leg2ReqId, 'leg2-manager2-approve');
      await shot(manager2, 'cash', 'branch-transfer-6-leg2-approved');

      // Step 7 (finance2 on branch2): dispatch -> DISPATCHED.
      await dispatchTransfer(finance2, leg2Id, 'leg2-finance2-dispatch');
      await shot(finance2, 'cash', 'branch-transfer-7-leg2-dispatched');

      // Step 8 (branchManager on MAIN): receive -> RECEIVED; both safes are back where they started.
      await receiveTransfer(manager1, leg2Id, denomId, TRANSFER_QTY, 'leg2-manager1-receive');
      await shot(manager1, 'cash', 'branch-transfer-8-leg2-received');
    } finally {
      await Promise.all([admin.close(), finance1.close(), manager1.close(), finance2.close(), manager2.close()]);
    }
  });
});
