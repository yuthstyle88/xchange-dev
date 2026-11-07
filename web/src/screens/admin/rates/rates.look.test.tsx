// OL-6 wave B admin-rates (audit #564 issuecomment-6702970101, shot 023): three things sat on the card edge or off it.
// - The policy card's "กรอบจำกัดการปรับเรทตามสกุลเงิน (%)" heading was an <h3> inside `.table-scroll`, flush on the card's
//   left edge; it now sits above the scroller as `h3.rates-subhead` on the card's 24px inset.
// - The published-rates card's freshness line ("อัตราอ้างอิง ธปท. …", shared RateFreshness) touched the card edge; the
//   card now insets it 22px like its heading.
// - "เรทหน้าร้านจาก เซิร์ฟเวอร์ · THB ต่อ 1 หน่วยเงินต่างประเทศ" floated on the grey page under the DRAFT card; it describes
//   the published rates, so it now closes the published-rates card (`p.rates-live-note`).
// OL-7 (verification of 2026-11-07, row 48; re-checked on feat/one-look-7 06b1e00b): while a draft is editable each
//   buy/sell cell holds a mode select, a manual-price input and its preview inline, every control at the admin's
//   width:100%, so the draft table measured 1138px in a 954px card at 1280 and cut the prices ("35.") and the reason
//   column. The edit controls now stack in their cell at a fixed 9rem, and the reason input keeps 12rem.
// Written before the fix by claude/bbe9ce8b for the OL sub-controller claude/1cc51615 (OL-6, carried to OL-7). Do not edit, delete or weaken.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
  makeRatePolicyData,
  makeRatePolicyVersionDto,
  makeRatesReferenceData,
  makeRatesReferenceRow,
} from '../../../shared/testing/fixtures';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { menu } from './menu';
import { th } from './i18n';

const line = (currency: string, buy: string, sell: string) =>
  makePublishedLineDto({ currency, group: 'ALL', unitSize: '1', buyRate: buy, sellRate: sell, lineId: `line-${currency}`, bands: [] });

const PUBLISHED = makePublishedRatesData({
  publication: makePublishedPublicationDto({ id: 'pub-7', version: 7, publishedAt: '2026-10-09T02:00:00+00:00' }),
  sourceDate: '2026-10-08',
  stale: false,
  lines: [line('USD', '33.25000000', '34.10000000'), line('EUR', '37.35520000', '38.08440000')],
  bandHeadings: [],
});

