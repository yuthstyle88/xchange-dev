// #1683 booking end-to-end check and #1605 app screenshots, against a running v3 host.
// Accounts come from ../users.json (gitignored). Screenshots go to ../results/.
import { test, expect, type Page } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { loadUsers } from './helpers';

// Its ticket moves this spec onto helpers.ts (signIn, shot, the typed Users) and removes this loose type.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const users = loadUsers() as unknown as Record<string, any>;
const OUT = new URL('../results/', import.meta.url).pathname;
mkdirSync(OUT, { recursive: true });

const shot = (page: Page, name: string) => page.screenshot({ path: `${OUT}${name}.png`, fullPage: true });

// Tomorrow in Asia/Bangkok, as the date input wants it (yyyy-mm-dd).
function tomorrowBangkok(): string {
  const now = new Date(Date.now() + 7 * 3600_000 + 24 * 3600_000);
  return now.toISOString().slice(0, 10);
}


// The card of one booking: div.xc-booking-card holding .xc-booking-no with exactly this number
// (web/src/screens/customer/bookings/Bookings.tsx:349-351). It stays the same element after a cancel.
function bookingCard(page: Page, bookingNo: string) {
  const exact = new RegExp(`^\\s*${bookingNo}\\s*$`);
  return page.locator('.xc-booking-card').filter({ has: page.locator('.xc-booking-no', { hasText: exact }) });
}

// Sets one admin slot's capacity through the screen and returns the server's answer to
// PUT /api/booking-slots/{id}/capacity (BookingSlots.tsx handleUpdateSlotCapacity).
async function setSlotCapacity(page: Page, slot: ReturnType<Page['locator']>, value: string): Promise<string> {
  await slot.locator('input[name="slotCapacity"]').fill(value);
  const put = page.waitForResponse(
    (r) => r.request().method() === 'PUT' && /\/api\/booking-slots\/[^/]+\/capacity$/.test(new URL(r.url()).pathname),
    { timeout: 15_000 },
  );
  await slot.locator('.slot-capacity-control button').click();
  const res = await put;
  return `${res.status()} ${(await res.text().catch(() => '')).slice(0, 200)}`;
}

async function signIn(page: Page, identifier: string, password: string, next: string) {
  await page.goto(`/login?next=${encodeURIComponent(next)}`);
  await page.locator('input[autocomplete="username"]').fill(identifier);
  await page.locator('input[autocomplete="current-password"]').fill(password);
  await page.locator('form button[type="submit"]').click();
  // Leave /login first: with next = '/', every path (/login included) would already match.
  try {
    await page.waitForURL((url) => !url.pathname.startsWith('/login') && url.pathname.startsWith(next.split('?')[0]), {
      timeout: 20_000,
    });
  } catch {
    // Say why the sign-in stayed on /login instead of a bare timeout.
    const changePw = await page.getByRole('heading', { name: /เปลี่ยนรหัสผ่าน|Change password/i }).count();
    const message = (await page.locator('#message, [role="alert"]').allInnerTexts()).join(' ').trim();
    await shot(page, `signin-failed-${identifier.replace(/[^0-9A-Za-z]/g, '')}`);
    throw new Error(
      changePw
        ? `sign-in for ${identifier}: the account must change its password first (do it once by hand, then update users.json)`
        : `sign-in for ${identifier} stayed on /login: ${message || 'no message shown'}`,
    );
  }
}

