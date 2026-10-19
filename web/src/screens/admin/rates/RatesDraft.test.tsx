// RE-5 (#602): the rates menu rebuilt on drafts and publications — A02 (draft → simulate → submit →
// publish, "adjust all", the out-of-bound mark, the BoT reference line per unit) and A03 (the
// history reads publications).
// Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/blueprint/rates.md`
// - §3.3: a rate is the 8-place string the server holds and every screen shows it through the one
//   `formatRate` (`web/src/shared/ui/rate.ts`), with `/ <unit> <code>` when the unit is > 1;
// - §4.2: the reference is read on demand, when a draft asks for one; BoT JPY is per 100 in BoT's
//   own unit; buy = BoT buying transfer, sell = BoT selling (A6);
// - §4.4: "adjust all" changes the percent only, MANUAL sides keep their values;
// - §4.5, §4.6: evaluation (in/out of bound), states, publish with `expected_version`;
// - §4.7: the routes (create/edit draft, simulate, submit, publish, policy save, fetch reference)
//   and their error codes (`staleVersion`, `approvalRequired`);
// - D-3: publications replace `rates`; `adminSaveRate` is retired and A03 reads publications;
// - §9 RE-5/RE-6; `xchange-v3-web.md` W7 (the screen computes no figure), W10 (th and en through
//   the screen's own i18n table).
// Wire shapes are main's at `f04fd32`: `api/src/presentation/http/v3/commands/rate_*.rs`, the
// views `published_rates`, `rate_drafts`, `rate_draft`, `rate_policy`, `rate_publications`,
// `rates_reference`, and `web/src/shared/api/generated/{published_rates,rate_draft,rate_drafts,
// rate_policy,rate_publications,rates_reference}.ts`.
//
// Signatures, props, DTO fields and DOM hooks this test assumes (the implementer follows them):
// - The menu is `./menu.ts`'s `menu.Component(props: AdminMenuProps)`, mounted as the shell mounts
//   it, beside the page's one `Dialog` and `Toast`. It builds its own `new ApiClient()`, so the fake
//   API is the global `fetch`. AppShell's page-wide reads (`PAGE_WIDE`,
//   `shared/testing/page-wide.ts`) are answered and never counted as the menu's own.
// - Writes go through `ApiClient.command(method, path, body, key)`: each carries a non-empty
//   `idempotency-key` header, a key of its own, and `branch` = the picked branch in the body.
// - On show it reads `published_rates`, `rate_drafts`, `rate_policy` and `rates_reference` with
//   `branch`, and `rate_draft` with `branch` and `id` = the open draft: the first of
//   `rate_drafts.drafts` (the view answers newest first) whose status is `DRAFT`,
//   `PENDING_APPROVAL` or `APPROVED`. A `REJECTED` or `PUBLISHED` draft is never reopened (§4.6).
// - "New draft" asks `POST /api/rate-references/fetch` first (§4.2: the reference is read when a
//   draft asks for one), then `POST /api/rate-drafts` with one line per currency of the current
//   publication (a publication replaces the branch's whole set, so a draft that left one out would
//   drop it), then reads `rate_draft` of the new id.
// - "Adjust all" sends the two percents as typed to `POST /api/rate-policies` (the only route that
//   holds the percents, §4.3), carrying the current `rate_policy`'s guards and stale settings
//   unchanged, then `PUT /api/rate-drafts/{id}` with `expectedVersion` = the draft's version and the
//   draft's lines, MANUAL sides as they were. It is there, and does the same, for OWNER and for
//   RATES (lane controller's ruling 2026-10-09 on #602: RATE_MANAGER may save a policy version
//   changing only buy/sell %, per Blueprint RATE_MANAGER "ปรับ %"; the server refuses a RATES save
//   that moves the frame, so the screen sends the frame unchanged).
// - The page's signed-in caller is AppShell's `/api/v3/views/me`; the fake answers OWNER or a RATES
//   member of MAIN.
// - Simulate answers `{ id, isInBound, outOfBound: [{ currency: "<CODE>", side: "BUY" | "SELL" }, …] }`;
//   the draft line of each listed currency is marked. (Re-pinned in a later commit by the lane
//   controller's ruling 2026-10-09 on #602: the first version pinned `"USD:Sell"`, the Debug text of
//   `Side` that `rate_draft_simulate.rs:81` answered at `f04fd32`; Debug text is not a wire contract.)
// - Publish sends `expectedVersion` = `published_rates.publication.version` (null before the first
//   publication); after a publish it reads `published_rates` again.
// - A refused command shows `errorText(code, message)` (the shared `error.staleVersion` key, or
//   the server's `message` when no shared key exists such as `error.approvalRequired`).
// - DOM hooks: a draft line is an element `[data-draft-line="<CODE>"]`; a line of the current
//   publication is `[data-published-line="<CODE>"]`; a button is found by its whole text; an input
//   by the `<label>` whose text starts with its i18n text.
// - Keys of `./i18n.ts` (th and en; the wording is the implementer's): `newDraft`, `simulate`,
//   `submit`, `publish`, `adjustAll`, `adjustBuyPct`, `adjustSellPct`, `outOfBound`, `inBound`,
//   and the kept `rateHistoryButton`.
//
// The menu and its table are read through eager globs, so on a branch without them each test fails
// on an assertion that the file exists, not on the import.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { PublishedLineDto, PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { RateDraftDetailData, RateDraftLineDto } from '../../../shared/api/generated/rate_draft';
import type { RateDraftItemDto } from '../../../shared/api/generated/rate_drafts';
import type { RatePolicyData } from '../../../shared/api/generated/rate_policy';
import type { RatePublicationRow } from './rate-history';
import type { RatesReferenceData, RatesReferenceRow } from '../../../shared/api/generated/rates_reference';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { PAGE_WIDE, QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

// A03 (#3399): the publications type is the local row of ./rate-history (generated file changes with #3399).
type RatePublicationItemDto = Omit<RatePublicationRow, 'lines'>;

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
  expect(m, 'web/src/screens/admin/rates/menu.ts exists').toBeDefined();
  return m!.menu;
}

