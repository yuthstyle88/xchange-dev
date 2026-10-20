// Shared helpers for every spec (design docs/design/qa-e2e.md §2). Specs import only from here; a spec that needs a new
// helper says so on its ticket and never edits this file.
import { expect, test, type BrowserContext, type Page } from '@playwright/test';
import { chmodSync, existsSync, mkdirSync, readFileSync } from 'node:fs';
import { dirname } from 'node:path';

export type Role = 'customer' | 'teller' | 'branchManager' | 'finance' | 'marketing' | 'admin';
export const ROLES: readonly Role[] = ['customer', 'teller', 'branchManager', 'finance', 'marketing', 'admin'];

export const ROLE_HOME: Record<Role, string> = {
  customer: '/app',
  teller: '/staff',
  branchManager: '/staff',
  finance: '/staff',
  marketing: '/',
  admin: '/',
};
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
  marketing?: Account;
  admin: Account;
  branch: string;
  referralCode?: string;
  currency?: string;
  quantity?: string;
}

export type ConsoleAllowKey = 'anonymousMyProfile401' | 'loyaltyUnavailable503' | 'documentsNotFound404';

export interface ConsoleAllowEntry {
  url: RegExp;
  status: number;
  code?: string;
  log?: string;
}

/** Expected HTTP console errors in `watchConsole`; each entry is named and carries a one-line reason. */
export const CONSOLE_ALLOWLIST: Record<ConsoleAllowKey, ConsoleAllowEntry> = {
  // Anonymous visitors on public /app pages have no customer session, so GET /api/v3/views/my_profile answers 401.
  anonymousMyProfile401: {
    url: /\/api\/v3\/views\/my_profile(?:\?|$)/,
    status: 401,
  },
  // The loyalty service is not configured on this host, so GET /api/v3/views/loyalty_rule_versions answers 503 loyaltyUnavailable.
  loyaltyUnavailable503: {
    url: /\/api\/v3\/views\/loyalty_rule_versions(?:\?|$)/,
    status: 503,
    code: 'loyaltyUnavailable',
    log: 'loyalty not configured on host',
  },
  // No ?ref= is a refusal by design (documents.rs:125); GET answers 404 documentsNotFound.
  documentsNotFound404: {
    url: /\/api\/v3\/views\/(staff_)?documents(?:\?|$)/,
    status: 404,
    code: 'documentsNotFound',
  },
};

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
  /** Optional permission from the admin menu's `menu.ts` (`needs`); `smoke` skips when `me.permissions` lacks it. */
  needs?: string;
  /** Expected console errors from `CONSOLE_ALLOWLIST` for this screen. */
  allowConsole?: readonly ConsoleAllowKey[];
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

/** Results subfolder where storageState sessions are saved (results/.auth/<role>.json). */
export function authFileFor(role: Role): string {
  return new URL(`../results/.auth/${role}.json`, import.meta.url).pathname;
}

/** Saves the browser context's cookies and storage state to `results/.auth/<role>.json` with mode 600. */
export async function saveStorageState(page: Page, role: Role): Promise<void> {
  const file = authFileFor(role);
  mkdirSync(dirname(file), { recursive: true, mode: 0o700 });
  try {
    chmodSync(dirname(file), 0o700);
  } catch {
    // best-effort
  }
  await page.context().storageState({ path: file });
  try {
    chmodSync(file, 0o600);
  } catch {
    // best-effort
  }
}

/** Loads the saved storage state from `results/.auth/<role>.json` into the page context. */
export async function loadStorageState(page: Page, role: Role): Promise<boolean> {
  const file = authFileFor(role);
  if (!existsSync(file)) return false;
  try {
    const raw = readFileSync(file, 'utf8');
    const state = JSON.parse(raw) as {
      cookies?: Parameters<BrowserContext['addCookies']>[0];
      origins?: Array<{ origin: string; localStorage: Array<{ name: string; value: string }> }>;
    };
    if (state.cookies && state.cookies.length > 0) {
      await page.context().addCookies(state.cookies);
    }
    if (state.origins && state.origins.length > 0) {
      await page.context().addInitScript((origins) => {
        for (const item of origins) {
          if (window.location.origin === item.origin) {
            for (const entry of item.localStorage) {
              try {
                window.localStorage.setItem(entry.name, entry.value);
              } catch {
                // ignore
              }
            }
          }
        }
      }, state.origins);
    }
    return true;
  } catch {
    return false;
  }
}

/** Drives the login form at /login?next=… directly. */
export async function loginWithForm(page: Page, role: Role, next: string): Promise<void> {
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
    if (changePw) {
      throw new Error(`${role} (${identifier}) must change its password first: do it once by hand, then update users.json`);
    }
    if (/ลองหลายครั้งเกินไป|กรุณารอสักครู่/i.test(message)) {
      throw new Error(`${role} (${identifier}): ${message}`);
    }
    throw new Error(
      `${role} (${identifier}) stayed on /login: ${message || 'no message shown'}`,
    );
  }
}

/**
 * Signs in as `role` and navigates to `next`.
 * Reuses saved session from `results/.auth/<role>.json` when available.
 * Falls back to the login form if the state is missing or the server answers 401 (a stale session),
 * then saves it again.
 */
