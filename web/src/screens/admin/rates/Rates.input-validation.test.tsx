// T13 #4912 (input-validation audit, design `docs/design/web-input-validation.md` §1 RC-2, §3): admin › Rates checks
// every typed number with the shared bounds (`web/src/shared/ui/bounded.ts`) before it sends, sends the trimmed text it
// checked, and says which tier blocks Save. Written before the implementation by the sub-controller
// (claude/studio-claude-4). Do not edit, delete or weaken; add tests in a file of your own.
//
// Server bounds (`api/src/presentation/http/v3/commands/rate_policy_save.rs`, `Rate::parse`): manual prices `RATE`
// (positive, 8 dp, <= 100000000, never trimmed); adjust % and tier margins `RATE_PERCENT` (numeric(9,4)); tier upper
// bound `MONEY` (2 dp); stale limit numeric(20,2) with no `+`.
//
// The harness (fake API, mount, DOM helpers) is copied from `RatesManualPrice.test.tsx`.
// DOM hooks this test assumes (the implementer follows them):
// - a refusal on screen is an element `[data-field-error="<field>"]` with worded text (Thai in th):
//   `manualBuy` inside `[data-draft-line="<CODE>"]`; `buyAdjustPct` for the adjust-buy % field; `staleLimitThb` for the
//   stale limit; `tier-<n>` for a tier's upper bound or margins (as `nonRisingTierNo` already does);
// - the adjust-buy % input is the input inside the label whose text is the i18n key `adjustBuyPct`;
// - the policy controls keep the hooks `RatesTiers.test.tsx` and `RatesPolicyFrame.test.tsx` use
//   (`input[name="valueTiersOn"]`, `tr[data-tier-no]`, `[data-field="…"]`, `select[name="onStale"]`,
//   `input[name="staleLimitThb"]`, `button[data-action="save-policy"]`).
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
} from '../../../shared/testing/fixtures';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

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

/** The screen's own Thai text for `key`. */
function say(key: string): string {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/rates/i18n.ts exists').toBeDefined();
  const value = m!.th[key];
  expect(value, `admin.rates i18n th.${key}`).toBeTruthy();
  return value!;
}

const V = '/api/v3/views/';
const THAI = /[฀-๿]/;

// ── the answers ──────────────────────────────────────────────────────────────

function line(currency: string, buy: string, sell: string) {
  return makePublishedLineDto({ currency, group: 'ALL', unitSize: '1', buyRate: buy, sellRate: sell, lineId: `line-${currency}`, bands: [] });
}

function published() {
  return makePublishedRatesData({
    publication: makePublishedPublicationDto({ id: 'pub-7', version: 7, publishedAt: '2026-10-09T02:00:00+00:00' }),
    sourceDate: '2026-10-08',
    stale: false,
    lines: [line('EUR', '37.35520000', '38.08440000'), line('USD', '33.25000000', '34.10000000')],
    bandHeadings: [],
  });
}

function draftItem() {
  return makeRateDraftItemDto({
    id: 'd1',
    branchId: 'MAIN',
    status: 'DRAFT',
    version: 3,
    createdBy: 'rates-9',
    createdAt: '2026-10-09T04:00:00+00:00',
    updatedAt: '2026-10-09T04:00:00+00:00',
    evaluation: null,
  });
}

function auto(currency: string) {
  return makeRateDraftLineDto({ currency, buyMode: 'AUTO', sellMode: 'AUTO', manualBuy: null, manualSell: null, reason: null });
}

let onStale: string;
let staleLimitThb: string | null;

