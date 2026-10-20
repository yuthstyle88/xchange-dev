// Shift open and close on /staff (staff shift card, web/src/screens/staff/shift/ShiftCard.tsx), against a running v3.
// The teller counts, the branch manager verifies: the verifier must not be the counter
// (api/crates/xc-cash/src/domain/person_rules.rs verifier_is_not_counter). Accounts come from ../users.json.
import { test, expect, type Page, type Browser } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { loadUsers } from './helpers';

// Its ticket moves this spec onto helpers.ts (signIn, shot, the typed Users) and removes this loose type.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const users = loadUsers() as unknown as Record<string, any>;
const OUT = new URL('../results/shift/', import.meta.url).pathname;
mkdirSync(OUT, { recursive: true });
const shot = (page: Page, name: string) => page.screenshot({ path: `${OUT}${name}.png`, fullPage: true });

const OPENING_QTY = '10'; // ten of the first THB note, the same at close, so the shift balances

async function signIn(page: Page, identifier: string, password: string, next: string) {
  await page.goto(`/login?next=${encodeURIComponent(next)}`);
  await page.locator('input[autocomplete="username"]').fill(identifier);
  await page.locator('input[autocomplete="current-password"]').fill(password);
  await page.locator('form button[type="submit"]').click();
  try {
    await page.waitForURL((url) => !url.pathname.startsWith('/login') && url.pathname.startsWith(next), { timeout: 20_000 });
  } catch {
    const changePw = await page.getByRole('heading', { name: /เปลี่ยนรหัสผ่าน|Change password/i }).count();
    const message = (await page.locator('#message, [role="alert"]').allInnerTexts()).join(' ').trim();
    throw new Error(changePw ? `${identifier}: must change its password first` : `${identifier} stayed on /login: ${message || 'no message'}`);
  }
}

async function asRole(browser: Browser, who: { phone: string; password: string }): Promise<Page> {
  const page = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
  await signIn(page, who.phone, who.password, '/staff');
  await expect(page.getByTestId('floor-clock').or(page.locator('[data-count-id]')).first()).toBeVisible({ timeout: 20_000 }).catch(() => {});
  return page;
}

// The shift buttons show only to someone clocked in (ShiftCard.tsx:392, floor.myClockIn): clock in with the floor card
// (FloorCard.tsx floor-clock, POST /api/attendance) when needed. Returns what happened.
async function clockIn(page: Page): Promise<string> {
  const shiftButton = page.getByTestId('shift-open-count').or(page.getByTestId('shift-close-count')).first();
  if (await shiftButton.isVisible().catch(() => false)) return 'already clocked in';
  const clock = page.getByTestId('floor-clock');
  await expect(clock, 'the floor card clock button').toBeVisible({ timeout: 20_000 });
  const posted = page.waitForResponse(
    (r) => r.request().method() === 'POST' && new URL(r.url()).pathname === '/api/attendance',
    { timeout: 15_000 },
  );
  await clock.click();
  const res = await posted;
  const answer = `${res.status()} ${(await res.text().catch(() => '')).slice(0, 200)}`;
  await expect(shiftButton, `shift buttons after clock-in (POST /api/attendance ${answer})`).toBeVisible({ timeout: 20_000 });
  return `clocked in: POST /api/attendance ${answer}`;
}

// Fills the count form (first THB denomination) and saves it; returns the server's answer to POST /api/cash-counts.
async function submitCount(page: Page, kind: 'open' | 'close', step: string): Promise<string> {
  await page.getByTestId(kind === 'open' ? 'shift-open-count' : 'shift-close-count').click();
  const form = page.getByTestId('shift-count-form');
  await expect(form).toBeVisible({ timeout: 15_000 });
  const thb = form.locator('[data-denomination-row]').filter({ hasText: 'THB' }).first();
  await expect(thb, 'a THB denomination row in the count form').toBeVisible({ timeout: 15_000 });
  await thb.locator('input[data-denomination-id]').fill(OPENING_QTY);
  await shot(page, `${step}-1-count-filled`);
  const posted = page.waitForResponse(
    (r) => r.request().method() === 'POST' && new URL(r.url()).pathname === '/api/cash-counts',
    { timeout: 15_000 },
  );
  await page.getByTestId('shift-count-save').click();
  const res = await posted;
  const body = (await res.text().catch(() => '')).slice(0, 300);
  await page.waitForTimeout(800);
  await shot(page, `${step}-2-count-saved`);
  const shown = (await page.getByTestId('shift-error').allInnerTexts().catch(() => [])).join(' ').trim();
  if (shown) console.log(`[${step}] screen error: ${shown}`);
  return `${res.status()} ${body}`;
}

