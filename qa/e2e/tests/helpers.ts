// Shared helpers for every spec (design docs/design/qa-e2e.md §2). Specs import only from here; a spec that needs a new
// helper says so on its ticket and never edits this file.
import { expect, test, type Page } from '@playwright/test';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';

export type Role = 'customer' | 'teller' | 'branchManager' | 'finance' | 'admin';
export type Lang = 'th' | 'en' | 'zh';

/** One account: staff and customers sign in with `phone`, the admin with `identifier`. */
export interface Account {
  phone?: string;
  identifier?: string;
  password: string;
}

/** The shape of users.json (users.example.json shows it; deploy/server/seed-test-booking.sh --json writes it). */
export interface Users {
  baseUrl: string;
  customer: Account;
  teller: Account;
  branchManager?: Account;
  finance?: Account;
  admin: Account;
  branch: string;
  referralCode?: string;
  currency?: string;
  quantity?: string;
}

/** One screen for `smoke`. */
export interface ScreenSpec {
  /** Results folder, the spec's area: `public`, `app`, `admin`, `staff`, `desk`. */
  area: string;
  /** File-name part of the screenshot, e.g. `book` or `admin-rates`. */
  screen: string;
  /** Route to open, e.g. `/app/book` or `/#rates`. */
  path: string;
  /** Who the page must be signed in as before `smoke` runs (`anonymous` = nobody). */
  role: Role | 'anonymous';
  /**
   * A Playwright selector string for the element that proves the screen rendered, cited in the spec from the screen's
   * source: `data-testid=book-screen`, a CSS selector, or `text=<the title from the screen's i18n.ts>`.
   */
  key: string;
  langs: readonly Lang[];
}

/** Languages each audience may pick (web/src/shared/i18n/languages.ts:7-8). */
export const CUSTOMER_LANGS: readonly Lang[] = ['th', 'en', 'zh'];
export const STAFF_LANGS: readonly Lang[] = ['th', 'en'];

/** What the app writes to <html lang> for each language (web/src/shared/i18n/languages.ts:19-23). */
export const HTML_LANG: Record<Lang, string> = { th: 'th', en: 'en', zh: 'zh-CN' };

/**
 * users.json, or users.example.json when users.json is missing, so `--list` and `tsc` work on a machine with no
 * secrets. A run with the example file fails at sign-in, by design (E-3: only the owner runs against a host).
 */
export function loadUsers(): Users {
  const real = new URL('../users.json', import.meta.url);
  const file = existsSync(real) ? real : new URL('../users.example.json', import.meta.url);
  return JSON.parse(readFileSync(file, 'utf8')) as Users;
}

export const users: Users = loadUsers();

/** True when users.json carries a real password for the role (the example file's `<…>` placeholders do not count). */
export function hasAccount(role: Role): boolean {
  const account = users[role];
  return !!account?.password && !account.password.startsWith('<');
}

/**
 * Signs in at /login?next=… (web/src/screens/login/LoginScreen.tsx:114, :121, :126) and waits until the page has left
 * /login and is on `next`. Skips the test when users.json has no account for the role.
 */