const KEYS = [
  'newDraft',
  'simulate',
  'submit',
  'publish',
  'adjustAll',
  'adjustBuyPct',
  'adjustSellPct',
  'outOfBound',
  'inBound',
  'rateHistoryButton',
] as const;
type K = (typeof KEYS)[number];

/** The screen's own text for `key` in the current test's language. */
function say(key: K, lang: 'th' | 'en' = 'th'): string {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/rates/i18n.ts exists').toBeDefined();
  const value = m![lang][key];
  expect(value, `admin.rates i18n ${lang}.${key}`).toBeTruthy();
  return value!;
}

const V = '/api/v3/views/';
const PUBLISHED = `${V}published_rates`;
const DRAFTS = `${V}rate_drafts`;
const DRAFT = `${V}rate_draft`;
const POLICY = `${V}rate_policy`;
const PUBLICATIONS = `${V}rate_publications`;
const REFERENCE = `${V}rates_reference`;
const OLD_READS = [`${V}admin_rates`, `${V}admin_rate_history`];
const RPC = '/api/rpc';

const THAI = /[฀-๿]/;

// ── the answers ──────────────────────────────────────────────────────────────

function line(currency: string, unitSize: string, buy: string | null, sell: string | null): PublishedLineDto {
  return { currency, group: 'ALL', unitSize, buyRate: buy, sellRate: sell, lineId: `line-${currency}` };
}

function publishedOf(version: number | null, lines: PublishedLineDto[]): PublishedRatesData {
  return {
    publication:
      version === null ? null : { id: `pub-${version}`, version, publishedAt: '2026-10-09T02:00:00+00:00' },
    sourceDate: '2026-10-08',
    stale: false,
    lines,
  };
}

/** MAIN's publication 7: USD per 1, JPY per 100 (§3.1 A1), EUR per 1. */
function mainPublished(): PublishedRatesData {
  return publishedOf(7, [
    line('EUR', '1', '37.35520000', '38.08440000'),
    line('JPY', '100', '23.50000000', '24.12345678'),
    line('USD', '1', '33.25000000', '34.10000000'),
  ]);
}

function draftItem(id: string, status: string, version: number, createdAt: string): RateDraftItemDto {
  return {
    id,
    branchId: 'MAIN',
    status,
    version,
    createdBy: 'rates-9',
    createdAt,
    updatedAt: createdAt,
    evaluation: null,
  };
}

function auto(currency: string): RateDraftLineDto {
  return { currency, buyMode: 'AUTO', sellMode: 'AUTO', manualBuy: null, manualSell: null, reason: null };
}

/** USD: buy AUTO, sell MANUAL 40 with its reason (the out-of-bound line of `rate_draft_flow_test.rs`). */
function usdManualSell(): RateDraftLineDto {
  return {
    currency: 'USD',
    buyMode: 'AUTO',
    sellMode: 'MANUAL',
    manualBuy: null,
    manualSell: '40.00000000',
    reason: 'a spike',
  };
}

function detailOf(item: RateDraftItemDto, lines: RateDraftLineDto[]): RateDraftDetailData {
  return { draft: { ...item }, lines };
}

const GUARDS = {
  USD: { buy: { min_pct: '-1.0000', max_pct: '1.0000' }, sell: { min_pct: '-1.0000', max_pct: '1.0000' } },
  JPY: { buy: { min_pct: '-2.0000', max_pct: '0.5000' }, sell: { min_pct: '-0.5000', max_pct: '2.0000' } },
};

