// RE-5 (#602): the A06 approvals menu — the queue of pending requests, approve or reject with a
// reason.
// Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/blueprint/rates.md` §5.1–§5.4 (one approval core for every type; the
// proposer may not decide; a decision has an outcome and a reason), §4.6 (an out-of-bound draft by
// RATES waits for OWNER), §4.7 (`GET /api/v3/views/approval_requests` with filters `status`, `type`,
// OWNER; `POST /api/approvals/:id/decide` → APPROVED or REJECTED with a reason, A06), §9 RE-5 (the
// A06 queue approves and rejects with a reason); Blueprint §03 A06 ("อนุมัติ/ปฏิเสธพร้อมเหตุผล");
// `xchange-v3-web.md` W7, W10. Wire shapes are main's at `f04fd32`:
// `api/src/presentation/http/v3/views/approval_requests.rs` (branch-scoped, newest first),
// `api/src/presentation/http/v3/commands/approval_decide.rs` (`{ outcome, reason }`), and
// `web/src/shared/api/generated/approval_requests.ts`; the rate publication's `proposedPayload` is
// the evaluation `rate_draft_service.rs` `submit` stores (`draft_id`, `policy_version_id`,
// `reference_line_ids`, `is_in_bound`, `out_of_bound: [{ "currency": "<CODE>", "side": "BUY" | "SELL" }]`).
// Re-pinned in a later commit by the lane controller's ruling 2026-10-09 on #602: the stored
// `out_of_bound` was `["USD:Sell"]`, the Debug text of `Side` (`rate_draft_service.rs:490` at
// `f04fd32`); Debug text is not a wire contract, and it now has simulate's structured shape.
//
// Signatures, props, DTO fields and DOM hooks this test assumes (the implementer follows them):
// - `web/src/screens/admin/approvals/menu.ts` exports `menu: AdminMenu` with id `approvals`,
//   `roles: ['OWNER']` (the view's and the command's policy rows are OWNER only, `policy.rs:72-73`),
//   `dated: false` (the queue has no period), reached through `menus.ts`'s glob; its texts are in
//   `./i18n.ts` (th and en, namespace `admin.approvals.*`).
// - The component builds its own `new ApiClient()`; the fake API is the global `fetch`.
// - On show it reads `approval_requests` with `branch` = the picked branch and `status=PENDING`.
// - Each request is an element `[data-request="<id>"]` holding its proposer (`requestedBy`), the
//   currencies its proposal lists out of bound, each with its side through the table (`sideBUY`,
//   `sideSELL`; never the raw `BUY`/`SELL`), a reason input (the `<label>` whose text starts with
//   the table's `decisionReason`), and the buttons `approve` and `reject` (found by whole text).
// - A decision is `ApiClient.command('POST', '/api/approvals/<id>/decide', { outcome, reason,
//   branch }, key)` with a non-empty key of its own; after it the queue is read again.
// - A reject with an empty reason sends nothing and shows the table's `reasonRequired`.
// - A refused decision shows the server's `message`.
// - Keys of `./i18n.ts` this test reads: `label`, `approve`, `reject`, `decisionReason`,
//   `reasonRequired`, `sideBUY`, `sideSELL`.
//
// The menu and its table are read through eager globs, so on a branch without them each test fails
// on an assertion that the file exists, not on the import.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the menu record's `roles: ['OWNER']` pin → `needs: 'ApprovalsDecide'` (#1030 §3.10: the menu follows
// the permission its view and command need, `policy.rs:567-569`; the intended change §9 names).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { screenLookup, setLang } from '../../../shared/i18n';
import {
  makeApprovalRequestItemDto,
  type ApprovalRequestItemDto,
} from '../../../shared/testing/fixtures';
import { menus, type AdminMenu, type AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { PAGE_WIDE, QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}
interface TextsModule {
  th: Record<string, string>;
  en: Record<string, string>;
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });
const textFiles = import.meta.glob<TextsModule>('./i18n.ts', { eager: true });
const sources = import.meta.glob<string>('./*.{ts,tsx}', { query: '?raw', import: 'default', eager: true });

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/approvals/menu.ts exists').toBeDefined();
  return m!.menu;
}

function texts(): TextsModule {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/approvals/i18n.ts exists').toBeDefined();
  return m!;
}

const KEYS = ['label', 'approve', 'reject', 'decisionReason', 'reasonRequired', 'sideBUY', 'sideSELL'] as const;
type K = (typeof KEYS)[number];

