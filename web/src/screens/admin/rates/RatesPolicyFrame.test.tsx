// A3 policy (survey xchange#564 issuecomment-6262359159 row 8): the policy card edits the frame of a
// policy version (the per-currency guards, `maxReferenceAgeHours`, `onStale`, `staleLimitThb`) instead of
// re-sending the stored values. Written before the implementation by the sub-controller
// (claude/dell-claude-3). Do not edit, delete or weaken; add tests in a file of your own.
//
// Design: `docs/design/blueprint/rates.md` §4.3 (`:332-349`): guards per currency and side as
// `{ "<CODE>": { "buy": {"min_pct","max_pct"}, "sell": {...} } }` (scale 4, signed); `on_stale` STOP / LIMIT /
// NULL (NULL = quotes go on, the screens show the stale badge), LIMIT with `stale_limit_thb`. The wire is
// `POST /api/rate-policies` (`api/src/presentation/http/v3/commands/rate_policy_save.rs:40-59`): the server
// accepts `onStale` STOP or LIMIT or null (`:108-110`), `staleLimitThb` a decimal of 2 places (`:122-130`),
// and refuses a frame change by a caller without EXECUTIVE on the branch with `roleNotAllowedHere`
// (`:341-354`, `:445-449`).
//
// The harness is copied from `RatesDraft.test.tsx`; `me` here also answers `permissions: ['RatePolicySave']`
// (the save button needs it, `Rates.tsx:111`), and the policy POST can be refused.
// DOM hooks this test assumes (the implementer follows them), inside the policy card:
// - one row `[data-guard-line="<CODE>"]` per currency of the current publication, with inputs
//   `name="guardBuyMin"`, `guardBuyMax`, `guardSellMin`, `guardSellMax` (percent strings, as the server holds
//   them); a row left wholly empty means "no guard" for that currency and is left out of `guards`;
// - `input[name="maxReferenceAgeHours"]`; `select[name="onStale"]` with options `STOP`, `LIMIT` and `""`
//   (none = null); `input[name="staleLimitThb"]`, present only while `onStale` is `LIMIT`;
// - a refusal on screen is `[data-field-error="<field>"]` in the card: `guards`, `maxReferenceAgeHours`,
//   `staleLimitThb`;
// - the save is the existing `button[data-testid="save-policy"]`.
// - Keys of `./i18n.ts` (th and en): `guardsTitle`, `guardMin`, `guardMax`, `maxReferenceAgeHoursLabel`,
//   `onStaleLabel`, `onStaleStop`, `onStaleLimit`, `onStaleNone`, `staleLimitThbLabel`, `guardRangeInvalid`,
//   `maxReferenceAgeInvalid`, `staleLimitRequired`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
  makeRateDraftDetailData,
  makeRateDraftDetailItemDto,
  makeRateDraftItemDto,
  makeRateDraftLineDto,
  makeRatePolicyData,
  makeRatePolicyVersionDto,
  makeRatesReferenceData,
  makeRatesReferenceRow,
  type PublishedLineDto,
  type PublishedRatesData,
  type RateDraftDetailData,
  type RateDraftItemDto,
  type RateDraftLineDto,
  type RatePolicyData,
  type RatePublicationRow,
  type RatesReferenceData,
} from '../../../shared/testing/fixtures';
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
  'guardsTitle',
  'guardMin',
  'guardMax',
  'maxReferenceAgeHoursLabel',
  'onStaleLabel',
  'onStaleStop',
  'onStaleLimit',
  'onStaleNone',
  'staleLimitThbLabel',
  'guardRangeInvalid',
  'maxReferenceAgeInvalid',
  'staleLimitRequired',
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

function line(currency: string, unitSize: string, buy: string | null, sell: string | null) {
  return makePublishedLineDto({ currency, group: 'ALL', unitSize, buyRate: buy, sellRate: sell, lineId: `line-${currency}`, bands: [] });
}