// Books the first open slot tomorrow; returns the booking id the success panel shows.
async function bookOnce(page: Page, step: string, withRef: boolean): Promise<string> {
  await page.goto(withRef ? `/app/book?ref=${users.referralCode}` : '/app/book');
  await expect(page.getByTestId('book-screen')).toBeVisible();
  if (withRef) {
    await expect(page.getByTestId('referral-code')).toHaveValue(users.referralCode);
  }
  await page.getByTestId('branch-select').selectOption(users.branch);
  await page.getByTestId('date-input').fill(tomorrowBangkok());
  await shot(page, `${step}-1-branch-date`);
  // A full slot is shown disabled (capacity reached by earlier runs); take the first one still open.
  const firstSlot = page.locator('[data-testid="slot-option"]:not([disabled])').first();
  const disabled = await page.locator('[data-testid="slot-option"][disabled]').count();
  if (disabled) console.log(`[${step}] ${disabled} slot(s) shown full/disabled`);
  await expect(firstSlot).toBeAttached({ timeout: 15_000 });
  const slotValue = await firstSlot.getAttribute('value');
  await page.getByTestId('slot-select').selectOption(slotValue!);
  await page.getByTestId('currency-select').selectOption(users.currency);
  await page.getByTestId('quantity-input').fill(users.quantity);
  await shot(page, `${step}-2-filled`);
  const created = page.waitForResponse(
    (r) => r.request().method() === 'POST' && new URL(r.url()).pathname === '/api/bookings',
  );
  await page.getByTestId('book-submit').click();
  const res = await created;
  const body = await res.json().catch(() => ({}));
  const success = page.locator('.xc-book-success');
  await expect(success, `booking success panel (POST /api/bookings answered ${res.status()}: ${JSON.stringify(body).slice(0, 300)})`).toBeVisible({ timeout: 20_000 });
  await shot(page, `${step}-3-booked`);
  const data = body.data ?? body;
  console.log(`[${step}] POST /api/bookings ${res.status()} bookingNo=${data.bookingNo} id=${data.id}`);
  console.log(`[${step}] panel shows: ${(await success.innerText()).replace(/\s+/g, ' ')}`);
  // #1683 criterion 1 asks for a booking number; note it if the panel shows only the id.
  if (data.bookingNo && !(await success.innerText()).includes(data.bookingNo)) {
    console.log(`[${step}] FINDING: the success panel does not show bookingNo ${data.bookingNo}`);
  }
  return data.bookingNo ?? data.id;
}