// The manager verifies the newest count of the given kind; returns the server's answer to POST /api/cash-counts/{id}/verify.
async function verifyCount(page: Page, kind: 'OPEN' | 'CLOSE', step: string): Promise<string> {
  await page.reload();
  const item = page.locator('[data-count-id]').filter({ hasText: kind }).first();
  await expect(item, `a ${kind} count waiting for verification`).toBeVisible({ timeout: 20_000 });
  await shot(page, `${step}-1-to-verify`);
  const verified = page.waitForResponse(
    (r) => r.request().method() === 'POST' && /\/api\/cash-counts\/[^/]+\/verify$/.test(new URL(r.url()).pathname),
    { timeout: 15_000 },
  );
  await item.getByTestId('shift-verify').click();
  const res = await verified;
  const body = (await res.text().catch(() => '')).slice(0, 300);
  await page.waitForTimeout(800);
  await shot(page, `${step}-2-verified`);
  const shown = (await page.getByTestId('shift-error').allInnerTexts().catch(() => [])).join(' ').trim();
  if (shown) console.log(`[${step}] screen error: ${shown}`);
  return `${res.status()} ${body}`;
}

const FUND_QTY = '100'; // of every THB note: far more than the opening count takes

// Waits for one POST whose path matches, runs `act`, and returns "<status> <body>".
async function posted(page: Page, path: RegExp, act: () => Promise<void>): Promise<{ ok: boolean; text: string; body: string }> {
  const res = page.waitForResponse((r) => r.request().method() === 'POST' && path.test(new URL(r.url()).pathname), { timeout: 15_000 });
  await act();
  const r = await res;
  const body = await r.text().catch(() => '');
  return { ok: r.ok(), text: `${r.status()} ${body.slice(0, 300)}`, body };
}

// The transfers row for one id: `tr[data-transfer-id]` (web/src/screens/staff/transfers/TransfersCard.tsx:574).
const transferRow = (page: Page, id: string) => page.locator(`tr[data-transfer-id="${id}"]`);

