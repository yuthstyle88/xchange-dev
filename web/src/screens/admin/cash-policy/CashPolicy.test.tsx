// CA-9b #1372: the admin cash-transfer limit policy screen (design `docs/design/blueprint/cash-shift-transfer.md` §4.7
// `cash_transfer_policy`, §9 row CA-9b; owner A5: "the limit is data EXECUTIVE sets"). Written before the
// implementation by the up-front test writer (claude/709ec749, on the xchange FE lane controller claude/5edf7bb5's
// order). Do not edit, delete or weaken; add tests in a file of your own.
//
// The screen reads the OrgView `cash_transfer_policy` once on open (its `data` is `null` before the first save, else
// `{version, limits: {CUR: str}, setBy, createdAt}`) and saves through `transferPolicySave`,
// `PUT /api/cash-transfer-policy` `{limits: {CUR: str}}` with one `Idempotency-Key` kept until a success, then reads the
// view once more. Both are #1371's (CA-8c, PR 1423), whose generated `cash_transfer_policy.ts` types the fixtures.
//
// Readings carried from #1371's ruling (claude/ad05bf8d, #1371): a refusal for too many decimals reaches the wire as
// `validationFailed` with `CashError::TooManyDecimals`' sentence, so the screen shows **the server's sentence**; a
// permission refusal is `roleNotAllowedHere`. `roleNotAllowedHere` and `idempotencyConflict` show `t('error.<code>')`.
//
// The test's handles ([proposed]; the design names no selectors):
//   - the screen `[data-section="cash-policy"]`; one `input[data-currency="<CUR>"]` per currency;
//   - `button[data-action="save"]`; pressing it again after an unknown outcome is the retry (same key, same body);
//   - the currency rows come from the register the cash screen reads (`admin_cash.currencies`, #1372 §7 [proposed]),
//     plus any currency the current version names.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import {
  makeCashTransferPolicyData,
  makeMe,
  type CashTransferPolicyData,
} from '../../../shared/testing/fixtures';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { setLang } from '../../../shared/i18n';
import { AdminShell } from '../AdminShell';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { menus } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}
interface Texts {
  th: Record<string, string>;
  en: Record<string, string>;
  [other: string]: unknown;
}
const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });
const textFiles = import.meta.glob<Texts>('./i18n.ts', { eager: true });
const sources = import.meta.glob<string>('./**/*.{ts,tsx}', { query: '?raw', import: 'default', eager: true });

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/cash-policy/menu.ts exists').toBeDefined();
  return m!.menu;
}

function texts(): Texts {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/cash-policy/i18n.ts exists').toBeDefined();
  return m!;
}

const SET = 'CashTransferPolicySet';
/** `CashError::TooManyDecimals`' sentence (`api/crates/xc-cash/src/domain/error.rs:17`), as the server sends it. */
const TOO_MANY_DECIMALS = 'จำนวนเงินมีทศนิยมเกินกำหนด';

// ── fixtures ────────────────────────────────────────────────────────────────────────────────────────────────────

function meWith(permissions: string[]) {
  return makeMe({
    id: 'u-exec',
    name: 'ผู้บริหาร',
    roles: [],
    permissions,
    branch: 'MAIN',
    branchName: 'สำนักงานใหญ่',
    branches: [{ id: 'MAIN', code: 'MAIN', name: 'สำนักงานใหญ่' }],
  });
}

/** Strings no arithmetic would give back unchanged: `12.5` (not `12.50`) and `0300.00`-free `300.00`. */
const V1 = makeCashTransferPolicyData({
  version: 'v-0001',
  limits: { USD: '300.00', EUR: '12.5' },
  valid: true,
  invalidEntries: [],
  setBy: 'u-exec',
  createdAt: '2026-10-12T01:00:00Z',
});
const V2 = makeCashTransferPolicyData({
  version: 'v-0002',
  limits: { USD: '450.00' },
  valid: true,
  invalidEntries: [],
  setBy: 'u-exec',
  createdAt: '2026-10-12T02:00:00Z',
});

// ── the fake server ─────────────────────────────────────────────────────────────────────────────────────────────