function publishedOf(version: number | null, lines: PublishedLineDto[]) {
  return makePublishedRatesData({
    publication:
      version === null ? null : makePublishedPublicationDto({ id: `pub-${version}`, version, publishedAt: '2026-10-09T02:00:00+00:00' }),
    sourceDate: '2026-10-08',
    stale: false,
    lines,
    bandHeadings: [],
  });
}

/** MAIN's publication 7: USD per 1, JPY per 100 (§3.1 A1), EUR per 1. */
function mainPublished() {
  return publishedOf(7, [
    line('EUR', '1', '37.35520000', '38.08440000'),
    line('JPY', '100', '23.50000000', '24.12345678'),
    line('USD', '1', '33.25000000', '34.10000000'),
  ]);
}

function draftItem(id: string, status: string, version: number, createdAt: string) {
  return makeRateDraftItemDto({
    id,
    branchId: 'MAIN',
    status,
    version,
    createdBy: 'rates-9',
    createdAt,
    updatedAt: createdAt,
    evaluation: null,
  });
}

function auto(currency: string) {
  return makeRateDraftLineDto({ currency, buyMode: 'AUTO', sellMode: 'AUTO', manualBuy: null, manualSell: null, reason: null });
}

/** USD: buy AUTO, sell MANUAL 40 with its reason (the out-of-bound line of `rate_draft_flow_test.rs`). */
function usdManualSell() {
  return makeRateDraftLineDto({
    currency: 'USD',
    buyMode: 'AUTO',
    sellMode: 'MANUAL',
    manualBuy: null,
    manualSell: '40.00000000',
    reason: 'a spike',
  });
}

function detailOf(item: RateDraftItemDto, lines: RateDraftLineDto[]) {
  return makeRateDraftDetailData({ draft: makeRateDraftDetailItemDto({ ...item }), lines });
}

const GUARDS = {
  USD: { buy: { min_pct: '-1.0000', max_pct: '1.0000' }, sell: { min_pct: '-1.0000', max_pct: '1.0000' } },
  JPY: { buy: { min_pct: '-2.0000', max_pct: '0.5000' }, sell: { min_pct: '-0.5000', max_pct: '2.0000' } },
};

function policyOf() {
  return makeRatePolicyData({
    policy: makeRatePolicyVersionDto({
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
    }),
  });
}

function refRow(code: string, unit: number, sight: string, transfer: string, selling: string) {
  return makeRatesReferenceRow({ code, date: '2026-10-08', unit, buyingSight: sight, buyingTransfer: transfer, selling });
}