function policyOf(): RatePolicyData {
  return {
    policy: {
      id: 'pol-1',
      branchId: 'MAIN',
      version: 1,
      buyAdjustPct: '0.0000',
      sellAdjustPct: '0.0000',
      guards: structuredClone(GUARDS),
      maxReferenceAgeHours: 24,
      onStale: 'STOP',
      staleLimitThb: null,
      validFrom: '2026-10-08T00:00:00+00:00',
      validTo: null,
      approvedBy: 'owner-1',
      reason: 'seeded',
      valueTiersOn: false,
      valueTiers: [],
      sharpMoveAlertPct: null,
    },
  };
}

function refRow(code: string, unit: number, sight: string, transfer: string, selling: string): RatesReferenceRow {
  return { code, date: '2026-10-08', unit, buyingSight: sight, buyingTransfer: transfer, selling };
}

/** After RE-2 #599: BoT JPY per 100, in BoT's own unit (§4.2). */
function referenceOf(): RatesReferenceData {
  return {
    rows: [refRow('USD', 1, '33.4218', '33.5008', '33.8278'), refRow('JPY', 100, '20.9500', '21.0100', '21.6300')],
    fetchedAt: '2026-10-09T01:30:00.000Z',
    error: null,
  };
}

function publication(version: number, by: string, state: string): RatePublicationItemDto {
  return {
    id: `pub-${version}`,
    branchId: 'MAIN',
    version,
    publishedAt: `2026-10-0${version % 9}T02:00:00+00:00`,
    publishedBy: by,
    approvedRequestId: `req-${version}`,
    draftId: `d-${version}`,
    state,
  };
}

// ── the fake API: the global fetch ──────────────────────────────────────────

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
  key: string | null;
  body: Record<string, unknown> | undefined;
}

type Reply = { status: number; body: unknown };

let sent: Sent[];
let published: Record<string, { version: number; data: PublishedRatesData }>;
let drafts: Record<string, RateDraftItemDto[]>;
let details: Record<string, RateDraftDetailData>;
let policies: Record<string, RatePolicyData>;
let publications: Record<string, RatePublicationItemDto[]>;
let reference: RatesReferenceData;
let simulateReply: (id: string) => Reply;
let submitStatus: string;
let publishRefusal: { status: number; code: string; message: string } | null;
let newDraftId: number;
let meRole: 'OWNER' | 'RATES';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function viewAnswer<T>(version: number, data: T, since: string | null): Reply {
  if (since !== null && Number(since) === version) return { status: 200, body: { version, changed: false } };
  return { status: 200, body: { version, changed: true, data: structuredClone(data) } };
}

function refusal(status: number, code: string, message: string): Reply {
  return { status, body: { error: { code, message } } };
}

function setDraftStatus(branch: string, id: string, status: string): void {
  for (const d of drafts[branch] ?? []) if (d.id === id) d.status = status;
  const detail = details[id];
  if (detail?.draft) detail.draft.status = status;
}

function linesOfBody(body: Record<string, unknown> | undefined): RateDraftLineDto[] {
  const lines = (body?.lines ?? []) as Array<Record<string, unknown>>;
  return lines.map((l) => ({
    currency: String(l.currency),
    buyMode: String(l.buyMode),
    sellMode: String(l.sellMode),
    manualBuy: (l.manualBuy as string | null | undefined) ?? null,
    manualSell: (l.manualSell as string | null | undefined) ?? null,
    reason: (l.reason as string | null | undefined) ?? null,
  }));
}