function say(key: K, lang: 'th' | 'en' = 'th'): string {
  const value = texts()[lang][key];
  expect(value, `admin.approvals i18n ${lang}.${key}`).toBeTruthy();
  return value!;
}

const REQUESTS = '/api/v3/views/approval_requests';
const THAI = /[฀-๿]/;

// ── the answers ──────────────────────────────────────────────────────────────

interface OutOfBoundSide {
  currency: string;
  side: 'BUY' | 'SELL';
}

function request(id: string, by: string, outOfBound: OutOfBoundSide[], branch = 'MAIN') {
  return makeApprovalRequestItemDto({
    id,
    type: 'RATE_PUBLICATION',
    branchId: branch,
    objectId: `draft-of-${id}`,
    objectVersionHash: `hash-${id}`,
    policyVersion: 'pol-1',
    proposedPayload: {
      draft_id: `draft-of-${id}`,
      policy_version_id: 'pol-1',
      reference_line_ids: ['line-1'],
      is_in_bound: false,
      out_of_bound: outOfBound,
    },
    requestedBy: by,
    requestedAt: '2026-10-09T03:00:00+00:00',
    status: 'PENDING',
    decision: null,
  });
}

// ── the fake API ─────────────────────────────────────────────────────────────

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
  key: string | null;
  body: Record<string, unknown> | undefined;
}
type Reply = { status: number; body: unknown };

let sent: Sent[];
let queue: Record<string, ApprovalRequestItemDto[]>;
let decideRefusal: { status: number; code: string; message: string } | null;
let queueVersion: number;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function answer(s: Sent): Promise<Reply> {
  const { path, params: p, method, body } = s;
  if (path === '/api/v3/views/me') {
    return {
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: { kind: 'owner', id: 'owner-1', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' },
      },
    };
  }
  if (path === QUEUE_CARD_PATH) return { status: 200, body: queueCardUnchanged() };
  if (path === REQUESTS && method === 'GET') {
    const branch = p.get('branch') ?? 'MAIN';
    const status = p.get('status');
    const rows = (queue[branch] ?? []).filter((r) => status === null || r.status === status);
    const since = p.get('since');
    if (since !== null && Number(since) === queueVersion) {
      return { status: 200, body: { version: queueVersion, changed: false } };
    }
    return { status: 200, body: { version: queueVersion, changed: true, data: { requests: structuredClone(rows) } } };
  }
  const m = /^\/api\/approvals\/([^/]+)\/decide$/.exec(path);
  if (m && method === 'POST') {
    if (decideRefusal) {
      return { status: decideRefusal.status, body: { error: { code: decideRefusal.code, message: decideRefusal.message } } };
    }
    const outcome = String(body?.outcome);
    for (const rows of Object.values(queue)) {
      for (const r of rows) if (r.id === m[1]) r.status = outcome;
    }
    queueVersion += 1;
    return { status: 200, body: { status: outcome } };
  }
  return { status: 500, body: { error: { code: 'serverError', message: `unexpected ${method} ${path}` } } };
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
  const reply = await answer(s);
  return json(reply.status, reply.body);
}

