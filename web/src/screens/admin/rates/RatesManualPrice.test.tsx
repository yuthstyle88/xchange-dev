// A02 manual price (survey xchange#564 issuecomment-6262359159 row 7): a draft line can set either side
// to MANUAL with a value, and the line to a reason; the screen saves the lines with the draft's version.
// Written before the implementation by the sub-controller (claude/dell-claude-3). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/blueprint/rates.md` §4.4 (`:351`, MANUAL per side with a reason) and §4.7 (edit a
// draft: `PUT /api/rate-drafts/{id}` with `expectedVersion`); the wire is
// `web/src/shared/api/generated/rate_drafts_cmd.ts:7` (`DraftLineInput`). The server refuses a MANUAL side
// without its value (`api/src/presentation/http/v3/commands/mod.rs:83-139`, field `manualBuy` /
// `manualSell`) and the database refuses a MANUAL line without a reason
// (`api/migrations/20261010120000_rate_engine.sql:74`), so the screen refuses both before it sends.
//
// The harness (fake API, mount, DOM helpers) is copied unchanged from `RatesDraft.test.tsx`.
// DOM hooks this test assumes (the implementer follows them), inside `[data-draft-line="<CODE>"]`:
// - `select[name="buyMode"]` and `select[name="sellMode"]`, options `AUTO` and `MANUAL` (values);
// - `input[name="manualBuy"]` / `input[name="manualSell"]`, present only while that side is MANUAL;
// - one `input[name="reason"]` per line;
// - a refusal on screen is an element `[data-field-error="<field>"]` inside the line
//   (`manualBuy`, `manualSell` or `reason`).
// - The lines are saved by one button whose text is the i18n key `saveLines`; it sends every line of the
//   draft, AUTO sides with a null manual value.
// - Keys of `./i18n.ts` (th and en): `saveLines`, `modeAuto`, `modeManual`, `reasonLabel`,
//   `manualValueRequired`, `reasonRequired`.
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
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

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

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/rates/menu.ts exists').toBeDefined();
  return m!.menu;
}

