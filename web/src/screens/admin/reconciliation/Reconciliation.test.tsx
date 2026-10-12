// FN-1 (#1068): F01, FINANCE's reconcile-and-certify screen.
// Written before the implementation by the up-front test writer (claude/07d3765f) on the lane
// controller's order. Do not edit, delete or weaken; add tests in a file of your own.
//
// Design: `docs/design/blueprint/executive-finance.md` §5 (the sheet and certify), §8.2 (the two
// routes). §9's last row: "shows each part, marks unavailable ones, certify sends the shown hash".
// The API side is pinned by `api/tests/finance_reconciliation_test.rs`.
//
// Signatures, DTO fields and DOM hooks this test assumes (the implementer follows them):
// - The menu is `./menu.ts`'s `menu.Component(props: AdminMenuProps)`, id `reconciliation` (the
//   folder name, as `collectMenus` requires), mounted as the shell mounts it, beside the page's one
//   `Dialog` and `Toast`. It builds its own `new ApiClient()`, so the fake API is the global `fetch`.
//   AppShell's page-wide reads are answered and never counted as the menu's own. Which roles see
//   the menu is not pinned here (its field moves with OR-7.7 #1030); the API refuses other roles.
// - On show it reads `GET /api/v3/views/finance_reconciliation?branch=<picked branch>&date=<day>`,
//   `<day>` a `YYYY-MM-DD` business date. Which day the screen picks is the implementer's; this
//   test only checks that certify sends the day it read. The answer is `FinanceSheet` (ticket §7)
//   inside the v3 view envelope.
// - Each of the six parts is one element `[data-part="<name>"]`, `<name>` one of `transactions`,
//   `counts`, `transfers`, `exceptions`, `integrity`, `reversalsPending`. A part with
//   `available: false` carries the attribute `data-unavailable` and shows its `reason` text; an
//   available one carries no `data-unavailable`.
// - The transactions part shows each `byMethod` row's `method` and its money as the decimal strings
//   sent (not reformatted), and `reversedThb`. The exceptions part shows each row's `summary`.
// - Each certification is one `[data-certification="<id>"]`, with `data-stale="true"` when `stale`
//   and `data-stale="false"` otherwise.
// - Certify is `POST /api/v3/commands/reconciliation_certify` through `ApiClient.command`, with a
//   non-empty `idempotency-key` and the body `{ branch, date, figuresHash, note }`: the branch and
//   day of the sheet on screen, the `figuresHash` of the sheet on screen, and the note as typed. A
//   success reads the view again. A refusal shows the server's `message`.
// - DOM hooks: the note input is found by the `<label>` whose text starts with the i18n `note`;
//   the certify button is the button whose whole text is the i18n `certify`.
// - Keys of `./i18n.ts` (th and en; the wording is the implementer's): `note`, `certify`.
//
// The menu and its table are read through eager globs, so on a branch without them each test fails
// on an assertion that the file exists, not on the import.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}
interface TextsModule {
  th: Record<string, string>;
  en: Record<string, string>;
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });
const textFiles = import.meta.glob<TextsModule>('./i18n.ts', { eager: true });

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/reconciliation/menu.ts exists').toBeDefined();
  return m!.menu;
}

type K = 'note' | 'certify';

/** The screen's own text for `key`. */
function say(key: K, lang: 'th' | 'en' = 'th'): string {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/reconciliation/i18n.ts exists').toBeDefined();
  const value = m![lang][key];
  expect(value, `admin.reconciliation i18n ${lang}.${key}`).toBeTruthy();
  return value!;
}

const VIEW = '/api/v3/views/finance_reconciliation';
const CERTIFY = '/api/v3/commands/reconciliation_certify';
const B01 = 'XC-B01';
const B02 = 'XC-B02';
const DAY = /^\d{4}-\d{2}-\d{2}$/;
const PARTS = ['transactions', 'counts', 'transfers', 'exceptions', 'integrity', 'reversalsPending'];

// ── the fake server ──────────────────────────────────────────────────────────

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
  key: string | null;
  body: Record<string, any> | undefined;
}
interface Reply {
  status: number;
  body: unknown;
}

let sent: Sent[];
/** The hash the view answers per branch; a certify with any other hash is refused stale. */
let hashes: Record<string, string>;
let certified: Record<string, { id: string; figuresHash: string; note: string }[]>;

function waiting(reason: string) {
  return { available: false, reason, data: null };
}