export async function signIn(page: Page, role: Role, next: string): Promise<void> {
  test.skip(!hasAccount(role), `no ${role} account in users.json: run scripts/reseed.sh`);
  const loaded = await loadStorageState(page, role);
  if (loaded) {
    let got401 = false;
    const onResponse = (res: { status(): number }) => {
      if (res.status() === 401) got401 = true;
    };
    page.on('response', onResponse);
    try {
      const res = await page.goto(next);
      if (res?.status() === 401) got401 = true;
      await page.waitForLoadState('domcontentloaded');
      const pathname = new URL(page.url(), users.baseUrl).pathname;
      if (!next.split(/[?#]/)[0].startsWith('/login') && pathname.startsWith('/login')) {
        got401 = true;
      }
    } catch {
      got401 = true;
    } finally {
      page.off('response', onResponse);
    }
    if (!got401) {
      return;
    }
  }

  await loginWithForm(page, role, next);
  await saveStorageState(page, role);
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
  pending?: Promise<void>[];
}

/** Reads the signed-in staff/admin user's `me.permissions` from `GET /api/v3/views/me` (`AdminShell.tsx:246`). */
export async function mePermissions(page: Page): Promise<string[]> {
  const res = await page.request.get('/api/v3/views/me');
  if (!res.ok()) return [];
  const body = (await res.json().catch(() => ({}))) as { data?: { permissions?: string[] } };
  return Array.isArray(body.data?.permissions) ? body.data.permissions : [];
}

/** Collects console errors and uncaught page errors from now on, excluding any named `CONSOLE_ALLOWLIST` entries. */
export function watchConsole(page: Page, allow: readonly ConsoleAllowKey[] = []): ConsoleLog {
  const pending: Promise<void>[] = [];
  const seen = new Set<ConsoleAllowKey>();
  const log: ConsoleLog = { errors: [], pending };
  const matchesStatus = (text: string, status: number) => new RegExp(`\\b${status}\\b`).test(text);
  page.on('console', (msg) => {
    if (msg.type() !== 'error') return;
    const text = msg.text();
    const locUrl = msg.location().url ?? '';
    const allowed = allow.some((k) => {
      const entry = CONSOLE_ALLOWLIST[k];
      return (
        matchesStatus(text, entry.status) &&
        (entry.url.test(locUrl) || entry.url.test(text) || (seen.has(k) && !/\/api\//.test(locUrl)))
      );
    });
    if (!allowed) log.errors.push(text);
  });
  page.on('response', (res) => {
    for (const k of allow) {
      const entry = CONSOLE_ALLOWLIST[k];
      if (!entry.url.test(res.url()) || res.status() !== entry.status) continue;
      seen.add(k);
      const idx = log.errors.findIndex((e) => matchesStatus(e, entry.status) && /Failed to load resource/i.test(e));
      if (idx !== -1) log.errors.splice(idx, 1);
      if (!entry.code && !entry.log) continue;
      pending.push(
        res
          .text()
          .catch(() => null)
          .then((body) => {
            // The page may navigate before the body is read (e.g. /staff/documents with no ref goes to /staff);
            // then only the URL and status can be checked, and the entry's URL pattern is narrow enough for that.
            if (body === null) return;
            if (entry.code && !body.includes(entry.code)) {
              log.errors.push(`${res.status()} ${res.url()}: ${body.slice(0, 200)}`);
              return;
            }
            if (entry.log) console.log(entry.log);
          }),
      );
    }
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
  if (spec.needs) {
    const perms = await mePermissions(page);
    test.skip(!perms.includes(spec.needs), `${spec.path}: signed-in ${spec.role} lacks ${spec.needs}`);
  }
  const allow =
    spec.role === 'anonymous'
      ? Array.from(new Set<ConsoleAllowKey>(['anonymousMyProfile401', ...(spec.allowConsole ?? [])]))
      : (spec.allowConsole ?? []);
  const log = watchConsole(page, allow);
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
    await page.waitForLoadState('networkidle', { timeout: 15_000 }).catch(() => {});
    if (log.pending?.length) await Promise.all(log.pending.splice(0));
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
  const clock = page.getByTestId('floor-clock');
  await expect(clock, 'the floor card clock button').toBeVisible({ timeout: 20_000 });
  const shiftButton = page.getByTestId('shift-open-count').or(page.getByTestId('shift-close-count')).first();
  if (await shiftButton.isVisible().catch(() => false)) return 'already clocked in';
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

/** If an earlier run left the teller's shift open, close and verify it first so the flow starts from a closed shift. */
export async function ensureShiftClosed(teller: Page, manager: Page): Promise<void> {
  console.log(`[shift] ${await clockIn(teller)}`);
  if (await teller.getByTestId('shift-close-count').isVisible().catch(() => false)) {
    console.log('[shift] leftover open shift found; closing it first');
    const count = await submitCount(teller, 'close');
    expect(count.ok, `leftover closing count accepted: ${count.text}`).toBeTruthy();
    const verify = await verifyCount(manager, 'CLOSE');
    expect(verify.ok, `leftover closing count verified: ${verify.text}`).toBeTruthy();
    await teller.reload();
    await expect(teller.getByTestId('shift-open-count'), 'the leftover shift is closed').toBeVisible({ timeout: 20_000 });
  }
}

/**
 * Opens the teller's shift: the teller (on /staff) clocks in and counts the opening, the branch manager (on /staff)
 * verifies it; the verifier is never the counter (api/crates/xc-cash/src/domain/person_rules.rs). Needs a funded safe
 * (flow-cash's first test). Throws with the server's answer when a step is refused.
 */
export async function openShift(teller: Page, manager: Page): Promise<void> {
  await ensureShiftClosed(teller, manager);
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