export async function signIn(page: Page, role: Role, next: string): Promise<void> {
  test.skip(!hasAccount(role), `no ${role} account in users.json: run scripts/reseed.sh`);
  const account = users[role]!;
  const identifier = account.identifier ?? account.phone ?? '';
  await page.goto(`/login?next=${encodeURIComponent(next)}`);
  await page.locator('input[autocomplete="username"]').fill(identifier);
  await page.locator('input[autocomplete="current-password"]').fill(account.password);
  await page.locator('form button[type="submit"]').click();
  const path = next.split(/[?#]/)[0];
  try {
    // Leave /login first: with next = '/', every path (/login included) would already match.
    await page.waitForURL((url) => !url.pathname.startsWith('/login') && url.pathname.startsWith(path), { timeout: 20_000 });
  } catch {
    const changePw = await page.getByRole('heading', { name: /เปลี่ยนรหัสผ่าน|Change password/i }).count();
    const message = (await page.locator('#message, [role="alert"]').allInnerTexts()).join(' ').trim();
    throw new Error(
      changePw
        ? `${role} (${identifier}) must change its password first: do it once by hand, then update users.json`
        : `${role} (${identifier}) stayed on /login: ${message || 'no message shown'}`,
    );
  }
}

/**
 * Picks the language before the next navigation: the app reads localStorage 'xc.lang' at start-up
 * (web/src/shared/i18n/index.ts:9-17). A language the audience does not allow falls back to th (index.ts:40-44).
 */
export async function withLang(page: Page, lang: Lang): Promise<void> {
  await page.addInitScript((l) => {
    try {
      window.localStorage.setItem('xc.lang', l);
    } catch {
      // storage blocked: the page shows th
    }
  }, lang);
}

export interface ConsoleLog {
  errors: string[];
}

/** Collects console errors and uncaught page errors from now on. `smoke` fails a screen that logs any. */
export function watchConsole(page: Page): ConsoleLog {
  const log: ConsoleLog = { errors: [] };
  page.on('console', (msg) => {
    if (msg.type() === 'error') log.errors.push(msg.text());
  });
  page.on('pageerror', (err) => log.errors.push(String(err)));
  return log;
}

/** Full-page screenshot at results/<area>/<screen>[-<lang>].png. */
export async function shot(page: Page, area: string, screen: string, lang?: Lang): Promise<void> {
  const dir = new URL(`../results/${area}/`, import.meta.url).pathname;
  mkdirSync(dir, { recursive: true });
  await page.screenshot({ path: `${dir}${screen}${lang ? `-${lang}` : ''}.png`, fullPage: true });
}

/**
 * The smoke tier (§4): for each language, open the screen, expect its key element, expect no console error, and take a
 * screenshot. It never clicks anything. The caller signs `page` in as `spec.role` first (`signIn`), once per spec file.
 */
export async function smoke(page: Page, spec: ScreenSpec): Promise<void> {
  const log = watchConsole(page);
  for (const lang of spec.langs) {
    log.errors.length = 0;
    await withLang(page, lang);
    // A fresh document every time: admin menus differ only by hash (/#rates), and a same-page hash change would not
    // re-run withLang's init script.
    await page.goto('about:blank');
    await page.goto(spec.path);
    await expect(page.locator(spec.key).first(), `${spec.path} (${lang}): key element ${spec.key}`).toBeVisible({ timeout: 20_000 });
    // The app writes the chosen language to <html lang> (web/src/shared/i18n/languages.ts:19-23, index.ts syncHtmlLang).
    await expect(page.locator('html'), `${spec.path} shows ${lang}`).toHaveAttribute('lang', HTML_LANG[lang]);
    await shot(page, spec.area, spec.screen, lang);
    expect(log.errors, `${spec.path} (${lang}): console errors`).toEqual([]);
  }
}

// ---- Shift steps shared by flow-cash (F3) and flow-queue (F2): a queue counter is a teller with an OPEN shift
// (docs/design/blueprint/queue-tickets.md Q7). Lifted from PR 3838's shift.spec.ts.

/** Ten of the first THB note at open and at close, so the shift balances. */
export const SHIFT_QTY = '10';

/** Waits for one POST whose path matches while `act` runs; returns `ok`, `<status> <body>` and the raw body. */
export async function posted(page: Page, path: RegExp, act: () => Promise<void>): Promise<{ ok: boolean; text: string; body: string }> {
  const res = page.waitForResponse((r) => r.request().method() === 'POST' && path.test(new URL(r.url()).pathname), { timeout: 15_000 });
  await act();
  const r = await res;
  const body = await r.text().catch(() => '');
  return { ok: r.ok(), text: `${r.status()} ${body.slice(0, 300)}`, body };
}

/**
 * The shift buttons show only to someone clocked in (web/src/screens/staff/shift/ShiftCard.tsx, floor.myClockIn), so
 * clock in with the floor card (FloorCard.tsx floor-clock, POST /api/attendance) when needed. `page` is on /staff.
 */
export async function clockIn(page: Page): Promise<string> {
  const shiftButton = page.getByTestId('shift-open-count').or(page.getByTestId('shift-close-count')).first();
  if (await shiftButton.isVisible().catch(() => false)) return 'already clocked in';
  const clock = page.getByTestId('floor-clock');
  await expect(clock, 'the floor card clock button').toBeVisible({ timeout: 20_000 });
  const res = await posted(page, /^\/api\/attendance$/, () => clock.click());
  await expect(shiftButton, `shift buttons after clock-in (POST /api/attendance ${res.text})`).toBeVisible({ timeout: 20_000 });
  return `clocked in: POST /api/attendance ${res.text}`;
}

/** The teller fills the count form (first THB row, SHIFT_QTY) and saves it; returns POST /api/cash-counts's answer. */
export async function submitCount(page: Page, kind: 'open' | 'close'): Promise<{ ok: boolean; text: string }> {
  await page.getByTestId(kind === 'open' ? 'shift-open-count' : 'shift-close-count').click();
  const form = page.getByTestId('shift-count-form');
  await expect(form).toBeVisible({ timeout: 15_000 });
  const thb = form.locator('[data-denomination-row]').filter({ hasText: 'THB' }).first();
  await expect(thb, 'a THB denomination row in the count form').toBeVisible({ timeout: 15_000 });
  await thb.locator('input[data-denomination-id]').fill(SHIFT_QTY);
  return posted(page, /^\/api\/cash-counts$/, () => page.getByTestId('shift-count-save').click());
}

/** The branch manager verifies the newest count of `kind`; returns POST /api/cash-counts/{id}/verify's answer. */
export async function verifyCount(page: Page, kind: 'OPEN' | 'CLOSE'): Promise<{ ok: boolean; text: string }> {
  await page.reload();
  const item = page.locator('[data-count-id]').filter({ hasText: kind }).first();
  await expect(item, `a ${kind} count waiting for verification`).toBeVisible({ timeout: 20_000 });
  return posted(page, /^\/api\/cash-counts\/[^/]+\/verify$/, () => item.getByTestId('shift-verify').click());
}

/**
 * Opens the teller's shift: the teller (on /staff) clocks in and counts the opening, the branch manager (on /staff)
 * verifies it; the verifier is never the counter (api/crates/xc-cash/src/domain/person_rules.rs). Needs a funded safe
 * (flow-cash's first test). Throws with the server's answer when a step is refused.
 */
export async function openShift(teller: Page, manager: Page): Promise<void> {
  console.log(`[shift] ${await clockIn(teller)}`);
  const count = await submitCount(teller, 'open');
  expect(count.ok, `opening count accepted: ${count.text}`).toBeTruthy();
  const verify = await verifyCount(manager, 'OPEN');
  expect(verify.ok, `opening count verified: ${verify.text}`).toBeTruthy();
  await teller.reload();
  await expect(teller.getByTestId('shift-close-count'), 'the shift is open').toBeVisible({ timeout: 20_000 });
}

/** Closes it with the same figures and clocks the teller out, so the next run starts from the same state. */
export async function closeShift(teller: Page, manager: Page): Promise<void> {
  console.log(`[shift] ${await clockIn(teller)}`);
  const count = await submitCount(teller, 'close');
  expect(count.ok, `closing count accepted: ${count.text}`).toBeTruthy();
  const verify = await verifyCount(manager, 'CLOSE');
  expect(verify.ok, `closing count verified: ${verify.text}`).toBeTruthy();
  await teller.reload();
  await expect(teller.getByTestId('shift-open-count'), 'the shift is closed').toBeVisible({ timeout: 20_000 });
  await teller.getByTestId('floor-clock').click().catch(() => {});
}