interface Sent {
  method: string;
  path: string;
  body: string | null;
  key: string | null;
}
type Reply = { status: number; body?: unknown } | 'network';

class Server {
  me = meWith([SET]);
  /** What the view answers, read by read (the last repeats). */
  current: (CashTransferPolicyData | null)[] = [null];
  saves: Reply[] = [{ status: 200, body: V1 }];
  sent: Sent[] = [];
  private reads = 0;
  private puts = 0;

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(raw, 'http://xc.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    const headers = new Headers(init?.headers);
    const body = typeof init?.body === 'string' ? init.body : null;
    this.sent.push({ method, path: u.pathname, body, key: headers.get('idempotency-key') });
    const view = (data: unknown) => json(200, { version: 1, changed: true, data });
    if (method === 'GET') {
      switch (u.pathname) {
        case '/api/v3/views/me':
          return view(this.me);
        case QUEUE_CARD_PATH:
          return json(200, queueCardUnchanged());
        case '/api/v3/views/admin_branches':
          return view({ branches: [{ id: 'MAIN', name: 'สำนักงานใหญ่' }] });
        case '/api/v3/views/admin_cash':
          return view({
            openShifts: [],
            stock: [],
            stats: null,
            reference: { withReference: 0, withoutReference: 0, spread: '0.00', percent: '0.00' },
            currencies: ['THB', 'USD', 'EUR'],
            lastCounted: null,
          });
        case '/api/v3/views/cash_transfer_policy': {
          const data = this.current[Math.min(this.reads, this.current.length - 1)] ?? null;
          this.reads++;
          // an OrgView answer (org_view.rs): `changed`, `versions`, `data`
          return json(200, { changed: true, versions: {}, data });
        }
        default:
          return json(200, { version: 1, changed: true, data: null });
      }
    }
    if (method === 'PUT' && u.pathname === '/api/cash-transfer-policy') {
      const r = this.saves[Math.min(this.puts, this.saves.length - 1)]!;
      this.puts++;
      if (r === 'network') throw new TypeError('Failed to fetch');
      return json(r.status, r.body ?? {});
    }
    return json(404, { error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` } });
  };

  policyReads(): Sent[] {
    return this.sent.filter((s) => s.method === 'GET' && s.path === '/api/v3/views/cash_transfer_policy');
  }

  writes(): Sent[] {
    return this.sent.filter((s) => s.method !== 'GET');
  }
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let server: Server;
let dispose: (() => void) | undefined;
let root: HTMLElement;

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 12; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

/** The menu as the shell mounts it, with the page's one `Dialog` and `Toast`. */
async function mount(over: Partial<Pick<Server, 'current' | 'saves'>> = {}): Promise<HTMLElement> {
  Object.assign(server, over);
  const menu = theMenu();
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-12', to: '2026-10-12' });
  const [shown] = createSignal(true);
  const Component = menu.Component;
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  await settle();
  return root;
}

/** The real admin entry: AdminShell with the menus glob, at `#cash-policy`. */
async function mountShell(permissions: string[]): Promise<HTMLElement> {
  server.me = meWith(permissions);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(() => <AdminShell api={new ApiClient()} menus={menus} />, root);
  await settle();
  await settle();
  return root;
}

beforeEach(() => {
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
  vi.stubGlobal('matchMedia', (query: string) => ({
    media: query,
    matches: false,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    onchange: null,
    dispatchEvent: () => false,
  }));
  location.hash = '#cash-policy';
  setLang('th');
});

afterEach(async () => {
  await settle();
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  document.body.innerHTML = '';
  vi.useRealTimers();
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
});

const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const screen = (el: ParentNode) => el.querySelector<HTMLElement>('[data-section="cash-policy"]');
const input = (el: ParentNode, cur: string) => el.querySelector<HTMLInputElement>(`input[data-currency="${cur}"]`);
const saveButton = (el: ParentNode) => el.querySelector<HTMLButtonElement>('button[data-action="save"]');

function type(el: HTMLInputElement | null, value: string): void {
  expect(el, 'the input is there').not.toBeNull();
  el!.value = value;
  el!.dispatchEvent(new Event('input', { bubbles: true }));
  el!.dispatchEvent(new Event('change', { bubbles: true }));
}

async function save(el: ParentNode): Promise<void> {
  const b = saveButton(el);
  expect(b, 'the save button').not.toBeNull();
  b!.click();
  await settle();
}

const bodyOf = (s: Sent) => JSON.parse(s.body ?? 'null') as Record<string, unknown>;

// ── the real admin entry and the menu's permission ──────────────────────────────────────────────────────────────

describe('the real admin entry (AdminShell with the menus glob)', () => {
  it('with CashTransferPolicySet the cash-policy menu is in the nav and #cash-policy draws the screen', async () => {
    const el = await mountShell([SET]);
    expect(el.querySelector('a[href="#cash-policy"]'), 'the nav item').not.toBeNull();
    expect(screen(el), 'the screen through the shell').not.toBeNull();
  });

  it('without CashTransferPolicySet the menu is hidden and the screen is not drawn', async () => {
    const el = await mountShell(['SystemSettings', 'CashManage']);
    expect(el.querySelector('a[href="#cash-policy"]')).toBeNull();
    expect(screen(el)).toBeNull();
  });

  it("the menu record: id 'cash-policy', needs 'CashTransferPolicySet', grouped beside cash", () => {
    const m = theMenu();
    expect(m.id).toBe('cash-policy');
    expect(m.needs).toBe(SET);
    const cash = menus.find((x) => x.id === 'cash');
    expect(cash, 'the cash menu').toBeDefined();
    expect(m.group).toBe(cash!.group);
  });
});

// ── the read ────────────────────────────────────────────────────────────────────────────────────────────────────

describe('the read (one, on open)', () => {
  it('null: an empty form, the "no limit set yet" line, and exactly one read', async () => {
    const el = await mount({ current: [null] });
    expect(server.policyReads()).toHaveLength(1);
    const s = screen(el);
    expect(s).not.toBeNull();
    for (const cur of ['THB', 'USD', 'EUR']) expect(input(s!, cur)?.value, cur).toBe('');
    const noLimit = texts().th.noLimit;
    expect(noLimit, 'th.noLimit').toBeTruthy();
    expect(text(s)).toContain(noLimit!);
  });

  it("a version: each limit as the server's string, with version, setBy and createdAt", async () => {
    const el = await mount({ current: [V1] });
    const s = screen(el)!;
    expect(input(s, 'USD')?.value).toBe('300.00');
    expect(input(s, 'EUR')?.value).toBe('12.5');
    expect(input(s, 'THB')?.value).toBe('');
    for (const shown of [V1.version, V1.setBy]) expect(text(s)).toContain(shown);
    expect(text(s)).not.toContain(texts().th.noLimit ?? '\u0000');
  });

  it('no read on a timer: 10 minutes of fake time add none', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'setInterval', 'clearTimeout', 'clearInterval'] });
    await mount({ current: [V1] });
    expect(server.policyReads()).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    expect(server.policyReads()).toHaveLength(1);
  });
});