/** After RE-2 #599: BoT JPY per 100, in BoT's own unit (§4.2). */
function referenceOf() {
  return makeRatesReferenceData({
    rows: [refRow('USD', 1, '33.4218', '33.5008', '33.8278'), refRow('JPY', 100, '20.9500', '21.0100', '21.6300')],
    fetchedAt: '2026-10-09T01:30:00.000Z',
    error: null,
  });
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
let policyRefusal: { status: number; code: string; message: string } | null;

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

function linesOfBody(body: Record<string, unknown> | undefined) {
  const lines = (body?.lines ?? []) as Array<Record<string, unknown>>;
  return lines.map((l) =>
    makeRateDraftLineDto({
      currency: String(l.currency),
      buyMode: String(l.buyMode),
      sellMode: String(l.sellMode),
      manualBuy: (l.manualBuy as string | null | undefined) ?? null,
      manualSell: (l.manualSell as string | null | undefined) ?? null,
      reason: (l.reason as string | null | undefined) ?? null,
    }),
  );
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
            ? { kind: 'owner', id: 'owner-1', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner', permissions: ['RatePolicySave'] }
            : { kind: 'staff', id: 'employee-RATES', role: 'RATES', branch: 'MAIN', name: 'Rates', username: 'rates', permissions: ['RatePolicySave'] },
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
  if (path === '/api/rate-policies' && method === 'POST') {
    if (policyRefusal) return refusal(policyRefusal.status, policyRefusal.code, policyRefusal.message);
    return { status: 200, body: { id: 'pol-2', version: 2 } };
  }
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
  policyRefusal = null;
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


function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}



function pageText(): string {
  return text(document.body);
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

function policyCard(el: ParentNode): HTMLElement {
  const save = el.querySelector<HTMLButtonElement>('button[data-testid="save-policy"]');
  expect(save, 'the policy card has its save button').not.toBeNull();
  const card = save!.closest<HTMLElement>('.rates-section');
  expect(card, 'the save button sits in the policy card').not.toBeNull();
  return card!;
}

function guardLine(card: ParentNode, code: string): HTMLElement {
  const found = card.querySelectorAll<HTMLElement>(`[data-guard-line="${code}"]`);
  expect(found.length, `one guard row [data-guard-line="${code}"]`).toBe(1);
  return found[0]!;
}

function input(scope: ParentNode, name: string): HTMLInputElement {
  const found = scope.querySelector<HTMLInputElement>(`input[name="${name}"]`);
  expect(found, `input[name="${name}"]`).not.toBeNull();
  return found!;
}

function choose(scope: ParentNode, name: string, value: string): void {
  const sel = scope.querySelector<HTMLSelectElement>(`select[name="${name}"]`);
  expect(sel, `select[name="${name}"]`).not.toBeNull();
  sel!.value = value;
  sel!.dispatchEvent(new Event('input', { bubbles: true }));
  sel!.dispatchEvent(new Event('change', { bubbles: true }));
}

function policyPosts(): Sent[] {
  return writes().filter((w) => w.path === '/api/rate-policies' && w.method === 'POST');
}

async function save(el: ParentNode): Promise<void> {
  const btn = el.querySelector<HTMLButtonElement>('button[data-testid="save-policy"]')!;
  expect(btn.disabled, 'save is enabled').toBe(false);
  btn.click();
  await settle();
}

describe('A3 policy: the card shows the stored frame (rates.md §4.3)', () => {
  it('shows each guard of USD and JPY, the max reference age 24, STOP, and no stale-limit input', async () => {
    const { el } = mount();
    await settle();
    const card = policyCard(el);
    const usd = guardLine(card, 'USD');
    expect(input(usd, 'guardBuyMin').value).toBe('-1.0000');
    expect(input(usd, 'guardBuyMax').value).toBe('1.0000');
    expect(input(usd, 'guardSellMin').value).toBe('-1.0000');
    expect(input(usd, 'guardSellMax').value).toBe('1.0000');
    const jpy = guardLine(card, 'JPY');
    expect(input(jpy, 'guardBuyMin').value).toBe('-2.0000');
    expect(input(jpy, 'guardSellMax').value).toBe('2.0000');
    expect(input(card, 'maxReferenceAgeHours').value).toBe('24');
    expect(card.querySelector<HTMLSelectElement>('select[name="onStale"]')?.value).toBe('STOP');
    expect(card.querySelector('input[name="staleLimitThb"]'), 'no stale limit unless LIMIT').toBeNull();
  });

  it('a currency of the publication with no guard has an empty row (EUR)', async () => {
    const { el } = mount();
    await settle();
    const eur = guardLine(policyCard(el), 'EUR');
    for (const n of ['guardBuyMin', 'guardBuyMax', 'guardSellMin', 'guardSellMax']) {
      expect(input(eur, n).value, `EUR ${n}`).toBe('');
    }
  });
});

describe('A3 policy: saving the frame (rates.md §4.3; rate_policy_save.rs)', () => {
  it('USD buy max 0.8000, age 12, LIMIT 50000: one POST carrying them, the other guards unchanged', async () => {
    const { el } = mount();
    await settle();
    const card = policyCard(el);
    type(input(guardLine(card, 'USD'), 'guardBuyMax'), '0.8000');
    type(input(card, 'maxReferenceAgeHours'), '12');
    choose(card, 'onStale', 'LIMIT');
    await settle();
    type(input(card, 'staleLimitThb'), '50000');
    await save(el);

    expect(policyPosts().length, 'one policy save').toBe(1);
    const body = policyPosts()[0]!.body!;
    expect(body.branch).toBe('MAIN');
    expect(body.guards).toEqual({
      USD: { buy: { min_pct: '-1.0000', max_pct: '0.8000' }, sell: { min_pct: '-1.0000', max_pct: '1.0000' } },
      JPY: { buy: { min_pct: '-2.0000', max_pct: '0.5000' }, sell: { min_pct: '-0.5000', max_pct: '2.0000' } },
    });
    expect(body.maxReferenceAgeHours).toBe(12);
    expect(body.onStale).toBe('LIMIT');
    expect(body.staleLimitThb).toBe('50000');
    expectWellFormedWrites();
  });

  it('a guard typed into the empty EUR row adds EUR to guards', async () => {
    const { el } = mount();
    await settle();
    const eur = guardLine(policyCard(el), 'EUR');
    type(input(eur, 'guardBuyMin'), '-1.5000');
    type(input(eur, 'guardBuyMax'), '1.5000');
    type(input(eur, 'guardSellMin'), '-1.5000');
    type(input(eur, 'guardSellMax'), '1.5000');
    await save(el);
    const guards = policyPosts()[0]!.body!.guards as Record<string, unknown>;
    expect(guards.EUR).toEqual({
      buy: { min_pct: '-1.5000', max_pct: '1.5000' },
      sell: { min_pct: '-1.5000', max_pct: '1.5000' },
    });
  });

  it('onStale none sends null for onStale and staleLimitThb', async () => {
    const { el } = mount();
    await settle();
    choose(policyCard(el), 'onStale', '');
    await settle();
    await save(el);
    const body = policyPosts()[0]!.body!;
    expect(body.onStale).toBeNull();
    expect(body.staleLimitThb).toBeNull();
  });

  it('LIMIT with no limit is refused on screen (staleLimitThb) and sends nothing', async () => {
    const { el } = mount();
    await settle();
    const card = policyCard(el);
    choose(card, 'onStale', 'LIMIT');
    await settle();
    await save(el);
    expect(policyPosts()).toEqual([]);
    const err = card.querySelector('[data-field-error="staleLimitThb"]');
    expect(err).not.toBeNull();
    expect(text(err)).toBe(say('staleLimitRequired'));
  });

  it('a guard whose min is above its max is refused on screen (guards) and sends nothing', async () => {
    const { el } = mount();
    await settle();
    const card = policyCard(el);
    type(input(guardLine(card, 'USD'), 'guardBuyMin'), '2.0000');
    await save(el);
    expect(policyPosts()).toEqual([]);
    const err = card.querySelector('[data-field-error="guards"]');
    expect(err).not.toBeNull();
    expect(text(err)).toContain(say('guardRangeInvalid'));
  });

  it('a max reference age that is not a whole number of hours above 0 is refused on screen and sends nothing', async () => {
    const { el } = mount();
    await settle();
    const card = policyCard(el);
    for (const bad of ['0', '-3', '1.5', 'x']) {
      type(input(card, 'maxReferenceAgeHours'), bad);
      await save(el);
      expect(policyPosts(), `nothing sent for ${JSON.stringify(bad)}`).toEqual([]);
      expect(card.querySelector('[data-field-error="maxReferenceAgeHours"]'), `refused: ${bad}`).not.toBeNull();
    }
  });

  it("the server's roleNotAllowedHere answer to a frame change is shown, not swallowed", async () => {
    policyRefusal = { status: 403, code: 'roleNotAllowedHere', message: 'frame needs EXECUTIVE (test)' };
    const { el } = mount();
    await settle();
    type(input(policyCard(el), 'maxReferenceAgeHours'), '12');
    await save(el);
    expect(policyPosts().length).toBe(1);
    expect(pageText()).toMatch(/บทบาทนี้ไม่มีสิทธิ์|frame needs EXECUTIVE \(test\)/);
  });
});

describe('A3 policy: Thai and English (W10)', () => {
  it('every key this test reads is in both tables, the English one with no Thai', () => {
    for (const k of KEYS) {
      expect(say(k, 'th')).toMatch(THAI);
      expect(say(k, 'en')).not.toMatch(THAI);
    }
  });
});