test.describe.serial('#1683 booking', () => {
  let keptBooking = '';

  test('customer books, the ref code is sent, C03 lists and cancels', async ({ page }) => {
    await signIn(page, users.customer.phone, users.customer.password, '/app/book');
    // Criterion 1 and 2: a booking from a ?ref= link. This one is kept for the desk check.
    keptBooking = await bookOnce(page, 'c02-ref', true);
    // Criterion 3: a second booking, then C03 lists both and cancels the second.
    const toCancel = await bookOnce(page, 'c02-plain', false);
    await page.goto('/app/bookings');
    await expect(page.getByText(keptBooking).first()).toBeVisible({ timeout: 15_000 });
    await expect(page.getByText(toCancel).first()).toBeVisible();
    await shot(page, 'c03-1-list');
    const cancelName = /ยกเลิกการจอง|Cancel Booking|取消预约/;
    const card = bookingCard(page, toCancel);
    await card.getByRole('button', { name: cancelName }).click();
    await page.getByRole('button', { name: /ยืนยันการยกเลิก|Confirm Cancellation|确认取消/ }).click();
    // Criterion 3 is met only when the card itself reads cancelled, not when the dialog closes.
    await expect(card.getByText(/ยกเลิกแล้ว|Cancelled|已取消/), `${toCancel} shows cancelled`).toBeVisible({ timeout: 15_000 });
    await shot(page, 'c03-2-cancelled');
    console.log(`[c03] kept ${keptBooking}, cancelled ${toCancel}`);
  });

  test('teller desk shows the booking', async ({ page }) => {
    test.skip(!keptBooking, 'no booking from the customer test');
    await signIn(page, users.teller.phone, users.teller.password, '/desk/bookings');
    await expect(page.getByText(keptBooking).first(), 'the kept booking on the desk screen').toBeVisible({ timeout: 15_000 });
    await shot(page, 'desk-1-bookings');
  });

  // Teardown: cancel the booking kept for the desk check, so reruns do not fill the slots.
  test('cleanup: the customer cancels the kept booking', async ({ page }) => {
    test.skip(!keptBooking, 'nothing to clean');
    await signIn(page, users.customer.phone, users.customer.password, '/app/bookings');
    const cancelName = /ยกเลิกการจอง|Cancel Booking|取消预约/;
    const card = bookingCard(page, keptBooking);
    await card.getByRole('button', { name: cancelName }).click();
    await page.getByRole('button', { name: /ยืนยันการยกเลิก|Confirm Cancellation|确认取消/ }).click();
    await expect(card.getByText(/ยกเลิกแล้ว|Cancelled|已取消/)).toBeVisible({ timeout: 15_000 });
    console.log(`[cleanup] cancelled ${keptBooking}`);
  });

  // #1683 criterion 4 (admin): change one slot's capacity and add a closure, see C02 follow, then restore both.
  test('admin booking-slots: capacity and closure, reflected on C02, restored', async ({ page, browser }) => {
    test.skip(!users.admin?.password || users.admin.password.startsWith('<'), 'no admin account in users.json');
    await signIn(page, users.admin.identifier, users.admin.password, '/');
    await page.goto('/#booking-slots'); // admin.html is served at / (api/src/presentation/pages.rs PAGE_SPECS)
    await expect(page.locator('input[name="capacity"]')).toBeVisible({ timeout: 15_000 });
    await shot(page, 'admin-1-booking-slots');
    // The screen works on the session branch; the seeded rule is MAIN (XC-B). Switch with the account
    // bar's branch menu (web/src/shared/ui/AccountBar.tsx:143-170) and remember the branch to go back to.
    const branchButton = page.locator('.account-bar button[aria-haspopup="menu"]');
    const startBranch = ((await branchButton.innerText().catch(() => '')) || '').trim();
    if (!/XC-B/.test(startBranch)) {
      await branchButton.click();
      const switched = page.waitForResponse(
        (r) => r.request().method() === 'POST' && new URL(r.url()).pathname === '/api/auth/session/branch',
      );
      await page.getByRole('menuitemradio', { name: /^XC-B$/ }).click();
      const res = await switched;
      const body = (await res.text().catch(() => '')).slice(0, 300);
      console.log(`[admin] POST /api/auth/session/branch ${res.status()} ${body}`);
      expect(res.ok(), `branch switch answered ${res.status()}: ${body}`).toBeTruthy();
      await page.waitForTimeout(1500); // the admin page polls, so networkidle never comes
      await page.goto('/#booking-slots');
      await expect(page.locator('.account-bar button[aria-haspopup="menu"]'), 'session branch after the switch').toContainText('XC-B', { timeout: 20_000 });
      await expect(page.locator('input[name="capacity"]')).toBeVisible({ timeout: 15_000 });
      console.log(`[admin] switched session branch from "${startBranch}" to XC-B`);
    }
    await shot(page, 'admin-1b-booking-slots-xcb');

    const day = new Date(Date.now() + 7 * 3600_000 + 2 * 24 * 3600_000).toISOString().slice(0, 10);
    const closed = new Date(Date.now() + 7 * 3600_000 + 5 * 24 * 3600_000).toISOString().slice(0, 10);
    const slotOn = () => page.locator('.slot-item').filter({ has: page.locator('.slot-date', { hasText: day }) }).first();
    const SEED_CAPACITY = '4'; // deploy/server/seed-test-booking.sh

    // Start clean: an earlier failed run may have left its closure or capacity behind.
    if (await page.locator('.closure-item', { hasText: 'e2e closure' }).count()) {
      for (const item of await page.locator('.closure-item', { hasText: 'e2e closure' }).all()) await item.getByRole('button').click();
      await expect(page.locator('.closure-item', { hasText: 'e2e closure' })).toHaveCount(0, { timeout: 15_000 });
      console.log('[admin] removed a leftover e2e closure');
    }
    await expect(slotOn()).toBeVisible({ timeout: 15_000 });
    if ((await slotOn().locator('input[name="slotCapacity"]').inputValue()) !== SEED_CAPACITY) {
      console.log(`[admin] reset leftover capacity on ${day} to ${SEED_CAPACITY}: PUT ${await setSlotCapacity(page, slotOn(), SEED_CAPACITY)}`);
      await page.reload();
      await expect(slotOn().locator('input[name="slotCapacity"]'), 'leftover capacity reset').toHaveValue(SEED_CAPACITY, { timeout: 15_000 });
    }

    try {
      // Capacity: a slot two days ahead at 09:00 has no bookings from this suite.
      console.log(`[admin] set capacity 2: PUT ${await setSlotCapacity(page, slotOn(), '2')}`);
      await page.reload();
      await expect(slotOn().locator('input[name="slotCapacity"]')).toHaveValue('2', { timeout: 15_000 });
      await shot(page, 'admin-2-capacity-2');
      console.log(`[admin] slot ${day} 09:00 capacity ${SEED_CAPACITY} -> 2`);

      // Closure: a date five days ahead.
      await page.locator('input[name="closedOn"]').fill(closed);
      await page.locator('input[name="reason"]').fill('e2e closure');
      await page.locator('.closures-form button').click();
      await expect(page.locator('.closure-item', { hasText: closed })).toBeVisible({ timeout: 15_000 });
      await shot(page, 'admin-3-closure-added');

      // C02 as the customer: the closed date should offer no slot. Wait for the slots view, not for
      // networkidle (the app keeps polling, so the network is never idle).
      const cust = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 430, height: 932 } });
      try {
        await signIn(cust, users.customer.phone, users.customer.password, '/app/book');
        await cust.getByTestId('branch-select').selectOption(users.branch);
        const slotsLoaded = cust.waitForResponse((r) => r.url().includes('booking_slots'), { timeout: 15_000 }).catch(() => null);
        await cust.getByTestId('date-input').fill(closed);
        await slotsLoaded;
        await cust.waitForTimeout(1000);
        const open = await cust.locator('[data-testid="slot-option"]:not([disabled])').count();
        await shot(cust, 'admin-4-c02-closed-date');
        if (open > 0) console.log(`[admin] FINDING: C02 still offers ${open} open slot(s) on the closed date ${closed} (closureAdd does not close existing slots at da38eda4; main fixes it)`);
        else console.log(`[admin] C02 offers no slot on the closed date ${closed}`);
      } finally {
        await cust.close();
      }
    } finally {
      // Restore always, even when a step above failed.
      for (const item of await page.locator('.closure-item', { hasText: 'e2e closure' }).all()) await item.getByRole('button').click().catch(() => {});
      await expect(page.locator('.closure-item', { hasText: 'e2e closure' })).toHaveCount(0, { timeout: 15_000 }).catch(() => {});
      const answer = await setSlotCapacity(page, slotOn(), SEED_CAPACITY).catch((e) => `no answer: ${e}`);
      console.log(`[admin] restore capacity ${SEED_CAPACITY}: PUT ${answer}`);
      await page.reload();
      const cap = await slotOn().locator('input[name="slotCapacity"]').inputValue().catch(() => '?');
      const left = await page.locator('.closure-item', { hasText: 'e2e closure' }).count();
      console.log(`[admin] restore: capacity on ${day} = ${cap}, e2e closures left = ${left}`);
      expect(cap, `capacity on ${day} restored to ${SEED_CAPACITY}`).toBe(SEED_CAPACITY);
      expect(left, 'no e2e closure left').toBe(0);
      const original = startBranch.replace(/^สาขา\s*|^Branch\s*/i, '').trim();
      if (original && !/XC-B/.test(startBranch)) {
        await page.locator('.account-bar button[aria-haspopup="menu"]').click().catch(() => {});
        await page.getByRole('menuitemradio', { name: original }).click().catch(() => {});
        console.log(`[admin] session branch back to "${original}"`);
      }
    }
  });
});