function sheetOf(branch: string, day: string) {
  const hash = hashes[branch]!;
  return {
    branch,
    date: day,
    transactions: {
      available: true,
      reason: null,
      data: {
        byMethod: [
          { method: 'Cash', buyThb: '1300.00', sellThb: '2000.00', fees: '30.00', bankFees: '0.00', count: 3 },
          { method: 'PromptPay', buyThb: '0.00', sellThb: '500.00', fees: '5.00', bankFees: '5.00', count: 1 },
        ],
        reversedThb: '250.00',
      },
    },
    counts: waiting('CA-3'),
    transfers: waiting('CA-5'),
    exceptions: {
      available: true,
      reason: null,
      data: [{ id: 'EX-77', position: 1, reviewed: false, summary: 'EX-77 short count' }],
    },
    integrity: waiting('LG-7'),
    reversalsPending: waiting('RV-1'),
    figuresHash: hash,
    certifications: [...(certified[branch] ?? [])].reverse().map((c) => ({
      id: c.id,
      certifiedBy: 'u-fin',
      certifiedAt: '2026-10-11T10:00:00Z',
      note: c.note,
      figuresHash: c.figuresHash,
      stale: c.figuresHash !== hash,
    })),
  };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function refusal(status: number, code: string, message: string): Reply {
  return { status, body: { error: { code, message } } };
}

function answer(s: Sent): Reply {
  const { path, params, method, body } = s;
  if (path === '/api/v3/views/me') {
    return {
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: { kind: 'staff', id: 'u-fin', role: 'FINANCE', roles: ['FINANCE'], branch: B01, name: 'Fin', username: 'fin' },
      },
    };
  }
  if (path === '/api/v3/views/kiosk_notifications') return { status: 200, body: { version: 1, changed: false } };
  if (path === VIEW && method === 'GET') {
    const branch = params.get('branch') ?? '';
    const day = params.get('date') ?? '';
    if (!(branch in hashes)) return refusal(400, 'noSuchBranch', 'no such branch (test)');
    if (!DAY.test(day)) return refusal(400, 'invalidRequest', 'bad date (test)');
    return { status: 200, body: { version: 1, changed: true, data: sheetOf(branch, day) } };
  }
  if (path === CERTIFY && method === 'POST') {
    const branch = String(body?.branch ?? '');
    if (body?.figuresHash !== hashes[branch]) {
      return refusal(409, 'staleVersion', 'The figures changed; read the sheet again (test)');
    }
    const row = { id: `C-${(certified[branch]?.length ?? 0) + 1}`, figuresHash: String(body?.figuresHash), note: String(body?.note ?? '') };
    (certified[branch] ??= []).push(row);
    return {
      status: 200,
      body: { id: row.id, certifiedBy: 'u-fin', certifiedAt: '2026-10-11T10:00:00Z', note: row.note, figuresHash: row.figuresHash, stale: false },
    };
  }
  return refusal(500, 'serverError', `unexpected ${method} ${path}`);
}

function headerOf(init: RequestInit | undefined, name: string): string | null {
  const h = init?.headers;
  if (!h) return null;
  if (h instanceof Headers) return h.get(name);
  if (Array.isArray(h)) return h.find(([k]) => k.toLowerCase() === name)?.[1] ?? null;
  const rec = h as Record<string, string>;
  const k = Object.keys(rec).find((x) => x.toLowerCase() === name);
  return k === undefined ? null : rec[k]!;
}

async function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = typeof init?.body === 'string' && init.body !== '' ? JSON.parse(init.body) : undefined;
  const s: Sent = { path: url.pathname, params: url.searchParams, method, key: headerOf(init, 'idempotency-key'), body };
  sent.push(s);
  const reply = answer(s);
  return json(reply.status, reply.body);
}

function readsOf(branch: string): Sent[] {
  return sent.filter((s) => s.path === VIEW && s.method === 'GET' && s.params.get('branch') === branch);
}
function certifies(): Sent[] {
  return sent.filter((s) => s.path === CERTIFY && s.method === 'POST');
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

// ── mounting ─────────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

interface Mounted {
  el: HTMLElement;
  setBranch: (b: string) => void;
}

function mount(): Mounted {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-05', to: '2026-10-05' });
  const [shown] = createSignal(true);
  const Component = menu.Component;
  const el = document.createElement('div');
  root = el;
  document.body.appendChild(el);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    el,
  );
  return { el, setBranch };
}

beforeEach(() => {
  sent = [];
  hashes = { [B01]: 'hash-b01-aaaa', [B02]: 'hash-b02-bbbb' };
  certified = {};
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  toast('');
  location.hash = '#reconciliation';
  setLang('th');
});

afterEach(async () => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  root = undefined;
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
  await settle();
});

