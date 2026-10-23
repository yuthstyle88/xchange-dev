// Ticket #4881 — Sick or leaving teller: hand the shift over (owner's decision): prove it end to end on xc-v3.
// Import only from './helpers'. Never run against a host from a worker's machine (E-3): check with --list and tsc.
import { expect, test, type Page } from '@playwright/test';
import {
  clockIn,
  cutOff,
  ensureShiftClosed,
  hasAccount,
  posted,
  shot,
  signIn,
  startService,
  users,
  verifyCount,
  type Account,
  type Role,
} from './helpers';

// 5 USD at the seeded 32.10 THB/USD rate pays 160.50 THB, an exact multiple of the smallest THB coin (0.25 THB),
// so both the handover count and B's closing count break down into THB denominations with zero remainder.
const EXCHANGE_AMOUNT = '5';
// One 1,000 THB note from the branch SAFE covers both 5 USD BUY exchanges (2 * 160.50 = 321.00 THB).
const OPENING_FLOAT_THB = 1000;
const PHASE_CLOSED = /(ปิดกะ|^Closed$|^CLOSED$)/i;

interface UsersWithTeller2 {
  teller2?: Account;
}

interface MeViewAnswer {
  data?: {
    id?: string;
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

interface AdminShiftBalanceRow {
  code: string;
  expected: string;
  counted: string | null;
  difference: string | null;
}

interface AdminShiftRow {
  id: string;
  status: string;
  openedAt: string;
  staff: string;
  note: string;
  balances: AdminShiftBalanceRow[];
}

interface AdminShiftsAnswer {
  rows?: AdminShiftRow[];
  data?: AdminShiftRow[] | { rows?: AdminShiftRow[] };
}

interface HandoverStartAnswer {
  id?: string;
  handoverId?: string;
  data?: {
    id?: string;
    handoverId?: string;
  };
}

interface TransactionAnswer {
  id?: string;
  status?: string;
}

function getTeller2Account(): Account | undefined {
  return (users as unknown as UsersWithTeller2).teller2;
}

function hasTeller2Account(): boolean {
  const acc = getTeller2Account();
  return !!acc?.password && !acc.password.startsWith('<');
}

/** Signs in `teller2` via /login?next=... with the same form contract as `loginWithForm` in helpers.ts. */
async function signInTeller2(page: Page, next: string): Promise<void> {
  const account = getTeller2Account()!;
  const identifier = account.identifier ?? account.phone ?? '';
  await page.goto(`/login?next=${encodeURIComponent(next)}`);
  await page.locator('input[autocomplete="username"]').fill(identifier);
  await page.locator('input[autocomplete="current-password"]').fill(account.password);
  await page.locator('form button[type="submit"]').click();
  const path = next.split(/[?#]/)[0];
  await page.waitForURL((url) => !url.pathname.startsWith('/login') && url.pathname.startsWith(path), {
    timeout: 20_000,
  });
}

async function meUserId(page: Page): Promise<string> {
  const res = await page.request.get('/api/v3/views/me');
  expect(res.ok(), `GET /api/v3/views/me status ${res.status()}`).toBeTruthy();
  const body = (await res.json().catch(() => ({}))) as MeViewAnswer;
  const uid = String(body.data?.id ?? '');
  expect(uid, 'me.data.id from /api/v3/views/me').not.toBe('');
  return uid;
}

async function readMyShift(page: Page): Promise<NonNullable<MyShiftAnswer['data']> | null> {
  const res = await page.request.get(`/api/v3/views/my_shift?branch=${encodeURIComponent(users.branch)}`);
  expect(res.ok(), `GET /api/v3/views/my_shift status ${res.status()}`).toBeTruthy();
  const body = (await res.json().catch(() => ({}))) as MyShiftAnswer;
  return body.data ?? null;
}

async function readAdminShifts(page: Page): Promise<AdminShiftRow[]> {
  const res = await page.request.get(`/api/v3/views/admin_shifts?branch=${encodeURIComponent(users.branch)}`);
  expect(res.ok(), `GET /api/v3/views/admin_shifts status ${res.status()}`).toBeTruthy();
  const body = (await res.json().catch(() => ({}))) as AdminShiftsAnswer;
  if (Array.isArray(body.rows)) return body.rows;
  if (Array.isArray(body.data)) return body.data;
  if (body.data && Array.isArray(body.data.rows)) return body.data.rows;
  return [];
}

function balanceMap(balances: Array<{ code: string; expected: string }>): Record<string, number> {
  const out: Record<string, number> = {};
  for (const b of balances) {
    out[b.code] = Number(b.expected);
  }
  return out;
}

async function clockOutIfClockedIn(page: Page): Promise<void> {
  if (!new URL(page.url(), users.baseUrl).pathname.startsWith('/staff')) {
    await page.goto('/staff');
  } else {
    await page.reload();
  }
  const shiftButton = page.getByTestId('shift-open-count').or(page.getByTestId('shift-close-count')).first();
  if (!(await shiftButton.isVisible().catch(() => false))) return;
  const clock = page.getByTestId('floor-clock');
  await expect(clock, 'floor-clock visible before clock-out').toBeVisible({ timeout: 20_000 });
  const res = await posted(page, /^\/api\/attendance$/, () => clock.click());
  expect(res.ok, `clock-out POST /api/attendance: ${res.text}`).toBeTruthy();
}

/** Saves one cash BUY exchange of `EXCHANGE_AMOUNT` on `/desk?branch=...` and returns the completed transaction id. */
async function saveOneCashBuyExchange(tellerPage: Page, label: string): Promise<string> {
  const currency = users.currency ?? 'USD';
  await tellerPage.goto('/desk?branch=' + users.branch);
  await expect(
    tellerPage.locator(`select[name="currency"] option[value="${currency}"]`),
    `[${label}] rate for ${currency} loaded in desk_config`,
  ).toHaveCount(1, { timeout: 20_000 });

  const quoteRes = await posted(tellerPage, /^\/api\/quotes$/, async () => {
    await tellerPage.locator('select[name="type"]').selectOption('BUY');
    await tellerPage.locator('select[name="currency"]').selectOption(currency);
    await tellerPage.locator('input[name="amount"]').fill(EXCHANGE_AMOUNT);
  });
  expect(quoteRes.ok, `[${label}] POST /api/quotes: ${quoteRes.text}`).toBeTruthy();
  await expect(tellerPage.locator('.desk-rate-line strong')).toBeVisible({ timeout: 15_000 });

  await tellerPage.getByRole('button', { name: /ถัดไป · พาสปอร์ต \/ ลูกค้า|Next · Passport \/ customer/ }).click();

  const skipBtn = tellerPage.getByRole('button', { name: /ข้ามข้อมูลลูกค้า|Skip customer details/ });
  await expect(skipBtn, `[${label}] customer step skip button`).toBeVisible({ timeout: 15_000 });
  const reserveRes = await posted(tellerPage, /^\/api\/rpc$/, () => skipBtn.click());
  expect(reserveRes.ok, `[${label}] reserveCustomer on skip: ${reserveRes.text}`).toBeTruthy();

  const confirmBookingBtn = tellerPage.getByRole('button', { name: /ยืนยันการจอง|Confirm booking/ });
  await expect(confirmBookingBtn, `[${label}] booking step confirm button`).toBeVisible({ timeout: 15_000 });
  await confirmBookingBtn.click();

  const methodSelect = tellerPage.locator('select[name="method"]');
  await expect(methodSelect, `[${label}] payment step method select`).toBeVisible({ timeout: 15_000 });
  await methodSelect.selectOption('Cash');

  const accountField = tellerPage.locator('[name="account"]');
  await expect(accountField, `[${label}] payment account control`).toBeVisible({ timeout: 15_000 });
  const accountOptions = accountField.locator('option');
  if ((await accountOptions.count()) > 0) {
    const firstOption = (await accountOptions.first().getAttribute('value')) ?? '';
    await accountField.selectOption(firstOption);
  } else if ((await accountField.inputValue()).trim() === '') {
    await accountField.fill('Cash');
  }

  await tellerPage.locator('input[name="paymentChecked"]').check();
  tellerPage.once('dialog', (d) => d.accept());

  const confirmPromise = tellerPage.waitForResponse(
    (r) => r.request().method() === 'POST' && /^\/api\/transactions\/[^/]+\/confirm$/.test(new URL(r.url()).pathname),
    { timeout: 15_000 },
  );
  const completePromise = tellerPage.waitForResponse(
    (r) => r.request().method() === 'POST' && /^\/api\/transactions\/[^/]+\/complete$/.test(new URL(r.url()).pathname),
    { timeout: 15_000 },
  );

  const createRes = await posted(tellerPage, /^\/api\/transactions$/, () =>
    tellerPage.getByRole('button', { name: /ยืนยันและบันทึกบัญชี|Confirm and save/ }).click(),
  );
  expect(createRes.ok, `[${label}] POST /api/transactions: ${createRes.text}`).toBeTruthy();
  const createdTx = JSON.parse(createRes.body) as TransactionAnswer;
  const txId = String(createdTx.id ?? '');
  expect(txId, `[${label}] transaction id`).not.toBe('');

  const confirmRes = await confirmPromise;
  expect(confirmRes.ok(), `[${label}] POST /api/transactions/${txId}/confirm`).toBeTruthy();
  const completeRes = await completePromise;
  expect(completeRes.ok(), `[${label}] POST /api/transactions/${txId}/complete`).toBeTruthy();

  await expect(tellerPage.locator('section.desk-form strong').first()).toHaveText(/^XC-\d{8}-\d{6}$/, {
    timeout: 20_000,
  });
  return txId;
}

/** Fills the count or handover denomination inputs so they match `expectedByCurrency` from `my_shift`. */
async function fillDenominationsForBalances(
  form: ReturnType<Page['getByTestId']>,
  expectedByCurrency: Record<string, number>,
): Promise<void> {
  const rows = form.locator('[data-denomination-row]');
  await expect(rows.first(), 'denomination rows loaded').toBeVisible({ timeout: 15_000 });
  const count = await rows.count();

  const remaining: Record<string, number> = {};
  for (const [code, amt] of Object.entries(expectedByCurrency)) {
    if (amt > 0) remaining[code] = Math.round(amt * 100);
  }

  const denomsByCurrency: Record<
    string,
    Array<{ index: number; denomMinor: number; input: ReturnType<typeof rows.nth> }>
  > = {};

  for (let i = 0; i < count; i++) {
    const row = rows.nth(i);
    const cur = ((await row.locator('.xc-shift-denom-currency').innerText().catch(() => '')) || '').trim();
    const valStr = ((await row.locator('.xc-shift-denom-val').innerText().catch(() => '')) || '').trim();
    const val = Number(valStr);
    if (!cur || !Number.isFinite(val) || val <= 0) continue;
    if (!denomsByCurrency[cur]) denomsByCurrency[cur] = [];
    denomsByCurrency[cur].push({ index: i, denomMinor: Math.round(val * 100), input: row });
  }

  for (const [cur, list] of Object.entries(denomsByCurrency)) {
    list.sort((a, b) => b.denomMinor - a.denomMinor);
    let rem = remaining[cur] ?? 0;
    for (const item of list) {
      const inputEl = item.input.locator('input[data-denomination-id]');
      if (rem >= item.denomMinor && item.denomMinor > 0) {
        const qty = Math.floor(rem / item.denomMinor);
        rem -= qty * item.denomMinor;
        await inputEl.fill(String(qty));
      } else {
        await inputEl.fill('');
      }
    }
    expect(rem, `exact denomination breakdown for ${cur} (${expectedByCurrency[cur]})`).toBe(0);
  }
}

test.describe.serial('sick or leaving teller: hand the shift over end to end (#4881)', () => {
  test('teller A opens shift and saves an exchange, hands over to teller B, B accepts and saves an exchange on B shift, B closes shift and branch day closes', async ({
    browser,
  }) => {
    const required: readonly Role[] = ['teller', 'branchManager'];
    for (const role of required) {
      if (!hasAccount(role)) {
        const reason = `no ${role} account in users.json: run scripts/reseed.sh`;
        console.log(`[#4881] skip: ${reason}`);
        test.skip(true, reason);
      }
    }
    if (!hasTeller2Account()) {
      const reason = 'no teller2 account in users.json: run scripts/reseed.sh';
      console.log(`[#4881] skip: ${reason}`);
      test.skip(true, reason);
    }

    const tellerA = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const tellerB = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });
    const manager = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 1280, height: 900 } });

    let started: 'started' | 'alreadyOpen' | undefined;

    try {
      await signIn(tellerA, 'teller', '/staff');
      await signInTeller2(tellerB, '/staff');
      await signIn(manager, 'branchManager', '/staff');

      // Clean up any leftover open shifts on A or B before starting.
      await ensureShiftClosed(tellerA, manager);
      await ensureShiftClosed(tellerB, manager);

      const tellerAUserId = await meUserId(tellerA);
      const tellerBUserId = await meUserId(tellerB);
      expect(tellerBUserId, 'tellerB is a distinct user from tellerA').not.toBe(tellerAUserId);

      // Step 1: Teller A opens a shift with a 1,000 THB float (OPEN count, verified by branchManager) and saves one exchange.
      await tellerA.getByTestId('shift-open-count').click();
      const openForm = tellerA.getByTestId('shift-count-form');
      await expect(openForm, 'teller A shift-count-form visible for OPEN').toBeVisible({ timeout: 15_000 });
      await fillDenominationsForBalances(openForm, { THB: OPENING_FLOAT_THB });
      const openCountRes = await posted(tellerA, /^\/api\/cash-counts$/, () =>
        tellerA.getByTestId('shift-count-save').click(),
      );
      expect(openCountRes.ok, `teller A opening count accepted: ${openCountRes.text}`).toBeTruthy();

      await manager.goto('/staff');
      const openVerifyRes = await verifyCount(manager, 'OPEN');
      if (!openVerifyRes.ok) {
        const errText =
          (await manager.getByTestId('shift-error').innerText({ timeout: 5_000 }).catch(() => '')).trim() ||
          openVerifyRes.text;
        if (
          /stockInsufficient|ตู้เซฟ|ลิ้นชักไม่พอ|Not enough cash in the safe/i.test(
            `${errText} ${openVerifyRes.text}`,
          )
        ) {
          test.skip(true, errText);
        }
        throw new Error(errText);
      }
      await tellerA.reload();
      await expect(tellerA.getByTestId('shift-close-count'), 'teller A shift is OPEN').toBeVisible({ timeout: 20_000 });

      const shiftABefore = await readMyShift(tellerA);
      expect(shiftABefore?.id, 'teller A has an OPEN shift after OPEN count verification').toBeTruthy();
      expect(shiftABefore?.status, 'teller A shift status is OPEN').toBe('OPEN');
      const shiftAId = shiftABefore!.id;

      started = await startService(manager);
      const startedByThisSpec = started === 'started';

      const currency = users.currency ?? 'USD';
      const txAId = await saveOneCashBuyExchange(tellerA, 'tellerA');
      console.log(`[#4881] step 1: teller A (${tellerAUserId}) saved exchange ${txAId} on shift ${shiftAId}`);

      const shiftAAfterTx = await readMyShift(tellerA);
      expect(shiftAAfterTx?.id, 'teller A still holds shift A after saving exchange').toBe(shiftAId);
      const balancesAAfterTx = balanceMap(shiftAAfterTx!.balances);
      expect(
        balancesAAfterTx[currency] ?? 0,
        `teller A shift ${currency} expected balance increased after BUY exchange`,
      ).toBeGreaterThan(0);

      // Step 2: Teller A starts a hand-over to teller B, as if leaving sick.
      await tellerA.goto('/staff');
      await expect(tellerA.getByTestId('shift-handover'), 'teller A shift-handover button visible').toBeVisible({
        timeout: 20_000,
      });
      await tellerA.getByTestId('shift-handover').click();
      const handoverForm = tellerA.getByTestId('shift-handover-form');
      await expect(handoverForm, 'shift-handover-form visible').toBeVisible({ timeout: 15_000 });

      const receiverSelect = handoverForm.getByTestId('shift-receiver');
      await expect(
        receiverSelect.locator(`option[value="${tellerBUserId}"]`),
        'teller B is offered in handover_receivers',
      ).toHaveCount(1, { timeout: 15_000 });
      await receiverSelect.selectOption(tellerBUserId);

      await fillDenominationsForBalances(handoverForm, balancesAAfterTx);

      const startHandoverRes = await posted(tellerA, /^\/api\/handovers$/, () =>
        handoverForm.getByTestId('shift-handover-save').click(),
      );
      console.log(`[#4881] step 2: POST /api/handovers ${startHandoverRes.text}`);
      expect(startHandoverRes.ok, `handoverStart A -> B: ${startHandoverRes.text}`).toBeTruthy();
      const parsedStart = JSON.parse(startHandoverRes.body || '{}') as HandoverStartAnswer;
      const handoverId = String(
        parsedStart.id ?? parsedStart.handoverId ?? parsedStart.data?.id ?? parsedStart.data?.handoverId ?? '',
      );
      await shot(tellerA, 'staff', 'sick-teller-1-handover-started');

      // Step 3: B sees it in the pending list, counts and accepts. A's shift is CLOSED and B's is OPEN, with A's cash carried over.
      await tellerB.goto('/staff');
      await clockIn(tellerB);
      await tellerB.reload();
      const pendingItem = handoverId
        ? tellerB.locator(`[data-handover-id="${handoverId}"]`)
        : tellerB.locator('[data-handover-id]').first();
      await expect(pendingItem, 'pending handover visible to teller B on /staff').toBeVisible({ timeout: 20_000 });
      const resolvedHandoverId = handoverId || ((await pendingItem.getAttribute('data-handover-id')) ?? '');
      expect(resolvedHandoverId, 'resolved handoverId for B').not.toBe('');

      const acceptRes = await posted(
        tellerB,
        new RegExp(`^/api/handovers/${encodeURIComponent(resolvedHandoverId)}/accept$`),
        () => pendingItem.getByTestId('shift-accept').click(),
      );
      console.log(`[#4881] step 3: POST /api/handovers/${resolvedHandoverId}/accept ${acceptRes.text}`);
      expect(acceptRes.ok, `handoverAccept by B: ${acceptRes.text}`).toBeTruthy();

      await tellerA.reload();
      await tellerB.reload();
      await expect(
        tellerA.getByTestId('shift-open-count'),
        'teller A sees shift-open-count on /staff (shift A is CLOSED)',
      ).toBeVisible({ timeout: 20_000 });
      await expect(
        tellerB.getByTestId('shift-close-count'),
        'teller B sees shift-close-count on /staff (shift B is OPEN)',
      ).toBeVisible({ timeout: 20_000 });

      const shiftAAfterAccept = await readMyShift(tellerA);
      expect(shiftAAfterAccept, 'teller A my_shift is null after handover accept').toBeNull();

      const shiftBAfterAccept = await readMyShift(tellerB);
      expect(shiftBAfterAccept?.id, 'teller B has an OPEN shift after handover accept').toBeTruthy();
      expect(shiftBAfterAccept?.status, 'teller B shift status is OPEN').toBe('OPEN');
      const shiftBId = shiftBAfterAccept!.id;
      expect(shiftBId, 'shift B is a new shift id distinct from shift A').not.toBe(shiftAId);

      const balancesBAfterAccept = balanceMap(shiftBAfterAccept!.balances);
      expect(
        balancesBAfterAccept,
        "teller B's shift carries over teller A's cash balances",
      ).toEqual(balancesAAfterTx);

      // Also verify through admin_shifts (read by branchManager).
      const adminShiftsAfterAccept = await readAdminShifts(manager);
      const rowA = adminShiftsAfterAccept.find((r) => r.id === shiftAId);
      const rowB = adminShiftsAfterAccept.find((r) => r.id === shiftBId);
      expect(rowA?.status, `admin_shifts shows shift A (${shiftAId}) CLOSED`).toBe('CLOSED');
      expect(rowB?.status, `admin_shifts shows shift B (${shiftBId}) OPEN`).toBe('OPEN');
      expect(
        balanceMap(rowB!.balances),
        `admin_shifts shows shift B carries over shift A's cash`,
      ).toEqual(balancesAAfterTx);
      await shot(tellerB, 'staff', 'sick-teller-2-handover-accepted');

      // Sick teller A clocks out and leaves.
      await clockOutIfClockedIn(tellerA);

      // Step 4: B saves an exchange on that shift. It posts to B's shift.
      const txBId = await saveOneCashBuyExchange(tellerB, 'tellerB');
      console.log(`[#4881] step 4: teller B (${tellerBUserId}) saved exchange ${txBId} on shift ${shiftBId}`);

      const shiftBAfterTx = await readMyShift(tellerB);
      expect(shiftBAfterTx?.id, 'teller B shift id unchanged after B exchange').toBe(shiftBId);
      const balancesBAfterTx = balanceMap(shiftBAfterTx!.balances);
      expect(
        balancesBAfterTx[currency] ?? 0,
        `B's exchange posted to B's shift (${currency} increased from ${balancesBAfterAccept[currency]} to ${balancesBAfterTx[currency]})`,
      ).toBeGreaterThan(balancesBAfterAccept[currency] ?? 0);
      expect(
        balancesBAfterTx.THB ?? 0,
        `B's exchange paid THB from B's shift (${balancesBAfterTx.THB} < ${balancesBAfterAccept.THB})`,
      ).toBeLessThan(balancesBAfterAccept.THB ?? 0);

      const adminShiftsAfterBTx = await readAdminShifts(manager);
      const rowAAfterBTx = adminShiftsAfterBTx.find((r) => r.id === shiftAId);
      const rowBAfterBTx = adminShiftsAfterBTx.find((r) => r.id === shiftBId);
      expect(
        balanceMap(rowAAfterBTx!.balances),
        "A's closed shift balances in admin_shifts remain untouched by B's exchange",
      ).toEqual(balancesAAfterTx);
      expect(
        balanceMap(rowBAfterBTx!.balances),
        "admin_shifts reflects B's exchange posted to B's shift",
      ).toEqual(balancesBAfterTx);
      await shot(tellerB, 'desk', 'sick-teller-3-b-exchange-saved');

      // Step 5: B closes the shift normally (B's CLOSE count, verified by branchManager). The branch day can then close.
      if (started === 'started') {
        await cutOff(manager);
        started = undefined;
      }

      await tellerB.goto('/staff');
      await clockIn(tellerB);
      await tellerB.getByTestId('shift-close-count').click();
      const closeForm = tellerB.getByTestId('shift-count-form');
      await expect(closeForm, 'teller B shift-count-form visible for CLOSE').toBeVisible({ timeout: 15_000 });
      await fillDenominationsForBalances(closeForm, balancesBAfterTx);

      const closeCountRes = await posted(tellerB, /^\/api\/cash-counts$/, () =>
        tellerB.getByTestId('shift-count-save').click(),
      );
      expect(closeCountRes.ok, `teller B closing count accepted: ${closeCountRes.text}`).toBeTruthy();

      await manager.goto('/staff');
      const verifyRes = await verifyCount(manager, 'CLOSE');
      expect(verifyRes.ok, `manager verifies B closing count: ${verifyRes.text}`).toBeTruthy();

      await tellerB.reload();
      await expect(tellerB.getByTestId('shift-open-count'), 'teller B shift is CLOSED').toBeVisible({
        timeout: 20_000,
      });
      await clockOutIfClockedIn(tellerB);

      const adminShiftsFinal = await readAdminShifts(manager);
      const rowBFinal = adminShiftsFinal.find((r) => r.id === shiftBId);
      expect(rowBFinal?.status, `admin_shifts shows shift B (${shiftBId}) CLOSED`).toBe('CLOSED');

      if (startedByThisSpec) {
        await manager.reload();
        await expect(
          manager.getByTestId('floor-phase'),
          'branch day reaches CLOSED after B closes the shift',
        ).toHaveText(PHASE_CLOSED, { timeout: 20_000 });
      }
      await shot(manager, 'staff', 'sick-teller-4-day-closed');
    } finally {
      if (started === 'started') {
        await cutOff(manager).catch(() => {});
      }
      await tellerA.goto('/staff').catch(() => {});
      await tellerB.goto('/staff').catch(() => {});
      await manager.goto('/staff').catch(() => {});
      await ensureShiftClosed(tellerA, manager).catch(() => {});
      await ensureShiftClosed(tellerB, manager).catch(() => {});
      await clockOutIfClockedIn(tellerA).catch(() => {});
      await clockOutIfClockedIn(tellerB).catch(() => {});
      await Promise.all([
        tellerA.close().catch(() => {}),
        tellerB.close().catch(() => {}),
        manager.close().catch(() => {}),
      ]);
    }
  });
});
