// Shift open and close on /staff (staff shift card, web/src/screens/staff/shift/ShiftCard.tsx), against a running v3.
// The teller counts, the branch manager verifies: the verifier must not be the counter
// (api/crates/xc-cash/src/domain/person_rules.rs verifier_is_not_counter). Accounts come from ../users.json.
import { test, expect, type Page, type Browser } from '@playwright/test';
import { mkdirSync, readFileSync } from 'node:fs';

const users = JSON.parse(readFileSync(new URL('../users.json', import.meta.url), 'utf8'));
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

test.describe.serial('shift open and close', () => {
  test.skip(!users.branchManager?.password || String(users.branchManager.password).startsWith('<'),
    'no branchManager in users.json: run scripts/reseed.sh');

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