// ── helpers on the DOM ───────────────────────────────────────────────────────

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function button(el: ParentNode, name: string): HTMLButtonElement {
  const found = [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
  expect(found.length, `one button ${JSON.stringify(name)}`).toBe(1);
  expect(found[0]!.disabled, `the button ${JSON.stringify(name)} is enabled`).toBe(false);
  return found[0]!;
}

function field(el: ParentNode, label: string): HTMLInputElement | HTMLTextAreaElement {
  const found = [...el.querySelectorAll('label')].filter((l) => text(l).startsWith(label));
  expect(found.length, `one label starting ${JSON.stringify(label)}`).toBe(1);
  const control = (found[0] as HTMLLabelElement).control as HTMLInputElement | null;
  expect(control, `the control of ${label}`).not.toBeNull();
  return control!;
}

function part(el: ParentNode, name: string): HTMLElement {
  const found = el.querySelectorAll<HTMLElement>(`[data-part="${name}"]`);
  expect(found.length, `one [data-part="${name}"]`).toBe(1);
  return found[0]!;
}

function type(input: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

async function certify(el: ParentNode, lang: 'th' | 'en' = 'th'): Promise<void> {
  button(el, say('certify', lang)).click();
  await settle();
}

// ── the tests ────────────────────────────────────────────────────────────────

describe('F01 reconciliation screen (FN-1 #1068)', () => {
  it('reads the picked branch and a business date on show', async () => {
    mount();
    await settle();
    const reads = readsOf(B01);
    expect(reads.length, 'one read of B01 on show').toBeGreaterThanOrEqual(1);
    expect(reads.at(-1)!.params.get('date') ?? '', 'date=YYYY-MM-DD').toMatch(DAY);
  });

  it('shows each part and marks the unavailable ones with their reason', async () => {
    const { el } = mount();
    await settle();
    for (const name of PARTS) part(el, name);

    for (const [name, reason] of [
      ['counts', 'CA-3'],
      ['transfers', 'CA-5'],
      ['integrity', 'LG-7'],
      ['reversalsPending', 'RV-1'],
    ] as const) {
      const p = part(el, name);
      expect(p.hasAttribute('data-unavailable'), `${name} is marked unavailable`).toBe(true);
      expect(text(p), `${name} shows what it waits for`).toContain(reason);
    }
    for (const name of ['transactions', 'exceptions']) {
      expect(part(el, name).hasAttribute('data-unavailable'), `${name} is available`).toBe(false);
    }

    const tx = text(part(el, 'transactions'));
    for (const shown of ['Cash', 'PromptPay', '1300.00', '2000.00', '30.00', '500.00', '250.00']) {
      expect(tx, `transactions shows ${shown}`).toContain(shown);
    }
    expect(text(part(el, 'exceptions'))).toContain('EX-77 short count');
  });

  it('certify sends the branch, day and hash of the sheet on screen, with the note and a key', async () => {
    const { el } = mount();
    await settle();
    const day = readsOf(B01).at(-1)!.params.get('date');
    type(field(el, say('note')), 'all checked');
    await settle();
    await certify(el);

    const sent = certifies();
    expect(sent.length, 'one certify').toBe(1);
    expect(sent[0]!.body).toEqual({ branch: B01, date: day, figuresHash: 'hash-b01-aaaa', note: 'all checked' });
    expect(sent[0]!.key, 'an idempotency key').toBeTruthy();
    // a success reads the sheet again, which lists the new certification, not stale
    const row = el.querySelectorAll<HTMLElement>('[data-certification="C-1"]');
    expect(row.length, 'the new certification is listed').toBe(1);
    expect(row[0]!.dataset.stale).toBe('false');
  });

  it('after the branch changes, certify sends the new sheet’s branch and hash', async () => {
    const { el, setBranch } = mount();
    await settle();
    setBranch(B02);
    await settle();
    expect(readsOf(B02).length, 'B02 is read').toBeGreaterThanOrEqual(1);
    await certify(el);
    const sent = certifies();
    expect(sent.length).toBe(1);
    expect(sent[0]!.body?.branch).toBe(B02);
    expect(sent[0]!.body?.figuresHash).toBe('hash-b02-bbbb');
  });

  it('a stale refusal shows the server’s message; a stale certification is marked', async () => {
    certified[B01] = [{ id: 'C-OLD', figuresHash: 'hash-before', note: '' }];
    const { el } = mount();
    await settle();
    const old = el.querySelectorAll<HTMLElement>('[data-certification="C-OLD"]');
    expect(old.length, 'the earlier certification is listed').toBe(1);
    expect(old[0]!.dataset.stale).toBe('true');

    hashes[B01] = 'hash-b01-moved'; // the figures moved after the screen read them
    await certify(el);
    expect(certifies().length).toBe(1);
    expect(document.body.textContent ?? '').toContain('The figures changed; read the sheet again (test)');
  });
});
