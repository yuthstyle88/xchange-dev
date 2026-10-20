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
    await page.goto(spec.path);
    await expect(page.locator(spec.key).first(), `${spec.path} (${lang}): key element ${spec.key}`).toBeVisible({ timeout: 20_000 });
    await shot(page, spec.area, spec.screen, lang);
    expect(log.errors, `${spec.path} (${lang}): console errors`).toEqual([]);
  }
}