const KEYS = [
  'saveLines',
  'modeAuto',
  'modeManual',
  'reasonLabel',
  'manualValueRequired',
  'reasonRequired',
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
let putRefusal: { status: number; code: string; field: string; message: string } | null;

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
    if (path === PUBLICATIONS) return viewAnswer(1, { publications: publications[branch] ?? [] }, since);
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
      if (putRefusal) {
        return { status: putRefusal.status, body: { error: { code: putRefusal.code, field: putRefusal.field, message: putRefusal.message } } };
      }
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
  putRefusal = null;
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


// ── reads ────────────────────────────────────────────────────────────────────


// ── helpers of this file ─────────────────────────────────────────────────────

function control<T extends Element>(line: HTMLElement, selector: string): T | null {
  return line.querySelector<T>(selector);
}

function pick(select: HTMLSelectElement | null, value: string, what: string): void {
  expect(select, what).not.toBeNull();
  select!.value = value;
  select!.dispatchEvent(new Event('input', { bubbles: true }));
  select!.dispatchEvent(new Event('change', { bubbles: true }));
}

function puts(): Sent[] {
  return writes().filter((w) => w.method === 'PUT' && w.path.startsWith('/api/rate-drafts/'));
}

describe('A02 manual price: the open draft shows each side and its manual value (rates.md §4.4)', () => {
  it('shows USD sell as MANUAL 40 with its reason, and USD buy as AUTO with no manual input', async () => {
    const { el } = mount();
    await settle();
    const usd = draftLine(el, 'USD');
    expect(control<HTMLSelectElement>(usd, 'select[name="sellMode"]')?.value).toBe('MANUAL');
    expect(control<HTMLInputElement>(usd, 'input[name="manualSell"]')?.value).toBe('40.00000000');
    expect(control<HTMLInputElement>(usd, 'input[name="reason"]')?.value).toBe('a spike');
    expect(control<HTMLSelectElement>(usd, 'select[name="buyMode"]')?.value).toBe('AUTO');
    expect(control(usd, 'input[name="manualBuy"]'), 'no manual buy input while buy is AUTO').toBeNull();
  });
});

describe('A02 manual price: saving the lines (rates.md §4.4, §4.7)', () => {
  it('EUR buy MANUAL 38.5 with a reason: one PUT with the draft version and every line, USD unchanged', async () => {
    const { el } = mount();
    await settle();
    const eur = draftLine(el, 'EUR');
    pick(control<HTMLSelectElement>(eur, 'select[name="buyMode"]'), 'MANUAL', 'EUR buy mode');
    await settle();
    const manualBuy = control<HTMLInputElement>(eur, 'input[name="manualBuy"]');
    expect(manualBuy, 'a manual buy input appears for MANUAL').not.toBeNull();
    type(manualBuy!, '38.5');
    type(control<HTMLInputElement>(eur, 'input[name="reason"]')!, 'promo week');
    await click(el, say('saveLines'));

    expect(puts().length, 'one PUT of the lines').toBe(1);
    const put = puts()[0]!;
    expect(put.path).toBe('/api/rate-drafts/d1');
    expect(put.body?.expectedVersion).toBe(3);
    expect(put.body?.branch).toBe('MAIN');
    const lines = sentLines(put);
    expect(lines.EUR).toEqual({
      currency: 'EUR',
      buyMode: 'MANUAL',
      sellMode: 'AUTO',
      manualBuy: '38.5',
      manualSell: null,
      reason: 'promo week',
    });
    expect(lines.USD).toEqual({
      currency: 'USD',
      buyMode: 'AUTO',
      sellMode: 'MANUAL',
      manualBuy: null,
      manualSell: '40.00000000',
      reason: 'a spike',
    });
    expectWellFormedWrites();
  });

  it('USD sell back to AUTO: the line goes as AUTO/AUTO with no manual value', async () => {
    const { el } = mount();
    await settle();
    const usd = draftLine(el, 'USD');
    pick(control<HTMLSelectElement>(usd, 'select[name="sellMode"]'), 'AUTO', 'USD sell mode');
    await settle();
    expect(control(usd, 'input[name="manualSell"]'), 'the manual input goes with MANUAL').toBeNull();
    await click(el, say('saveLines'));
    expect(puts().length).toBe(1);
    const usdSent = sentLines(puts()[0]!).USD!;
    expect(usdSent.buyMode).toBe('AUTO');
    expect(usdSent.sellMode).toBe('AUTO');
    expect(usdSent.manualBuy).toBeNull();
    expect(usdSent.manualSell).toBeNull();
  });

  it('MANUAL with no value is refused on screen (manualBuy) and sends nothing', async () => {
    const { el } = mount();
    await settle();
    const eur = draftLine(el, 'EUR');
    pick(control<HTMLSelectElement>(eur, 'select[name="buyMode"]'), 'MANUAL', 'EUR buy mode');
    await settle();
    type(control<HTMLInputElement>(eur, 'input[name="reason"]')!, 'promo week');
    button(el, say('saveLines')).click();
    await settle();
    expect(puts(), 'nothing is sent').toEqual([]);
    const err = eur.querySelector('[data-field-error="manualBuy"]');
    expect(err, 'the EUR line shows the manual-value refusal').not.toBeNull();
    expect(text(err)).toBe(say('manualValueRequired'));
  });

  it('a MANUAL line with no reason is refused on screen (reason) and sends nothing', async () => {
    const { el } = mount();
    await settle();
    const eur = draftLine(el, 'EUR');
    pick(control<HTMLSelectElement>(eur, 'select[name="buyMode"]'), 'MANUAL', 'EUR buy mode');
    await settle();
    type(control<HTMLInputElement>(eur, 'input[name="manualBuy"]')!, '38.5');
    button(el, say('saveLines')).click();
    await settle();
    expect(puts(), 'nothing is sent').toEqual([]);
    const err = eur.querySelector('[data-field-error="reason"]');
    expect(err, 'the EUR line shows the reason refusal').not.toBeNull();
    expect(text(err)).toBe(say('reasonRequired'));
  });

  it('a value that is not a positive decimal of at most 8 places is refused on screen and sends nothing', async () => {
    const { el } = mount();
    await settle();
    const eur = draftLine(el, 'EUR');
    pick(control<HTMLSelectElement>(eur, 'select[name="buyMode"]'), 'MANUAL', 'EUR buy mode');
    await settle();
    type(control<HTMLInputElement>(eur, 'input[name="reason"]')!, 'promo week');
    for (const bad of ['-1', 'abc', '1.123456789', '0']) {
      type(control<HTMLInputElement>(eur, 'input[name="manualBuy"]')!, bad);
      button(el, say('saveLines')).click();
      await settle();
      expect(puts(), `nothing is sent for ${JSON.stringify(bad)}`).toEqual([]);
      expect(eur.querySelector('[data-field-error="manualBuy"]'), `refused: ${JSON.stringify(bad)}`).not.toBeNull();
    }
  });

  it('a staleVersion answer to the save is shown, not swallowed, and the lines are not re-read', async () => {
    putRefusal = { status: 409, code: 'staleVersion', field: '', message: 'draft moved (test)' };
    const { el } = mount();
    await settle();
    const before = readsOf(DRAFT).length;
    await click(el, say('saveLines'));
    expect(puts().length, 'the PUT was sent').toBe(1);
    expect(pageText()).toContain('ข้อมูลมีการเปลี่ยนแปลง กรุณาโหลดหน้าใหม่แล้วลองอีกครั้ง');
    expect(readsOf(DRAFT).length, 'no re-read of the draft after a refused save').toBe(before);
  });
});

describe('A02 manual price: Thai and English (W10)', () => {
  it('every key this test reads is in both tables, the English one with no Thai', () => {
    for (const k of KEYS) {
      expect(say(k, 'th')).toMatch(THAI);
      expect(say(k, 'en')).not.toMatch(THAI);
    }
  });
});
