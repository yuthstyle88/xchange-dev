import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { PublishedLineDto, PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { RateDraftDetailData, RateDraftLineDto } from '../../../shared/api/generated/rate_draft';
import type { RateDraftItemDto } from '../../../shared/api/generated/rate_drafts';
import type { RatePolicyData } from '../../../shared/api/generated/rate_policy';
import type { RatePublicationRow } from '../../../shared/api/generated/rate_publications';

// #3399 renamed the export; the list rows are the publication without its lines.
type RatePublicationItemDto = Omit<RatePublicationRow, 'lines'>;
import type { RatesReferenceData, RatesReferenceRow } from '../../../shared/api/generated/rates_reference';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { menu } from './menu';
import * as i18n from './i18n';

const V = '/api/v3/views/';
const PUBLISHED = `${V}published_rates`;
const DRAFTS = `${V}rate_drafts`;
const DRAFT = `${V}rate_draft`;
const POLICY = `${V}rate_policy`;
const PUBLICATIONS = `${V}rate_publications`;
const REFERENCE = `${V}rates_reference`;

function say(key: keyof typeof i18n.th): string {
  return i18n.th[key];
}

function line(currency: string, unitSize: string, buy: string | null, sell: string | null): PublishedLineDto {
  return { currency, group: 'ALL', unitSize, buyRate: buy, sellRate: sell, lineId: `line-${currency}`, bands: [] };
}

function publishedOf(version: number | null, lines: PublishedLineDto[]): PublishedRatesData {
  return {
    publication:
      version === null ? null : { id: `pub-${version}`, version, publishedAt: '2026-10-09T02:00:00+00:00' },
    sourceDate: '2026-10-08',
    stale: false,
    lines,
    bandHeadings: [],
  };
}

function mainPublished(): PublishedRatesData {
  return publishedOf(7, [
    line('EUR', '1', '37.35520000', '38.08440000'),
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

function policyOf(): RatePolicyData {
  return {
    policy: {
      id: 'pol-1',
      branchId: 'MAIN',
      version: 1,
      buyAdjustPct: '0.0000',
      sellAdjustPct: '0.0000',
      guards: {},
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

function referenceOf(): RatesReferenceData {
  return {
    rows: [refRow('USD', 1, '33.4218', '33.5008', '33.8278')],
    fetchedAt: '2026-10-09T01:30:00.000Z',
    error: null,
  };
}

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
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
let putDelay: Promise<void> | null;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function viewAnswer<T>(version: number, data: T, since: string | null): Reply {
  if (since !== null && Number(since) === version) return { status: 200, body: { version, changed: false } };
  return { status: 200, body: { version, changed: true, data: structuredClone(data) } };
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
        data: { kind: 'owner', id: 'owner-1', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' },
      },
    };
  }
  if (path === QUEUE_CARD_PATH) return { status: 200, body: queueCardUnchanged() };
  if (method === 'GET') {
    if (path === PUBLISHED) {
      const e = published[branch];
      return e ? viewAnswer(e.version, e.data, since) : viewAnswer(1, publishedOf(null, []), since);
    }
    if (path === DRAFTS) return viewAnswer(100, { drafts: drafts[branch] ?? [] }, since);
    if (path === DRAFT) {
      const d = details[p.get('id') ?? ''];
      return viewAnswer(200, d ?? { draft: null, lines: [] }, since);
    }
    if (path === POLICY) return viewAnswer(1, policies[branch] ?? { policy: null }, since);
    if (path === PUBLICATIONS) return viewAnswer(1, { publications: publications[branch] ?? [] }, since);
    if (path === REFERENCE) return viewAnswer(3, reference, since);
  }
  const m = /^\/api\/rate-drafts\/([^/]+)$/.exec(path);
  if (m && method === 'PUT') {
    if (putDelay) await putDelay;
    const id = m[1]!;
    const d = details[id];
    if (!d?.draft) return { status: 404, body: { error: { code: 'notFound', message: 'not found' } } };
    d.draft.version += 1;
    d.lines = linesOfBody(body);
    return { status: 200, body: { id, version: d.draft.version, status: 'DRAFT' } };
  }
  return { status: 500, body: { error: { code: 'serverError', message: `unexpected ${method} ${path}` } } };
}

async function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = typeof init?.body === 'string' && init.body !== '' ? JSON.parse(init.body) : undefined;
  const s: Sent = { path: url.pathname, params: url.searchParams, method, body };
  sent.push(s);
  const reply = await answer(s);
  return json(reply.status, reply.body);
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

function mount(branchName = 'MAIN'): { el: HTMLElement } {
  const m = menu as AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
  const [branch] = createSignal(branchName);
  const [period] = createSignal({ from: '2026-10-09', to: '2026-10-09' });
  const [shown] = createSignal(true);
  const Component = m.Component;
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
  return { el };
}

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function puts(): Sent[] {
  return sent.filter((w) => w.method === 'PUT' && w.path.startsWith('/api/rate-drafts/'));
}

beforeEach(() => {
  sent = [];
  putDelay = null;
  const d1 = draftItem('d1', 'DRAFT', 3, '2026-10-09T04:00:00+00:00');
  drafts = { MAIN: [d1] };
  details = { d1: detailOf(d1, [auto('EUR'), usdManualSell()]) };
  published = { MAIN: { version: 7, data: mainPublished() } };
  policies = { MAIN: policyOf() };
  publications = { MAIN: [] };
  reference = referenceOf();
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

describe('RatesManualPrice extra coverage (#3356)', () => {
  it('sends an all-AUTO line with its typed reason when a reason is provided', async () => {
    const { el } = mount();
    await settle();
    const eur = el.querySelector<HTMLElement>('[data-draft-line="EUR"]')!;
    const reasonInput = eur.querySelector<HTMLInputElement>('input[name="reason"]')!;
    reasonInput.value = 'audit note';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));
    const saveBtn = [...el.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => text(b) === say('saveLines'),
    )!;
    saveBtn.click();
    await settle();

    expect(puts().length).toBe(1);
    const lines = (puts()[0]!.body?.lines ?? []) as Array<Record<string, unknown>>;
    const eurSent = lines.find((l) => l.currency === 'EUR');
    expect(eurSent).toEqual({
      currency: 'EUR',
      buyMode: 'AUTO',
      sellMode: 'AUTO',
      manualBuy: null,
      manualSell: null,
      reason: 'audit note',
    });
  });

  it('saving twice fast sends only one PUT and disables the save button while in flight', async () => {
    let resolvePut!: () => void;
    putDelay = new Promise<void>((r) => {
      resolvePut = r;
    });
    const { el } = mount();
    await settle();
    const saveBtn = [...el.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => text(b) === say('saveLines'),
    )!;
    saveBtn.click();
    await Promise.resolve();
    expect(saveBtn.disabled).toBe(true);
    saveBtn.click();
    resolvePut();
    await settle();

    expect(puts().length).toBe(1);
    expect(saveBtn.disabled).toBe(false);
  });

  it('renders no mode selects, manual/reason inputs, or saveLines button when the draft status is not DRAFT', async () => {
    const dApproved = draftItem('d1', 'APPROVED', 3, '2026-10-09T04:00:00+00:00');
    drafts.MAIN = [dApproved];
    details.d1 = detailOf(dApproved, [auto('EUR'), usdManualSell()]);

    const { el } = mount();
    await settle();

    expect(el.querySelectorAll('select[name="buyMode"]').length).toBe(0);
    expect(el.querySelectorAll('select[name="sellMode"]').length).toBe(0);
    expect(el.querySelectorAll('input[name="manualBuy"]').length).toBe(0);
    expect(el.querySelectorAll('input[name="manualSell"]').length).toBe(0);
    expect(el.querySelectorAll('input[name="reason"]').length).toBe(0);
    const saveBtns = [...el.querySelectorAll<HTMLButtonElement>('button')].filter(
      (b) => text(b) === say('saveLines'),
    );
    expect(saveBtns.length).toBe(0);
  });

  it('renders each adjust-all field (adjustBuyPct, adjustSellPct) inside label.form-field with a .form-label', async () => {
    const { el } = mount();
    await settle();
    const bar = el.querySelector('.adjust-all-bar.adjust-all-bar--draft');
    expect(bar).not.toBeNull();
    const fields = bar!.querySelectorAll<HTMLLabelElement>('label.form-field.adjust-field-label');
    expect(fields.length).toBe(2);
    expect(text(fields[0]!.querySelector('.form-label'))).toBe(say('adjustBuyPct'));
    expect(text(fields[1]!.querySelector('.form-label'))).toBe(say('adjustSellPct'));
  });
});