function policy() {
  return makeRatePolicyData({
    policy: makeRatePolicyVersionDto({
      id: 'pol-1',
      branchId: 'MAIN',
      version: 1,
      buyAdjustPct: '0.0000',
      sellAdjustPct: '0.0000',
      guards: {},
      maxReferenceAgeHours: 24,
      onStale,
      staleLimitThb,
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

// ── the fake API: the global fetch ──────────────────────────────────────────

interface Sent {
  path: string;
  method: string;
  body: Record<string, unknown> | undefined;
}

let sent: Sent[];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function view(data: unknown): Response {
  return json(200, { version: 1, changed: true, data: structuredClone(data) });
}

async function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = typeof init?.body === 'string' && init.body !== '' ? JSON.parse(init.body) : undefined;
  const path = url.pathname;
  sent.push({ path, method, body });
  if (path === `${V}me`) {
    return view({
      kind: 'owner',
      id: 'owner-1',
      role: 'OWNER',
      branch: 'MAIN',
      name: 'Owner',
      username: 'owner',
      permissions: ['OverviewRead', 'RatesDraft', 'RatesHistoryRead', 'RatePolicySave', 'SystemSettings'],
    });
  }
  if (path === QUEUE_CARD_PATH) return json(200, queueCardUnchanged());
  if (method === 'GET') {
    if (path === `${V}published_rates`) return view(published());
    if (path === `${V}rate_drafts`) return view({ drafts: [draftItem()] });
    if (path === `${V}rate_draft`) {
      return view(makeRateDraftDetailData({ draft: makeRateDraftDetailItemDto({ ...draftItem() }), lines: [auto('EUR'), auto('USD')] }));
    }
    if (path === `${V}rate_policy`) return view(policy());
    if (path === `${V}rate_publications`) return view({ publications: [] });
    if (path === `${V}rates_reference`) {
      return view(makeRatesReferenceData({ rows: [], fetchedAt: '2026-10-09T01:30:00.000Z', error: null }));
    }
    return json(500, { error: { code: 'serverError', message: `unexpected read ${path}` } });
  }
  if (path === '/api/rate-policies' && method === 'POST') return json(200, { id: 'pol-2', version: 2 });
  if (path === '/api/rate-drafts/d1' && method === 'PUT') return json(200, { id: 'd1', version: 4, status: 'DRAFT' });
  if (path === '/api/rate-drafts/d1/simulate' && method === 'POST') return json(200, { id: 'd1', isInBound: true, outOfBound: [] });
  return json(500, { error: { code: 'serverError', message: `unexpected ${method} ${path}` } });
}

function puts(): Sent[] {
  return sent.filter((s) => s.method === 'PUT' && s.path.startsWith('/api/rate-drafts/'));
}
function policySaves(): Sent[] {
  return sent.filter((s) => s.method === 'POST' && s.path === '/api/rate-policies');
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

function mount(): HTMLElement {
  const menu = theMenu();
  const [branch] = createSignal('MAIN');
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
  return el;
}

beforeEach(() => {
  sent = [];
  onStale = 'STOP';
  staleLimitThb = null;
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

function pick(select: HTMLSelectElement | null, value: string, what: string): void {
  expect(select, what).not.toBeNull();
  select!.value = value;
  select!.dispatchEvent(new Event('input', { bubbles: true }));
  select!.dispatchEvent(new Event('change', { bubbles: true }));
}

function find<T extends Element>(scope: ParentNode, selector: string, what: string): T {
  const el = scope.querySelector<T>(selector);
  expect(el, what).not.toBeNull();
  return el!;
}

function buttonNamed(scope: ParentNode, name: string): HTMLButtonElement {
  const found = [...scope.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
  expect(found.length, `one button ${JSON.stringify(name)}`).toBe(1);
  return found[0]!;
}

function inputLabelled(scope: ParentNode, label: string): HTMLInputElement {
  const labels = [...scope.querySelectorAll('label')].filter((l) => text(l.querySelector('.form-label')) === label);
  expect(labels.length, `one field labelled ${JSON.stringify(label)}`).toBe(1);
  return find<HTMLInputElement>(labels[0]!, 'input', `the input of ${JSON.stringify(label)}`);
}

function draftLine(scope: ParentNode, code: string): HTMLElement {
  return find<HTMLElement>(scope, `[data-draft-line="${code}"]`, `the draft line ${code}`);
}

/** EUR buy set to MANUAL with `value` and a reason. */
async function manualEurBuy(el: HTMLElement, value: string): Promise<HTMLElement> {
  const eur = draftLine(el, 'EUR');
  pick(eur.querySelector<HTMLSelectElement>('select[name="buyMode"]'), 'MANUAL', 'EUR buy mode');
  await settle();
  type(find<HTMLInputElement>(eur, 'input[name="manualBuy"]', 'EUR manual buy input'), value);
  type(find<HTMLInputElement>(eur, 'input[name="reason"]', 'EUR reason input'), 'promo week');
  return eur;
}

function saveButton(el: ParentNode): HTMLButtonElement {
  return find<HTMLButtonElement>(el, 'button[data-action="save-policy"]', 'the save policy button');
}

function tierRow(el: ParentNode, n: number): HTMLElement {
  return find<HTMLElement>(el, `tr[data-tier-no="${n}"]`, `tier row ${n}`);
}

/** Value tiers on, three rising upper bounds and every margin filled. */
async function validTiers(el: HTMLElement): Promise<void> {
  const sw = find<HTMLInputElement>(el, 'input[name="valueTiersOn"]', 'value tiers switch');
  sw.checked = true;
  sw.dispatchEvent(new Event('click', { bubbles: true }));
  sw.dispatchEvent(new Event('input', { bubbles: true }));
  sw.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
  const uppers = ['20000.00', '100000.00', '500000.00'];
  for (let n = 1; n <= 4; n++) {
    const row = tierRow(el, n);
    if (n < 4) type(find<HTMLInputElement>(row, '[data-field="upperInclusiveThb"]', `tier ${n} upper`), uppers[n - 1]!);
    type(find<HTMLInputElement>(row, '[data-field="buyMarginPct"]', `tier ${n} buy margin`), '0.1000');
    type(find<HTMLInputElement>(row, '[data-field="sellMarginPct"]', `tier ${n} sell margin`), '0.1000');
  }
  await settle();
}

/** A worded refusal: one element at the field, with Thai text (the language under test). */
function expectWordedLine(scope: ParentNode, field: string): string {
  const err = scope.querySelector(`[data-field-error="${field}"]`);
  expect(err, `a refusal line [data-field-error="${field}"]`).not.toBeNull();
  const words = text(err);
  expect(words, `the ${field} refusal is worded in Thai`).toMatch(THAI);
  return words;
}

// ── tests ────────────────────────────────────────────────────────────────────

describe('T13 #4912 admin › Rates: typed values are checked with the shared bounds before they are sent', () => {
  it('RT1: manual buy " 32.1 " is sent as "32.1" (Rate::parse never trims)', async () => {
    const el = mount();
    await settle();
    await manualEurBuy(el, ' 32.1 ');
    buttonNamed(el, say('saveLines')).click();
    await settle();
    expect(puts().length, 'one PUT of the lines').toBe(1);
    const lines = (puts()[0]!.body?.lines ?? []) as Array<Record<string, unknown>>;
    const eur = lines.find((l) => l.currency === 'EUR');
    expect(eur?.manualBuy, 'the trimmed text that was checked is sent').toBe('32.1');
  });

  it('RT2: manual buy "100000000.5" (over the RATE cap) shows a worded line and sends nothing', async () => {
    const el = mount();
    await settle();
    const eur = await manualEurBuy(el, '100000000.5');
    buttonNamed(el, say('saveLines')).click();
    await settle();
    expect(puts(), 'nothing is sent').toEqual([]);
    expectWordedLine(eur, 'manualBuy');
  });

  it('RT3: stale limit "+50" shows a worded line and sends nothing', async () => {
    onStale = 'LIMIT';
    staleLimitThb = '100.00';
    const el = mount();
    await settle();
    pick(el.querySelector<HTMLSelectElement>('select[name="onStale"]'), 'LIMIT', 'on-stale select');
    await settle();
    type(find<HTMLInputElement>(el, 'input[name="staleLimitThb"]', 'stale limit input'), '+50');
    await settle();
    saveButton(el).click();
    await settle();
    expect(policySaves(), 'no policy save is sent').toEqual([]);
    expectWordedLine(el, 'staleLimitThb');
  });

  it('RT4: tier 1 upper "1000.123" (MONEY takes 2 decimals) shows a worded line and sends nothing', async () => {
    const el = mount();
    await settle();
    await validTiers(el);
    type(find<HTMLInputElement>(tierRow(el, 1), '[data-field="upperInclusiveThb"]', 'tier 1 upper'), '1000.123');
    await settle();
    saveButton(el).click();
    await settle();
    expect(policySaves(), 'no policy save is sent').toEqual([]);
    expectWordedLine(el, 'tier-1');
  });

  it('RT5: a blank tier 2 margin disables Save and a line names tier 2', async () => {
    const el = mount();
    await settle();
    await validTiers(el);
    type(find<HTMLInputElement>(tierRow(el, 2), '[data-field="buyMarginPct"]', 'tier 2 buy margin'), '');
    await settle();
    expect(saveButton(el).disabled, 'Save is disabled').toBe(true);
    const words = expectWordedLine(el, 'tier-2');
    expect(words, 'the line names tier 2').toContain(`${say('tierLabelPrefix').trim()} 2`);
  });

  it('RT6: adjust buy % "1.23456" (RATE_PERCENT takes 4 decimals) shows a worded line and sends nothing', async () => {
    const el = mount();
    await settle();
    type(inputLabelled(el, say('adjustBuyPct')), '1.23456');
    await settle();
    buttonNamed(el, say('adjustAll')).click();
    await settle();
    expect(policySaves(), 'no policy save is sent').toEqual([]);
    expect(puts(), 'no draft save is sent').toEqual([]);
    expectWordedLine(el, 'buyAdjustPct');
  });
});