const POLICY = makeRatePolicyData({
  policy: makeRatePolicyVersionDto({
    id: 'pol-1',
    branchId: 'MAIN',
    version: 1,
    buyAdjustPct: '0.0000',
    sellAdjustPct: '0.0000',
    guards: { USD: { buy: { min_pct: '-1.0000', max_pct: '1.0000' }, sell: { min_pct: '-1.0000', max_pct: '1.0000' } } },
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

const REFERENCE = makeRatesReferenceData({
  rows: [makeRatesReferenceRow({ code: 'USD', date: '2026-10-08', unit: 1, buyingSight: '33.4218', buyingTransfer: '33.5008', selling: '33.8278' })],
  fetchedAt: '2026-10-09T01:30:00.000Z',
  error: null,
});

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const path = new URL(input, 'http://localhost').pathname;
  const method = (init?.method ?? 'GET').toUpperCase();
  const ok = (data: unknown) => json(200, { version: 1, changed: true, data });
  if (method !== 'GET') return json(500, { error: { code: 'serverError', message: 'no writes in this test' } });
  switch (path) {
    case '/api/v3/views/me':
      return ok({ kind: 'owner', id: 'owner-1', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner', permissions: ['RatePolicySave'] });
    case QUEUE_CARD_PATH:
      return json(200, queueCardUnchanged());
    case '/api/v3/views/published_rates':
      return ok(PUBLISHED);
    case '/api/v3/views/rate_drafts':
      return ok({ drafts: [] });
    case '/api/v3/views/rate_draft':
      return ok({ draft: null, lines: [] });
    case '/api/v3/views/rate_policy':
      return ok(POLICY);
    case '/api/v3/views/rate_publications':
      return ok({ publications: [] });
    case '/api/v3/views/rates_reference':
      return ok(REFERENCE);
    default:
      return ok(null);
  }
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

async function mount(): Promise<HTMLElement> {
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
  await settle();
  return el;
}

beforeEach(() => {
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
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

const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();

describe('admin-rates: headings and notes sit inside their cards', () => {
  it('RT-1 the limits heading is h3.rates-subhead, outside any .table-scroll, right before the limits scroller', async () => {
    const el = await mount();
    const heads = [...el.querySelectorAll('h3')].filter((h) => text(h) === th.guardsTitle);
    expect(heads, 'one limits heading').toHaveLength(1);
    const h = heads[0]!;
    expect(h.classList.contains('rates-subhead')).toBe(true);
    expect(h.closest('.table-scroll'), 'not inside the scroller').toBeNull();
    const next = h.nextElementSibling;
    expect(next?.classList.contains('table-scroll'), 'the scroller follows the heading').toBe(true);
    expect(next?.querySelector('tr[data-guard-line="USD"]'), 'it is the limits table').not.toBeNull();
  });

  it('RT-2 the live-rates note closes the published-rates card and appears once', async () => {
    const el = await mount();
    const notes = [...el.querySelectorAll('p')].filter((p) => text(p) === th.ratesLiveNote);
    expect(notes, 'one live-rates note').toHaveLength(1);
    const note = notes[0]!;
    expect(note.classList.contains('rates-live-note')).toBe(true);
    const card = note.closest('.rates-section');
    expect(card, 'inside a .rates-section').not.toBeNull();
    expect(card!.querySelector('h2') && text(card!.querySelector('h2')), 'the published-rates card').toBe(th.ratesTitle);
    expect(card!.lastElementChild, 'the last thing in the card').toBe(note);
    expect(note.closest('.section-note'), 'no longer the loose .section-note').toBeNull();
  });

  it('RT-3 the freshness line is a direct child of the published-rates card', async () => {
    const el = await mount();
    const fresh = el.querySelector('.xc-rate-freshness');
    expect(fresh).not.toBeNull();
    expect(fresh!.parentElement?.classList.contains('rates-section')).toBe(true);
  });
});

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    if (m[1]!.trim() !== selector) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return out;
  }
  return undefined;
}

describe('admin-rates: the insets', () => {
  let ratesCss = '';
  let policyCss = '';
  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    // resolve next to this file: CI runs vitest with --root web from the repo root
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    ratesCss = fs.readFileSync(here + 'rates.css', 'utf8');
    policyCss = fs.readFileSync(here + 'rates-policy.css', 'utf8');
  });

  it('RT-4 rates-policy.css: .rates-subhead margin 18px 24px 10px, 16px', () => {
    const d = decls(policyCss, '.xc-admin .xc-content .admin-rates-screen .rates-subhead');
    expect(d, 'rule `.xc-admin .xc-content .admin-rates-screen .rates-subhead`').toBeDefined();
    expect(d!.get('margin')).toBe('18px 24px 10px');
    expect(d!.get('font-size')).toBe('16px');
  });

  it('RT-5 rates.css: the freshness line in a rates card has margin 0 and padding 0 22px 12px', () => {
    const d = decls(ratesCss, '.xc-admin .xc-content .admin-rates-screen .rates-section > .xc-rate-freshness');
    expect(d, 'rule `… .rates-section > .xc-rate-freshness`').toBeDefined();
    expect(d!.get('margin')).toBe('0');
    expect(d!.get('padding')).toBe('0 22px 12px');
  });

  it('RT-6 rates.css: .rates-live-note margin 0, padding 12px 22px 18px, 13px muted', () => {
    const d = decls(ratesCss, '.xc-admin .xc-content .admin-rates-screen .rates-live-note');
    expect(d, 'rule `… .rates-live-note`').toBeDefined();
    expect(d!.get('margin')).toBe('0');
    expect(d!.get('padding')).toBe('12px 22px 18px');
    expect(d!.get('font-size')).toBe('13px');
    expect(d!.get('color')).toBe('var(--xc-muted)');
  });
});

describe('admin-rates OL-7: the editable draft cells fit their card', () => {
  let draftCss = '';
  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    draftCss = fs.readFileSync(here + 'rates-draft.css', 'utf8');
  });

  it('RT-7 rates-draft.css: a rate cell wraps, and its select and input stack at 9rem on the right', () => {
    const cell = decls(draftCss, '.xc-admin .xc-content.xc-content .admin-rates-screen .rates-table td.rate-number');
    expect(cell, 'rule `… .rates-table td.rate-number`').toBeDefined();
    expect(cell!.get('white-space')).toBe('normal');
    const ctl = decls(draftCss, '.xc-admin .xc-content.xc-content .admin-rates-screen .rates-table td.rate-number :is(select, input)');
    expect(ctl, 'rule `… td.rate-number :is(select, input)`').toBeDefined();
    expect(ctl!.get('display')).toBe('block');
    expect(ctl!.get('width')).toBe('9rem');
    expect(ctl!.get('margin-left')).toBe('auto');
  });

  it('RT-8 rates-draft.css: the reason input keeps 12rem', () => {
    const d = decls(draftCss, '.xc-admin .xc-content.xc-content .admin-rates-screen .rates-table td input[name="reason"]');
    expect(d, 'rule `… td input[name="reason"]`').toBeDefined();
    expect(d!.get('min-width')).toBe('12rem');
  });
});