function own(): Sent[] {
  return sent.filter((s) => !PAGE_WIDE.includes(s.path));
}
function reads(): Sent[] {
  return sent.filter((s) => s.path === REQUESTS && s.method === 'GET');
}
function writes(): Sent[] {
  return sent.filter((s) => s.method !== 'GET');
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

function mount(init: { branch?: string } = {}): { el: HTMLElement; setBranch: (b: string) => void } {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(init.branch ?? 'MAIN');
  const [period] = createSignal({ from: '2026-10-09', to: '2026-10-09' });
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
  queueVersion = 1;
  queue = {
    MAIN: [
      request('r1', 'employee-RATES', [{ currency: 'USD', side: 'SELL' }]),
      request('r2', 'rates-2', [{ currency: 'GBP', side: 'BUY' }]),
    ],
    B9: [request('r9', 'rates-b9', [{ currency: 'EUR', side: 'SELL' }], 'B9')],
  };
  decideRefusal = null;
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  toast('');
  location.hash = '#approvals';
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

function card(el: ParentNode, id: string): HTMLElement {
  const found = el.querySelectorAll<HTMLElement>(`[data-request="${id}"]`);
  expect(found.length, `one request [data-request="${id}"]`).toBe(1);
  return found[0]!;
}

function hasCard(el: ParentNode, id: string): boolean {
  return el.querySelectorAll(`[data-request="${id}"]`).length > 0;
}

function button(el: ParentNode, name: string): HTMLButtonElement {
  const found = [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
  expect(found.length, `one button ${JSON.stringify(name)}`).toBe(1);
  return found[0]!;
}

function field(el: ParentNode, label: string): HTMLInputElement | HTMLTextAreaElement {
  const found = [...el.querySelectorAll('label')].filter((l) => text(l).startsWith(label));
  expect(found.length, `one label starting ${JSON.stringify(label)}`).toBe(1);
  const control = (found[0] as HTMLLabelElement).control as HTMLInputElement | HTMLTextAreaElement | null;
  expect(control, `the control of ${label}`).not.toBeNull();
  return control!;
}

function type(input: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

async function decide(el: HTMLElement, id: string, reason: string, outcome: 'approve' | 'reject'): Promise<void> {
  const c = card(el, id);
  type(field(c, say('decisionReason')), reason);
  button(c, say(outcome)).click();
  await settle();
}

function decisions(): Sent[] {
  return writes().filter((s) => /^\/api\/approvals\/[^/]+\/decide$/.test(s.path));
}

// ── the menu record ──────────────────────────────────────────────────────────

describe('RE-5 A06: the approvals menu record (#602; design §4.7, §5)', () => {
  it('is the approvals menu: needs ApprovalsDecide, not dated, reached through the shell glob once', () => {
    const menu = theMenu();
    expect(menu.id).toBe('approvals');
    expect(menu.needs, 'approvalRead and approvalDecide need ApprovalsDecide (policy.rs:567-569)').toBe('ApprovalsDecide');
    expect(menu.dated).toBe(false);
    expect(typeof menu.Component).toBe('function');
    const listed = menus.filter((m) => m.id === 'approvals');
    expect(listed.length).toBe(1);
    expect(listed[0]).toBe(menu);
  });

  it('its label and heading follow setLang; the en table has every th key and no Thai', () => {
    const menu = theMenu();
    const { th, en } = texts();
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const k of KEYS) say(k);
    for (const [k, v] of Object.entries(en)) expect(THAI.test(v), `en.${k}`).toBe(false);
    try {
      setLang('th');
      expect(menu.label()).toBe(th.label);
      setLang('en');
      expect(menu.label()).toBe(en.label);
      expect(THAI.test(menu.heading.title())).toBe(false);
    } finally {
      setLang('th');
    }
    for (const k of Object.keys(th)) {
      expect(screenLookup.th[`admin.approvals.${k}`], `admin.approvals.${k}`).toBe(th[k]);
    }
  });
});

// ── the queue ────────────────────────────────────────────────────────────────

describe('RE-5 A06: the queue of pending requests (design §4.7)', () => {
  it('reads approval_requests for the picked branch with status=PENDING and writes nothing', async () => {
    mount();
    await settle();
    expect(reads().length).toBeGreaterThanOrEqual(1);
    expect(reads()[0]!.params.get('branch')).toBe('MAIN');
    expect(reads()[0]!.params.get('status')).toBe('PENDING');
    expect(own().every((s) => s.path === REQUESTS), JSON.stringify(own().map((s) => s.path))).toBe(true);
    expect(writes()).toEqual([]);
  });

  it('shows each request with its proposer and the currencies it proposes out of bound, each with its side', async () => {
    const m = mount();
    await settle();
    const r1 = text(card(m.el, 'r1'));
    expect(r1).toContain('employee-RATES');
    expect(r1).toContain('USD');
    expect(r1, 'the USD sell side, through the table').toContain(say('sideSELL'));
    expect(r1).not.toContain(say('sideBUY'));
    const r2 = text(card(m.el, 'r2'));
    expect(r2).toContain('rates-2');
    expect(r2).toContain('GBP');
    expect(r2, 'the GBP buy side, through the table').toContain(say('sideBUY'));
    expect(r2).not.toContain(say('sideSELL'));
    expect(hasCard(m.el, 'r9'), "another branch's request is not listed").toBe(false);
  });

  it('a branch change reads that branch', async () => {
    const m = mount();
    await settle();
    m.setBranch('B9');
    await settle();
    expect(reads().at(-1)!.params.get('branch')).toBe('B9');
    expect(hasCard(m.el, 'r9')).toBe(true);
    expect(hasCard(m.el, 'r1')).toBe(false);
  });
});

// ── deciding ─────────────────────────────────────────────────────────────────

describe('RE-5 A06: approve and reject with a reason (design §5; Blueprint §03 A06)', () => {
  it('approve sends APPROVED with the typed reason for the picked branch, then the request leaves the queue', async () => {
    const m = mount();
    await settle();
    const before = reads().length;
    await decide(m.el, 'r1', 'checked against BoT', 'approve');
    const d = decisions();
    expect(d.map((s) => `${s.method} ${s.path}`)).toEqual(['POST /api/approvals/r1/decide']);
    expect(d[0]!.body?.outcome).toBe('APPROVED');
    expect(d[0]!.body?.reason).toBe('checked against BoT');
    expect(d[0]!.body?.branch).toBe('MAIN');
    expect(d[0]!.key, 'an Idempotency-Key').toBeTruthy();
    expect(reads().length, 'the queue is read again').toBeGreaterThan(before);
    expect(hasCard(m.el, 'r1')).toBe(false);
    expect(hasCard(m.el, 'r2')).toBe(true);
  });

  it('reject sends REJECTED with the typed reason, then the request leaves the queue', async () => {
    const m = mount();
    await settle();
    await decide(m.el, 'r2', 'too far from BoT', 'reject');
    const d = decisions();
    expect(d.map((s) => s.path)).toEqual(['/api/approvals/r2/decide']);
    expect(d[0]!.body?.outcome).toBe('REJECTED');
    expect(d[0]!.body?.reason).toBe('too far from BoT');
    expect(hasCard(m.el, 'r2')).toBe(false);
    expect(hasCard(m.el, 'r1')).toBe(true);
  });

  it('two decisions carry keys of their own', async () => {
    const m = mount();
    await settle();
    await decide(m.el, 'r1', 'ok', 'approve');
    await decide(m.el, 'r2', 'no', 'reject');
    const keys = decisions().map((s) => s.key);
    expect(keys.length).toBe(2);
    expect(keys[0]).not.toBe(keys[1]);
  });

  it('a reject with no reason sends nothing and says a reason is needed', async () => {
    const m = mount();
    await settle();
    await decide(m.el, 'r1', '   ', 'reject');
    expect(decisions()).toEqual([]);
    expect(text(document.body)).toContain(say('reasonRequired'));
    expect(hasCard(m.el, 'r1')).toBe(true);
  });

  it('a refused decision shows the server message and keeps the request', async () => {
    decideRefusal = { status: 403, code: 'outOfScope', message: 'The proposer may not decide this request (test)' };
    const m = mount();
    await settle();
    await decide(m.el, 'r1', 'looks fine', 'approve');
    expect(decisions().length).toBe(1);
    expect(text(document.body)).toContain('The proposer may not decide this request (test)');
    expect(hasCard(m.el, 'r1')).toBe(true);
  });
});

// ── languages and the source ─────────────────────────────────────────────────

describe('RE-5 A06: Thai and English through the table; no direct fetch (W7, W10)', () => {
  it('in English: no Thai on the screen or in its attributes', async () => {
    setLang('en');
    try {
      const m = mount();
      await settle();
      expect(hasCard(m.el, 'r1')).toBe(true);
      expect(text(m.el)).not.toMatch(THAI);
      const attrs = [...m.el.querySelectorAll('[aria-label], [title], [placeholder]')].map(
        (e) => `${e.getAttribute('aria-label') ?? ''} ${e.getAttribute('title') ?? ''} ${e.getAttribute('placeholder') ?? ''}`,
      );
      expect(attrs.filter((a) => THAI.test(a))).toEqual([]);
    } finally {
      setLang('th');
    }
  });

  it('in Thai: the type and the status are shown through the table, never as their codes', async () => {
    const m = mount();
    await settle();
    const r1 = text(card(m.el, 'r1'));
    expect(r1).not.toContain('RATE_PUBLICATION');
    expect(r1).not.toContain('PENDING');
    expect(r1, 'the side through the table, not its code').not.toMatch(/\b(BUY|SELL)\b/);
    expect(r1, 'never the Debug text of a side').not.toMatch(/\b(Buy|Sell)\b/);
  });

  it('builds its own ApiClient, calls no fetch and parses no figure', () => {
    const own = Object.entries(sources).filter(([path]) => !path.includes('.test.'));
    expect(own.map(([p]) => p)).toContain('./menu.ts');
    const all = own.map(([, t]) => t).join('\n');
    expect(all).toMatch(/new ApiClient\(/);
    const forbidden = /Number\(|parseFloat|parseInt|Math\.round|toFixed|fetch\(/;
    for (const [path, t] of own) {
      t.split('\n').forEach((line, i) => {
        expect(forbidden.test(line), `${path}:${i + 1} ${line.trim()}`).toBe(false);
      });
    }
  });
});