test.describe.serial('shift open and close', () => {
  test.skip(!users.branchManager?.password || String(users.branchManager.password).startsWith('<'),
    'no branchManager in users.json: run scripts/reseed.sh');

  // Since LG-4 (#929, design ledger-inventory.md §4.4 row 1) verifying an OPEN count moves the counted cash out of
  // the branch's ledger SAFE (count_service.rs post_opening_in), so an empty safe refuses it with stockInsufficient.
  // Cash enters a safe only by an EXTERNAL_IN transfer (BANK -> SAFE, kinds/external.rs). A5 (cash-shift-transfer.md:17):
  // within the limit the source branch's BRANCH_MANAGER approves, never the creator (approval_service.rs:260), and an
  // above-limit one goes to EXECUTIVE. So FINANCE (CashManage) records it, the branch manager approves it on the admin
  // approvals screen, and FINANCE dispatches and receives it.
  test('finance funds the safe from the bank (EXTERNAL_IN), the branch manager approves it', async ({ browser }) => {
    test.skip(!users.finance?.password || String(users.finance.password).startsWith('<'), 'no finance in users.json: run scripts/reseed.sh');
    const finance = await asRole(browser, users.finance);
    await finance.getByTestId('transfer-new').click();
    const form = finance.getByTestId('transfer-form');
    await expect(form).toBeVisible({ timeout: 15_000 });
    await form.getByTestId('transfer-kind').selectOption('EXTERNAL_IN');
    await form.getByTestId('transfer-note').fill('e2e: fund the safe for the shift test');
    const thbRows = form.locator('[data-denomination-row]').filter({ hasText: 'THB' });
    const thbCount = await thbRows.count();
    expect(thbCount, 'THB rows in the transfer form').toBeGreaterThan(0);
    for (let i = 0; i < thbCount; i++) await thbRows.nth(i).locator('input[data-denomination-id]').fill(FUND_QTY);
    await shot(finance, 'fund-1-form');
    const submitted = finance.waitForResponse(
      (r) => r.request().method() === 'POST' && /\/api\/cash-transfers\/[^/]+\/submit$/.test(new URL(r.url()).pathname),
      { timeout: 15_000 },
    );
    const created = await posted(finance, /^\/api\/cash-transfers$/, () => form.getByTestId('transfer-save').click());
    console.log(`[fund] POST /api/cash-transfers ${created.text}`);
    expect(created.ok, `transfer created: ${created.text}`).toBeTruthy();
    const id = String(JSON.parse(created.body).id ?? '');
    expect(id, 'the new transfer id').not.toBe('');
    const sub = await submitted;
    const subText = `${sub.status()} ${(await sub.text().catch(() => '')).slice(0, 300)}`;
    console.log(`[fund] submit ${subText}`);
    expect(sub.ok(), `transfer submitted: ${subText}`).toBeTruthy();
    await finance.reload();
    const row = transferRow(finance, id);
    await expect(row).toBeVisible({ timeout: 20_000 });
    await shot(finance, 'fund-2-submitted');

    // The branch manager decides it on the admin approvals screen (admin.html at /, menu `approvals`).
    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    await signIn(manager, users.branchManager.phone, users.branchManager.password, '/');
    await manager.goto('/#approvals');
    const card = manager.locator('[data-request]').filter({ has: manager.locator('[data-request-type="CASH_TRANSFER"]') })
      .filter({ has: manager.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }) }).first();
    await expect(card, 'a pending CASH_TRANSFER approval for the branch manager').toBeVisible({ timeout: 20_000 });
    await shot(manager, 'fund-3-approval');
    const decided = await posted(manager, /^\/api\/approvals\/[^/]+\/decide$/, () =>
      card.getByRole('button', { name: /^(อนุมัติ|Approve)$/ }).click());
    console.log(`[fund-manager] decide ${decided.text}`);
    expect(decided.ok, `approval decided: ${decided.text}`).toBeTruthy();
    await manager.close();

    await finance.reload();
    await expect(row, 'the transfer is approved').toContainText('APPROVED', { timeout: 20_000 });
    const dispatched = await posted(finance, /\/api\/cash-transfers\/[^/]+\/dispatch$/, () => row.getByTestId('transfer-dispatch').click());
    console.log(`[fund] dispatch ${dispatched.text}`);
    expect(dispatched.ok, `transfer dispatched: ${dispatched.text}`).toBeTruthy();
    await finance.reload();
    await expect(row, 'the transfer is dispatched').toContainText('DISPATCHED', { timeout: 20_000 });
    await row.getByTestId('transfer-receive').click();
    const recv = finance.getByTestId('transfer-receive-form');
    await expect(recv).toBeVisible({ timeout: 15_000 });
    const recvRows = recv.locator('[data-denomination-row]').filter({ hasText: 'THB' });
    for (let i = 0; i < (await recvRows.count()); i++) await recvRows.nth(i).locator('input[data-denomination-id]').fill(FUND_QTY);
    const received = await posted(finance, /\/api\/cash-transfers\/[^/]+\/receive$/, () => recv.getByTestId('transfer-receive-save').click());
    console.log(`[fund] receive ${received.text}`);
    expect(received.ok, `transfer received: ${received.text}`).toBeTruthy();
    await finance.reload();
    await expect(row, 'the transfer is received').toContainText('RECEIVED', { timeout: 20_000 });
    await shot(finance, 'fund-4-received');
    await finance.close();
  });

  test('teller counts the opening, the branch manager verifies it', async ({ browser }) => {
    const teller = await asRole(browser, users.teller);
    console.log(`[open] ${await clockIn(teller)}`);
    await shot(teller, 'open-0-staff');
    const open = await submitCount(teller, 'open', 'open-teller');
    console.log(`[open] POST /api/cash-counts ${open}`);
    expect(open.startsWith('2'), `opening count accepted: ${open}`).toBeTruthy();

    const manager = await asRole(browser, users.branchManager);
    const verify = await verifyCount(manager, 'OPEN', 'open-manager');
    console.log(`[open] verify ${verify}`);
    expect(verify.startsWith('2'), `opening count verified: ${verify}`).toBeTruthy();

    await teller.reload();
    await expect(teller.getByTestId('shift-close-count'), 'the teller can now count the close').toBeVisible({ timeout: 20_000 });
    await shot(teller, 'open-3-teller-shift-open');
    await teller.close();
    await manager.close();
  });

  test('teller counts the close, the branch manager verifies it', async ({ browser }) => {
    const teller = await asRole(browser, users.teller);
    console.log(`[close] ${await clockIn(teller)}`);
    const close = await submitCount(teller, 'close', 'close-teller');
    console.log(`[close] POST /api/cash-counts ${close}`);
    expect(close.startsWith('2'), `closing count accepted: ${close}`).toBeTruthy();

    const manager = await asRole(browser, users.branchManager);
    const verify = await verifyCount(manager, 'CLOSE', 'close-manager');
    console.log(`[close] verify ${verify}`);
    expect(verify.startsWith('2'), `closing count verified: ${verify}`).toBeTruthy();

    await teller.reload();
    await expect(teller.getByTestId('shift-open-count'), 'the shift is closed: the teller can open again').toBeVisible({ timeout: 20_000 });
    await shot(teller, 'close-3-teller-shift-closed');
    // Clock out again so the next run starts from the same state.
    await teller.getByTestId('floor-clock').click().catch(() => {});
    await teller.close();
    await manager.close();
  });
});