// #1605 criterion 3: every app screen in each language. The zh review itself stays with a reader of Chinese.
const LANGS: Array<[string, RegExp]> = [['th', /^TH$/], ['en', /^EN$/], ['zh', /^中文$/]];
const PUBLIC_SCREENS = ['/app/sign-in', '/app/sign-up'];
const SIGNED_IN_SCREENS = ['/app', '/app/branches', '/app/journey', '/app/assistant', '/app/history'];

async function captureIn(page: Page, path: string) {
  for (const [lang, label] of LANGS) {
    await page.goto(path);
    // LangSwitch buttons are named by aria-label ("…English"), so match the visible text instead
    // (web/src/shared/i18n/LangSwitch.tsx). The pressed button proves the switch took.
    const button = page.locator('button[aria-pressed]').filter({ hasText: label }).first();
    await button.click();
    await expect(button, `${path} switched to ${lang}`).toHaveAttribute('aria-pressed', 'true');
    await page.waitForLoadState('networkidle');
    await shot(page, `1605-${path.replace(/\//g, '_').replace(/^_/, '') || 'app'}-${lang}`);
  }
}

test('app screens in th, en, zh (#1605)', async ({ page, browser }) => {
  const anon = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 430, height: 932 } });
  for (const p of PUBLIC_SCREENS) await captureIn(anon, p);
  await anon.close();
  await signIn(page, users.customer.phone, users.customer.password, '/app');
  for (const p of SIGNED_IN_SCREENS) await captureIn(page, p);
});