// ── the save ────────────────────────────────────────────────────────────────────────────────────────────────────

describe('the save (transferPolicySave)', () => {
  it('the first save: PUT {limits} of the typed strings only, a key, then one more read', async () => {
    const el = await mount({ current: [null, V1], saves: [{ status: 200, body: V1 }] });
    const s = screen(el)!;
    type(input(s, 'USD'), '300.00');
    type(input(s, 'EUR'), '12.5');
    await save(s);
    const w = server.writes();
    expect(w).toHaveLength(1);
    expect(w[0]!.method).toBe('PUT');
    expect(w[0]!.path).toBe('/api/cash-transfer-policy');
    const body = bodyOf(w[0]!);
    expect(body.limits).toEqual({ USD: '300.00', EUR: '12.5' });
    expect(Object.keys(body).filter((k) => k !== 'branch'), 'only limits (and the seam branch)').toEqual(['limits']);
    expect(w[0]!.key, 'an Idempotency-Key').toBeTruthy();
    expect(server.policyReads(), 'one read after the save').toHaveLength(2);
    expect(text(screen(el))).toContain(V1.version);
  });

  it('two saves: the screen shows the newest', async () => {
    const el = await mount({
      current: [null, V1, V2],
      saves: [
        { status: 200, body: V1 },
        { status: 200, body: V2 },
      ],
    });
    type(input(screen(el)!, 'USD'), '300.00');
    await save(screen(el)!);
    type(input(screen(el)!, 'USD'), '450.00');
    await save(screen(el)!);
    const w = server.writes();
    expect(w).toHaveLength(2);
    expect(w[1]!.key, 'two saves, two keys').not.toBe(w[0]!.key);
    const s = screen(el)!;
    expect(text(s)).toContain(V2.version);
    expect(input(s, 'USD')?.value).toBe('450.00');
  });

  it("too many decimals: the server's sentence, and no second request", async () => {
    const el = await mount({
      current: [null],
      saves: [{ status: 400, body: { error: { code: 'validationFailed', message: TOO_MANY_DECIMALS } } }],
    });
    type(input(screen(el)!, 'USD'), '1.001');
    const before = server.sent.length;
    await save(screen(el)!);
    expect(text(document.body)).toContain(TOO_MANY_DECIMALS);
    expect(server.sent.slice(before), 'the one PUT and nothing else').toHaveLength(1);
  });

  it.each(['roleNotAllowedHere', 'idempotencyConflict'])('%s: t(error.<code>), in TH and EN', async (code) => {
    const { th, en } = texts();
    expect(th[`error.${code}`], `th error.${code}`).toBeTruthy();
    expect(en[`error.${code}`], `en error.${code}`).toBeTruthy();
    const el = await mount({
      current: [null],
      saves: [{ status: 409, body: { error: { code, message: 'server words' } } }],
    });
    type(input(screen(el)!, 'USD'), '300.00');
    await save(screen(el)!);
    expect(text(document.body)).toContain(th[`error.${code}`]!);
  });

  it('a network failure, then save again: the same key and a byte-equal body', async () => {
    const el = await mount({ current: [null, V1], saves: ['network', { status: 200, body: V1 }] });
    type(input(screen(el)!, 'USD'), '300.00');
    await save(screen(el)!);
    await save(screen(el)!);
    const w = server.writes();
    expect(w).toHaveLength(2);
    expect(w[1]!.key, 'the same key on the retry').toBe(w[0]!.key);
    expect(w[1]!.body, 'the same body on the retry').toBe(w[0]!.body);
  });
});

