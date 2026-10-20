// #1683 booking end-to-end check and #1605 app screenshots, against a running v3 host.
// Accounts come from ../users.json (gitignored). Screenshots go to ../results/.
import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, readFileSync } from 'node:fs';

const users = JSON.parse(readFileSync(new URL('../users.json', import.meta.url), 'utf8'));
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
      await page.getByRole('menuitemradio', { name: /^XC-B$/ }).click();
      await expect(page.locator('.account-bar button[aria-haspopup="menu"]')).toContainText('XC-B', { timeout: 20_000 });
      await page.goto('/#booking-slots');
      await expect(page.locator('input[name="capacity"]')).toBeVisible({ timeout: 15_000 });
      console.log(`[admin] switched session branch from "${startBranch}" to XC-B`);
    }
    await shot(page, 'admin-1b-booking-slots-xcb');

    // Capacity: a slot two days ahead at 09:00 has no bookings from this suite.
    const day = new Date(Date.now() + 7 * 3600_000 + 2 * 24 * 3600_000).toISOString().slice(0, 10);
    const slot = page.locator('.slot-item').filter({ has: page.locator('.slot-date', { hasText: day }) }).first();
    await expect(slot).toBeVisible({ timeout: 15_000 });
    const capInput = slot.locator('input[name="slotCapacity"]');
    const before = await capInput.inputValue();
    await capInput.fill('2');
    await slot.locator('.slot-capacity-control button').click();
    await page.reload();
    await expect(page.locator('.slot-item').filter({ has: page.locator('.slot-date', { hasText: day }) }).first()
      .locator('input[name="slotCapacity"]')).toHaveValue('2', { timeout: 15_000 });
    await shot(page, 'admin-2-capacity-2');
    console.log(`[admin] slot ${day} 09:00 capacity ${before} -> 2`);

    // Closure: a date five days ahead.
    const closed = new Date(Date.now() + 7 * 3600_000 + 5 * 24 * 3600_000).toISOString().slice(0, 10);
    await page.locator('input[name="closedOn"]').fill(closed);
    await page.locator('input[name="reason"]').fill('e2e closure');
    await page.locator('.closures-form button').click();
    await expect(page.locator('.closure-item', { hasText: closed })).toBeVisible({ timeout: 15_000 });
    await shot(page, 'admin-3-closure-added');

    // C02 as the customer: the closed date should offer no slot.
    const cust = await browser.newPage({ baseURL: users.baseUrl, viewport: { width: 430, height: 932 } });
    await signIn(cust, users.customer.phone, users.customer.password, '/app/book');
    await cust.getByTestId('branch-select').selectOption(users.branch);
    await cust.getByTestId('date-input').fill(closed);
    await cust.waitForLoadState('networkidle');
    const open = await cust.locator('[data-testid="slot-option"]:not([disabled])').count();
    await shot(cust, 'admin-4-c02-closed-date');
    if (open > 0) console.log(`[admin] FINDING: C02 still offers ${open} open slot(s) on the closed date ${closed} (closureAdd does not close existing slots at da38eda4; main fixes it)`);
    else console.log(`[admin] C02 offers no slot on the closed date ${closed}`);
    await cust.close();

    // Restore: remove the closure, put the capacity back.
    await page.locator('.closure-item', { hasText: closed }).getByRole('button').click();
    await expect(page.locator('.closure-item', { hasText: closed })).toHaveCount(0, { timeout: 15_000 });
    const again = page.locator('.slot-item').filter({ has: page.locator('.slot-date', { hasText: day }) }).first();
    await again.locator('input[name="slotCapacity"]').fill(before);
    await again.locator('.slot-capacity-control button').click();
    await page.reload();
    await expect(page.locator('.slot-item').filter({ has: page.locator('.slot-date', { hasText: day }) }).first()
      .locator('input[name="slotCapacity"]')).toHaveValue(before, { timeout: 15_000 });
    console.log(`[admin] restored: closure ${closed} removed, capacity back to ${before}`);
    const original = startBranch.replace(/^สาขา\s*|^Branch\s*/i, '').trim();
    if (original && !/XC-B/.test(startBranch)) {
      await page.locator('.account-bar button[aria-haspopup="menu"]').click();
      await page.getByRole('menuitemradio', { name: original }).click();
      console.log(`[admin] session branch back to "${original}"`);
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