async function answer(s: Sent): Promise<Reply> {
  const { path, params: p, method, body } = s;
  const branch = (method === 'GET' ? p.get('branch') : (body?.branch as string | undefined)) ?? 'MAIN';
  const since = p.get('since');
  if (path === '/api/v3/views/me') {
    return {
      status: 200,
      body: {
        version: 1,
        changed: true,
        data:
          meRole === 'OWNER'
            ? { kind: 'owner', id: 'owner-1', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' }
            : { kind: 'staff', id: 'employee-RATES', role: 'RATES', branch: 'MAIN', name: 'Rates', username: 'rates' },
      },
    };
  }
  if (path === QUEUE_CARD_PATH) return { status: 200, body: queueCardUnchanged() };
  if (method === 'GET') {
    if (path === PUBLISHED) {
      const e = published[branch];
      return e ? viewAnswer(e.version, e.data, since) : viewAnswer(1, publishedOf(null, []), since);
    }
    if (path === DRAFTS) return viewAnswer(100 + JSON.stringify(drafts[branch] ?? []).length, { drafts: drafts[branch] ?? [] }, since);
    if (path === DRAFT) {
      const d = details[p.get('id') ?? ''];
      const data = d ?? { draft: null, lines: [] };
      return viewAnswer(200 + JSON.stringify(data).length, data, since);
    }
    if (path === POLICY) return viewAnswer(1, policies[branch] ?? { policy: null }, since);
    if (path === PUBLICATIONS) return viewAnswer(1, (publications[branch] ?? []).map((p) => ({ ...p, lines: [] })), since);
    if (path === REFERENCE) return viewAnswer(3, reference, since);
    return refusal(500, 'serverError', `unexpected read ${path}`);
  }
  if (path === '/api/rate-references/fetch' && method === 'POST') return { status: 200, body: { batchId: 'batch-1' } };
  if (path === '/api/rate-policies' && method === 'POST') return { status: 200, body: { id: 'pol-2', version: 2 } };
  if (path === '/api/rate-drafts' && method === 'POST') {
    const id = `d-new-${newDraftId++}`;
    const item = draftItem(id, 'DRAFT', 1, '2026-10-09T05:00:00+00:00');
    item.branchId = branch;
    drafts[branch] = [item, ...(drafts[branch] ?? [])];
    details[id] = detailOf(item, linesOfBody(body));
    return { status: 200, body: { id, version: 1, status: 'DRAFT' } };
  }
  const m = /^\/api\/rate-drafts\/([^/]+)(?:\/(simulate|submit|publish))?$/.exec(path);
  if (m) {
    const id = m[1]!;
    const step = m[2];
    if (!step && method === 'PUT') {
      const d = details[id];
      if (!d?.draft) return refusal(404, 'notFound', 'no such draft (test)');
      if (body?.expectedVersion !== d.draft.version) return refusal(409, 'staleVersion', 'draft moved (test)');
      d.draft.version += 1;
      d.lines = linesOfBody(body);
      for (const it of drafts[branch] ?? []) if (it.id === id) it.version = d.draft.version;
      return { status: 200, body: { id, version: d.draft.version, status: 'DRAFT' } };
    }
    if (step === 'simulate' && method === 'POST') return simulateReply(id);
    if (step === 'submit' && method === 'POST') {
      setDraftStatus(branch, id, submitStatus);
      return { status: 200, body: { requestId: 'req-1', status: submitStatus } };
    }
    if (step === 'publish' && method === 'POST') {
      if (publishRefusal) return refusal(publishRefusal.status, publishRefusal.code, publishRefusal.message);
      const cur = published[branch];
      const next = (cur?.data.publication?.version ?? 0) + 1;
      published[branch] = {
        version: (cur?.version ?? 1) + 10,
        data: { ...(cur?.data ?? publishedOf(null, [])), publication: { id: `pub-${next}`, version: next, publishedAt: '2026-10-09T05:10:00+00:00' } },
      };
      setDraftStatus(branch, id, 'PUBLISHED');
      return { status: 200, body: { id: `pub-${next}`, version: next } };
    }
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
  const reply = await answer(s);
  return json(reply.status, reply.body);
}

function readsOf(path: string): Sent[] {
  return sent.filter((s) => s.path === path && s.method === 'GET');
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

interface Mounted {
  el: HTMLElement;
  setBranch: (b: string) => void;
}

function mount(init: { branch?: string } = {}): Mounted {
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
  newDraftId = 1;
  meRole = 'OWNER';
  const d1 = draftItem('d1', 'DRAFT', 3, '2026-10-09T04:00:00+00:00');
  drafts = {
    MAIN: [
      // newest first, as the view answers: a rejected draft is final and never reopened (§4.6)
      draftItem('d-rejected', 'REJECTED', 2, '2026-10-09T04:30:00+00:00'),
      d1,
      draftItem('d0', 'PUBLISHED', 4, '2026-10-08T04:00:00+00:00'),
    ],
  };
  details = {
    d1: detailOf(d1, [auto('EUR'), usdManualSell()]),
    'd-rejected': detailOf(drafts.MAIN![0]!, [usdManualSell()]),
    d0: detailOf(drafts.MAIN![2]!, [auto('USD')]),
  };
  published = {
    MAIN: { version: 7, data: mainPublished() },
    B9: { version: 2, data: publishedOf(2, [line('USD', '1', '31.00000000', '32.00000000')]) },
  };
  policies = { MAIN: policyOf(), B9: policyOf() };
  publications = {
    MAIN: [publication(18, 'owner-1', 'PUBLISHED'), publication(17, 'rates-9', 'SUPERSEDED')],
  };
  reference = referenceOf();
  simulateReply = (id) => ({ status: 200, body: { id, isInBound: true, outOfBound: [] } });
  submitStatus = 'APPROVED';
  publishRefusal = null;
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  toast('');
  location.hash = '#rates';
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

function field(el: ParentNode, label: string): HTMLInputElement {
  const found = [...el.querySelectorAll('label')].filter((l) => text(l).startsWith(label));
  expect(found.length, `one label starting ${JSON.stringify(label)}`).toBe(1);
  const control = (found[0] as HTMLLabelElement).control as HTMLInputElement | null;
  expect(control, `the control of ${label}`).not.toBeNull();
  return control!;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function draftLine(el: ParentNode, code: string): HTMLElement {
  const found = el.querySelectorAll<HTMLElement>(`[data-draft-line="${code}"]`);
  expect(found.length, `one draft line [data-draft-line="${code}"]`).toBe(1);
  return found[0]!;
}

function publishedLine(el: ParentNode, code: string): HTMLElement {
  const found = el.querySelectorAll<HTMLElement>(`[data-published-line="${code}"]`);
  expect(found.length, `one published line [data-published-line="${code}"]`).toBe(1);
  return found[0]!;
}

function pageText(): string {
  return text(document.body);
}

async function click(el: ParentNode, name: string): Promise<void> {
  button(el, name).click();
  await settle();
}

/** The draft lines a write sent, keyed by currency. */
function sentLines(s: Sent): Record<string, Record<string, unknown>> {
  const lines = (s.body?.lines ?? []) as Array<Record<string, unknown>>;
  expect(Array.isArray(lines), `${s.method} ${s.path} carries lines`).toBe(true);
  const out: Record<string, Record<string, unknown>> = {};
  for (const l of lines) out[String(l.currency)] = l;
  return out;
}

function expectWellFormedWrites(): void {
  const keys = writes().map((w) => w.key);
  for (const w of writes()) {
    expect(w.key, `${w.method} ${w.path} carries an Idempotency-Key`).toBeTruthy();
  }
  expect(new Set(keys).size, `each write has a key of its own: ${JSON.stringify(keys)}`).toBe(keys.length);
}

function expectNoDirectSave(): void {
  expect(sent.filter((s) => s.path === RPC), 'nothing goes through /api/rpc (adminSaveRate is retired, D-3)').toEqual([]);
  for (const old of OLD_READS) {
    expect(readsOf(old).length, `${old} is not read: publications replace rates (D-3)`).toBe(0);
  }
}

// ── reads ────────────────────────────────────────────────────────────────────

describe('RE-5 A02: what it reads on show (#602; design §4.7, D-3)', () => {
  it('reads published_rates, rate_drafts, rate_policy, rates_reference and the open draft for the picked branch, writes nothing', async () => {
    mount();
    await settle();
    for (const path of [PUBLISHED, DRAFTS, POLICY]) {
      expect(readsOf(path).length, `${path} is read`).toBeGreaterThanOrEqual(1);
      expect(readsOf(path)[0]!.params.get('branch'), `${path} for the picked branch`).toBe('MAIN');
    }
    expect(readsOf(REFERENCE).length, 'the BoT reference line is read').toBeGreaterThanOrEqual(1);
    const draftReads = readsOf(DRAFT);
    expect(draftReads.length, 'the open draft is read').toBeGreaterThanOrEqual(1);
    expect(
      draftReads.map((r) => r.params.get('id')),
      'the open draft is the newest DRAFT/PENDING_APPROVAL/APPROVED one: d1, not the newer REJECTED one',
    ).toEqual(draftReads.map(() => 'd1'));
    expect(writes(), 'nothing is written on show').toEqual([]);
    const ownPaths = sent.filter((x) => !PAGE_WIDE.includes(x.path)).map((x) => x.path);
    for (const path of ownPaths) {
      expect([PUBLISHED, DRAFTS, DRAFT, POLICY, REFERENCE], `an unexpected read on show: ${path}`).toContain(path);
    }
    expectNoDirectSave();
  });

  it('a branch change reads the other branch', async () => {
    const m = mount({ branch: 'MAIN' });
    await settle();
    m.setBranch('B9');
    await settle();
    expect(readsOf(PUBLISHED).some((r) => r.params.get('branch') === 'B9')).toBe(true);
    expect(readsOf(DRAFTS).some((r) => r.params.get('branch') === 'B9')).toBe(true);
    expect(text(publishedLine(m.el, 'USD'))).toContain('31.00');
    expect(m.el.querySelectorAll('[data-published-line="JPY"]').length, "MAIN's JPY line is gone").toBe(0);
  });
});

// ── the display ──────────────────────────────────────────────────────────────

describe('RE-5 A02: every rate through formatRate, per its unit (design §3.3, §4.2)', () => {
  it('the current publication: JPY per 100 with its unit, USD per 1 without one, trailing zeros trimmed', async () => {
    const m = mount();
    await settle();
    const jpy = text(publishedLine(m.el, 'JPY'));
    expect(jpy).toContain('23.50 / 100 JPY');
    expect(jpy).toContain('24.12345678 / 100 JPY');
    expect(jpy, 'the wire string is not shown raw').not.toContain('23.50000000');
    const usd = text(publishedLine(m.el, 'USD'));
    expect(usd).toContain('33.25');
    expect(usd).toContain('34.10');
    expect(usd).not.toContain('/ 1 USD');
    expect(usd).not.toContain('33.25000000');
  });

  it('the BoT reference line shows the yen per 100 with its unit: buying transfer and selling (A6), no per-1 conversion', async () => {
    const m = mount();
    await settle();
    const all = text(m.el);
    expect(all).toContain('21.01 / 100 JPY');
    expect(all).toContain('21.63 / 100 JPY');
    expect(all, "v2's per-1 note is gone: the reference is shown in BoT's own unit").not.toContain(
      'แปลงจากต่อ 100 เยนเป็นต่อ 1 เยนแล้ว',
    );
    expect(all, 'USD is per 1: no unit suffix').toContain('33.5008');
    expect(all).not.toContain('/ 1 USD');
  });

  it('a MANUAL value of the open draft is shown through formatRate', async () => {
    const m = mount();
    await settle();
    const usd = text(draftLine(m.el, 'USD'));
    expect(usd).toContain('40.00');
    expect(usd).not.toContain('40.00000000');
    expect(text(draftLine(m.el, 'EUR')).length).toBeGreaterThan(0);
  });
});

// ── draft → simulate → submit → publish ─────────────────────────────────────

describe('RE-5 A02: draft → simulate → submit → publish (design §4.6, §4.7, §9)', () => {
  it('new draft: fetches the reference first, then creates a draft of every published currency for the picked branch, then reads it', async () => {
    drafts = { MAIN: [], B9: [] };
    details = {};
    published.B9 = { version: 2, data: publishedOf(2, [line('USD', '1', '31.00000000', '32.00000000'), line('JPY', '100', '23.50000000', '24.00000000')]) };
    const m = mount({ branch: 'B9' });
    await settle();
    await click(m.el, say('newDraft'));

    const w = writes();
    expect(w.map((s) => `${s.method} ${s.path}`)).toEqual(['POST /api/rate-references/fetch', 'POST /api/rate-drafts']);
    for (const s of w) expect(s.body?.branch, `${s.path} names the picked branch`).toBe('B9');
    const lines = sentLines(w[1]!);
    expect(Object.keys(lines).sort(), 'one line per currency of the current publication').toEqual(['JPY', 'USD']);
    for (const l of Object.values(lines)) {
      expect(['AUTO', 'MANUAL']).toContain(l.buyMode);
      expect(['AUTO', 'MANUAL']).toContain(l.sellMode);
    }
    expectWellFormedWrites();
    expect(
      readsOf(DRAFT).some((r) => r.params.get('id') === 'd-new-1' && r.params.get('branch') === 'B9'),
      'the new draft is read',
    ).toBe(true);
    expectNoDirectSave();
  });

  it('simulate, submit, publish in that order on the open draft; publish carries the current publication version and reads published_rates again', async () => {
    simulateReply = (id) => ({ status: 200, body: { id, isInBound: true, outOfBound: [] } });
    const m = mount();
    await settle();
    await click(m.el, say('simulate'));
    await click(m.el, say('submit'));
    const publishedReadsBefore = readsOf(PUBLISHED).length;
    await click(m.el, say('publish'));

    const w = writes();
    expect(w.map((s) => `${s.method} ${s.path}`)).toEqual([
      'POST /api/rate-drafts/d1/simulate',
      'POST /api/rate-drafts/d1/submit',
      'POST /api/rate-drafts/d1/publish',
    ]);
    for (const s of w) expect(s.body?.branch, `${s.path} names the picked branch`).toBe('MAIN');
    expect(w[2]!.body?.expectedVersion, 'expected_version = published_rates.publication.version (§4.6)').toBe(7);
    expectWellFormedWrites();
    expect(readsOf(PUBLISHED).length, 'published_rates is read again after the publish').toBeGreaterThan(
      publishedReadsBefore,
    );
    expectNoDirectSave();
  });

  it("publish on another branch carries that branch's version", async () => {
    drafts.B9 = [{ ...draftItem('d9', 'APPROVED', 2, '2026-10-09T04:00:00+00:00'), branchId: 'B9' }];
    details.d9 = detailOf(drafts.B9[0]!, [auto('USD')]);
    const m = mount({ branch: 'B9' });
    await settle();
    await click(m.el, say('publish'));
    const p = writes().filter((s) => s.path === '/api/rate-drafts/d9/publish');
    expect(p.length).toBe(1);
    expect(p[0]!.body?.expectedVersion).toBe(2);
    expect(p[0]!.body?.branch).toBe('B9');
  });

  it('before the first publication of a branch, publish sends expectedVersion null', async () => {
    published.B9 = { version: 1, data: publishedOf(null, []) };
    drafts.B9 = [{ ...draftItem('d9', 'APPROVED', 2, '2026-10-09T04:00:00+00:00'), branchId: 'B9' }];
    details.d9 = detailOf(drafts.B9[0]!, [auto('USD')]);
    const m = mount({ branch: 'B9' });
    await settle();
    await click(m.el, say('publish'));
    const p = writes().filter((s) => s.path === '/api/rate-drafts/d9/publish');
    expect(p.length).toBe(1);
    expect(p[0]!.body?.expectedVersion ?? null).toBeNull();
  });

  it('a staleVersion answer to publish is shown, not swallowed', async () => {
    details.d1!.draft!.status = 'APPROVED';
    drafts.MAIN![1]!.status = 'APPROVED';
    publishRefusal = { status: 409, code: 'staleVersion', message: 'Publication 8 is already current (test)' };
    const m = mount();
    await settle();
    await click(m.el, say('publish'));
    expect(writes().filter((s) => s.path === '/api/rate-drafts/d1/publish').length).toBe(1);
    expect(pageText()).toContain('ข้อมูลมีการเปลี่ยนแปลง กรุณาโหลดหน้าใหม่แล้วลองอีกครั้ง');
  });

  it('an approvalRequired answer to publish is shown, not swallowed', async () => {
    details.d1!.draft!.status = 'APPROVED';
    drafts.MAIN![1]!.status = 'APPROVED';
    publishRefusal = { status: 409, code: 'approvalRequired', message: 'The approval no longer matches this draft (test)' };
    const m = mount();
    await settle();
    await click(m.el, say('publish'));
    expect(pageText()).toContain('The approval no longer matches this draft (test)');
  });
});

// ── the evaluation ───────────────────────────────────────────────────────────

describe('RE-5 A02: an out-of-bound line is marked (design §4.5; simulate answers outOfBound)', () => {
  it('simulate answers USD SELL out of bound: the USD line is marked, the EUR line is not', async () => {
    simulateReply = (id) => ({ status: 200, body: { id, isInBound: false, outOfBound: [{ currency: 'USD', side: 'SELL' }] } });
    const m = mount();
    await settle();
    expect(text(draftLine(m.el, 'USD')), 'nothing is marked before simulate').not.toContain(say('outOfBound'));
    await click(m.el, say('simulate'));
    expect(text(draftLine(m.el, 'USD'))).toContain(say('outOfBound'));
    expect(text(draftLine(m.el, 'EUR'))).not.toContain(say('outOfBound'));
    expect(writes().map((s) => s.path), 'simulate is the only write').toEqual(['/api/rate-drafts/d1/simulate']);
  });

  it('an in-bound answer marks no line and says so', async () => {
    simulateReply = (id) => ({ status: 200, body: { id, isInBound: true, outOfBound: [] } });
    const m = mount();
    await settle();
    await click(m.el, say('simulate'));
    expect(text(draftLine(m.el, 'USD'))).not.toContain(say('outOfBound'));
    expect(text(draftLine(m.el, 'EUR'))).not.toContain(say('outOfBound'));
    expect(text(m.el)).toContain(say('inBound'));
  });
});

// ── adjust all ───────────────────────────────────────────────────────────────

describe('RE-5 A02: "adjust all" changes the percents only; MANUAL keeps its value (design §4.3, §4.4; Blueprint §05)', () => {
  async function adjustAll(): Promise<void> {
    const m = mount();
    await settle();
    type(field(m.el, say('adjustBuyPct')), '-0.5');
    type(field(m.el, say('adjustSellPct')), '1.25');
    await click(m.el, say('adjustAll'));

    const w = writes();
    expect(w.map((s) => `${s.method} ${s.path}`)).toEqual(['POST /api/rate-policies', 'PUT /api/rate-drafts/d1']);
    const policy = w[0]!.body!;
    expect(policy.buyAdjustPct, 'the percent as typed: the screen computes nothing (W7)').toBe('-0.5');
    expect(policy.sellAdjustPct).toBe('1.25');
    expect(policy.guards, 'the guards of the current policy, unchanged').toEqual(GUARDS);
    expect(policy.maxReferenceAgeHours).toBe(24);
    expect(policy.onStale).toBe('STOP');
    expect(policy.staleLimitThb ?? null).toBeNull();
    expect(typeof policy.reason === 'string' && policy.reason.trim() !== '', 'a policy version carries a reason').toBe(
      true,
    );
    expect(policy.branch).toBe('MAIN');

    const edit = w[1]!;
    expect(edit.body?.expectedVersion, "the draft's version").toBe(3);
    expect(edit.body?.branch).toBe('MAIN');
    const lines = sentLines(edit);
    expect(Object.keys(lines).sort()).toEqual(['EUR', 'USD']);
    expect(lines.USD!.sellMode, 'MANUAL stays MANUAL').toBe('MANUAL');
    expect(lines.USD!.manualSell, 'MANUAL keeps its value, digit for digit').toBe('40.00000000');
    expect(lines.USD!.reason, 'and its reason').toBe('a spike');
    expect(lines.USD!.buyMode, 'the AUTO side stays AUTO').toBe('AUTO');
    expect(lines.USD!.manualBuy ?? null, 'an AUTO side carries no manual value').toBeNull();
    expect([lines.EUR!.buyMode, lines.EUR!.sellMode]).toEqual(['AUTO', 'AUTO']);
    expectWellFormedWrites();
    expectNoDirectSave();
  }

  it('sends the typed percents with the current guards, then re-saves the draft with its MANUAL side unchanged', async () => {
    await adjustAll();
  });

  it('is there for a RATES member too, and sends the same: the percents only, the frame unchanged', async () => {
    meRole = 'RATES';
    await adjustAll();
  });
});

// ── A03 history ──────────────────────────────────────────────────────────────

describe('RE-5 A03: the history reads publications, not the old rates log (D-3)', () => {
  it('lists each publication version with who published it', async () => {
    const m = mount();
    await settle();
    await click(m.el, say('rateHistoryButton'));
    const reads = readsOf(PUBLICATIONS);
    expect(reads.length).toBeGreaterThanOrEqual(1);
    expect(reads[0]!.params.get('branch')).toBe('MAIN');
    const all = pageText();
    expect(all).toContain('18');
    expect(all).toContain('17');
    expect(all).toContain('owner-1');
    expect(all).toContain('rates-9');
    expectNoDirectSave();
  });
});

// ── languages ────────────────────────────────────────────────────────────────

describe('RE-5: Thai and English through the menu\'s own table (W10)', () => {
  it('every key this test reads is in both tables, the English one with no Thai', () => {
    for (const k of KEYS) {
      say(k, 'th');
      expect(THAI.test(say(k, 'en')), `en.${k} has no Thai`).toBe(false);
    }
  });

  it('in English: the screen, an out-of-bound mark and the history carry no Thai, and no raw status code shows in Thai', async () => {
    simulateReply = (id) => ({ status: 200, body: { id, isInBound: false, outOfBound: [{ currency: 'USD', side: 'SELL' }] } });
    setLang('en');
    try {
      const m = mount();
      await settle();
      await click(m.el, say('simulate', 'en'));
      expect(text(draftLine(m.el, 'USD'))).toContain(say('outOfBound', 'en'));
      expect(text(m.el)).not.toMatch(THAI);
      const attrs = [...m.el.querySelectorAll('[aria-label], [title], [placeholder]')].map(
        (e) => `${e.getAttribute('aria-label') ?? ''} ${e.getAttribute('title') ?? ''} ${e.getAttribute('placeholder') ?? ''}`,
      );
      expect(attrs.filter((a) => THAI.test(a))).toEqual([]);
      await click(m.el, say('rateHistoryButton', 'en'));
      expect(pageText()).not.toMatch(THAI);
    } finally {
      setLang('th');
    }
  });

  it('in Thai: a draft state is shown through the table, never as its code', async () => {
    submitStatus = 'PENDING_APPROVAL';
    const m = mount();
    await settle();
    expect(text(m.el)).not.toContain('PENDING_APPROVAL');
    await click(m.el, say('submit'));
    expect(text(m.el)).not.toContain('PENDING_APPROVAL');
    expect(text(m.el)).not.toMatch(/\bDRAFT\b/);
  });
});

// ── the source ───────────────────────────────────────────────────────────────

describe('RE-5: the source formats rates only through formatRate and saves nothing directly (design §3.3, D-3)', () => {
  const own = Object.entries(sources).filter(([path]) => !path.includes('.test.'));

  it('imports formatRate from shared/ui/rate', () => {
    const all = own.map(([, t]) => t).join('\n');
    expect(all).toMatch(/import\s*\{[^}]*\bformatRate\b[^}]*\}\s*from\s*['"]\.\.\/\.\.\/\.\.\/shared\/ui\/rate['"]/);
  });

  it('never names adminSaveRate, admin_rates or admin_rate_history', () => {
    for (const [path, t] of own) {
      expect(t, path).not.toMatch(/adminSaveRate|['"]admin_rates['"]|['"]admin_rate_history['"]/);
    }
  });
});