// ── texts and the source ────────────────────────────────────────────────────────────────────────────────────────

describe('texts and the source', () => {
  it('i18n.ts exports exactly th and en, with the same keys, no empty text, no zh', () => {
    const t = texts();
    expect(Object.keys(t).filter((k) => k !== 'default').sort()).toEqual(['en', 'th']);
    expect(Object.keys(t.en).sort()).toEqual(Object.keys(t.th).sort());
    for (const [k, v] of Object.entries(t.th)) expect(v, `th.${k}`).toBeTruthy();
    for (const [k, v] of Object.entries(t.en)) expect(v, `en.${k}`).toBeTruthy();
  });

  it('amounts stay strings and no role is named', () => {
    const own = Object.entries(sources).filter(([p]) => !p.includes('.test.'));
    expect(own.length, 'the folder has source files').toBeGreaterThan(0);
    const role = /['"`](EXECUTIVE|ADMIN|RATE_MANAGER|BRANCH_MANAGER|TELLER|FINANCE|MARKETING|VERIFICATION_REVIEWER|OWNER|MANAGER|FRONT|BACKEND)['"`]/;
    for (const [path, src] of own) {
      src.split('\n').forEach((line, i) => {
        expect(/Number\(|parseFloat|parseInt|Math\.|toFixed/.test(line), `${path}:${i + 1} ${line.trim()}`).toBe(false);
        expect(role.test(line), `${path}:${i + 1} ${line.trim()}`).toBe(false);
      });
    }
  });
});
