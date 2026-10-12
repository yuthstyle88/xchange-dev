// EX-4b (#1430): posted reversals on A01 buy and sell tiles, drilldown targetMenu boundary, TH/EN parity.
// Written by gemini/warut-4 before the implementation per ticket §9 edge cases.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type {
  DefinitionDto,
  DrilldownDto,
  ExecutiveOverviewData,
  MetricDto,
} from '../../../shared/api/generated/executive_overview';
import { setLang } from '../../../shared/i18n';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { Executive } from './Executive';
import { drilldownHash, targetMenu, TARGETS } from './drilldown';
import * as i18n from './i18n';

const ENDPOINT = '/api/executive/overview';
const FROM = '2026-10-05';
const TO = '2026-10-06';
const VERSIONS = { B1: 1 };

function drill(view: string, filter: Record<string, string> = {}, dropped: string[] = []): DrilldownDto {
  return { view, filter, dropped };
}

function metric(key: string, value: string | null, unavailable: MetricDto['unavailable'] = null, drilldown: DrilldownDto | null = null): MetricDto {
  return { key, value, unavailable, drilldown };
}

const DEFINITIONS: DefinitionDto[] = [
  { key: 'buy_volume_thb', th: 'มูลค่าบาทของรายการซื้อที่สำเร็จ', en: 'THB value of completed buys' },
  { key: 'sell_volume_thb', th: 'มูลค่าบาทของรายการขายที่สำเร็จ', en: 'THB value of completed sells' },
];

function sampleData(over: Partial<ExecutiveOverviewData> = {}): ExecutiveOverviewData {
  return {
    period: { from: FROM, to: TO },
    kpis: [
      metric('buy_volume_thb', '1000.00'),
      metric('sell_volume_thb', '2000.00'),
    ],
    branches: [
      {
        branchId: 'B1',
        name: 'สาขาหนึ่ง',
        volumeThb: '3000.00',
        availableCash: metric('available_cash', '10000.00'),
        inTransit: metric('in_transit', '0.00'),
        shifts: { open: 1, closedVerified: 0, closedUnverified: 0 },
        asOf: '2026-10-06T03:00:00Z',
        pendingEvents: 0,
        drilldown: drill('admin_overview', { branch: 'B1' }),
      },
    ],
    alerts: [],
    definitions: DEFINITIONS,
    buyReversedThb: '0.00',
    sellReversedThb: '0.00',
    ...over,
  };
}

let replyData: ExecutiveOverviewData;

function fakeFetch(input: string | URL | Request): Promise<Response> {
  const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  const url = new URL(raw, 'http://localhost');
  if (url.pathname === '/api/v3/views/me') {
    return Promise.resolve(new Response(JSON.stringify({ version: 1, changed: false }), { status: 200 }));
  }
  if (url.pathname === QUEUE_CARD_PATH) {
    return Promise.resolve(new Response(JSON.stringify(queueCardUnchanged()), { status: 200 }));
  }
  if (url.pathname === ENDPOINT) {
    return Promise.resolve(
      new Response(JSON.stringify({ changed: true, versions: VERSIONS, data: replyData }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );
  }
  return Promise.resolve(new Response(JSON.stringify({ error: { code: 'notFound', message: 'not found' } }), { status: 404 }));
}

let dispose: (() => void) | undefined;
let root: HTMLElement;

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

function mount(): HTMLElement {
  const [branch] = createSignal('B1');
  const [period] = createSignal({ from: FROM, to: TO });
  const [shown] = createSignal(true);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(() => (<Executive branch={branch} period={period} shown={shown} />) as JSX.Element, root);
  return root;
}

function tile(el: HTMLElement, key: string): HTMLElement {
  const found = el.querySelector<HTMLElement>(`[data-kpi="${key}"]`);
  expect(found, `tile [data-kpi="${key}"] exists`).not.toBeNull();
  return found!;
}

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

beforeEach(() => {
  replyData = sampleData();
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  setLang('th');
});

afterEach(async () => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.unstubAllGlobals();
  setLang('th');
  await settle();
});

describe('EX-4b: Section 9 Edge Cases (#1430)', () => {
  // Row 1: No reversals → both show "0.00"
  it('row 1: no reversals → both buy and sell tiles show "0.00" and are not hidden', async () => {
    replyData = sampleData({ buyReversedThb: '0.00', sellReversedThb: '0.00' });
    const el = mount();
    await settle();

    const buyTile = tile(el, 'buy_volume_thb');
    const sellTile = tile(el, 'sell_volume_thb');

    // In Thai
    expect(text(buyTile)).toContain('คืนเงินแล้ว');
    expect(text(buyTile)).toContain('0.00');
    expect(text(sellTile)).toContain('คืนเงินแล้ว');
    expect(text(sellTile)).toContain('0.00');

    // Neither tile hides the "0.00" reversal figure
    const buyReversedEl = buyTile.querySelector('.executive-kpi-reversed');
    const sellReversedEl = sellTile.querySelector('.executive-kpi-reversed');
    expect(buyReversedEl, 'buy tile has reversal line').not.toBeNull();
    expect(sellReversedEl, 'sell tile has reversal line').not.toBeNull();
    expect(text(buyReversedEl)).toBe('คืนเงินแล้ว: 0.00');
    expect(text(sellReversedEl)).toBe('คืนเงินแล้ว: 0.00');
  });

  // Row 2: A reversal on one side only
  it('row 2: a reversal on one side only shows the value on that side and "0.00" on the other', async () => {
    replyData = sampleData({ buyReversedThb: '500.00', sellReversedThb: '0.00' });
    const el = mount();
    await settle();

    const buyTile = tile(el, 'buy_volume_thb');
    const sellTile = tile(el, 'sell_volume_thb');

    const buyReversedEl = buyTile.querySelector('.executive-kpi-reversed');
    const sellReversedEl = sellTile.querySelector('.executive-kpi-reversed');
    expect(buyReversedEl).not.toBeNull();
    expect(sellReversedEl).not.toBeNull();
    expect(text(buyReversedEl)).toBe('คืนเงินแล้ว: 500.00');
    expect(text(sellReversedEl)).toBe('คืนเงินแล้ว: 0.00');
  });

  // Row 3: The gross figure is unchanged by the reversal: the mutant that subtracts must go red
  it('row 3: gross figure is unchanged by reversal (subtraction mutant goes red)', async () => {
    replyData = sampleData({
      kpis: [
        metric('buy_volume_thb', '1000.00'),
        metric('sell_volume_thb', '2000.00'),
      ],
      buyReversedThb: '300.00',
      sellReversedThb: '400.00',
    });
    const el = mount();
    await settle();

    const buyTile = tile(el, 'buy_volume_thb');
    const sellTile = tile(el, 'sell_volume_thb');

    const buyGross = buyTile.querySelector('.executive-kpi-value');
    const sellGross = sellTile.querySelector('.executive-kpi-value');

    // Gross figures remain 1,000.00 and 2,000.00 — NOT 700.00 or 1,600.00
    expect(text(buyGross)).toBe('1,000.00');
    expect(text(sellGross)).toBe('2,000.00');

    // Reversals are shown separately
    expect(text(buyTile.querySelector('.executive-kpi-reversed'))).toBe('คืนเงินแล้ว: 300.00');
    expect(text(sellTile.querySelector('.executive-kpi-reversed'))).toBe('คืนเงินแล้ว: 400.00');
  });

  // Row 4: An unknown drill-down view → no link
  it('row 4: any view outside the five maps to targetMenu null and renders no drill-down link', async () => {
    // 1. targetMenu boundary: only the five views in §4.4 return a menu; any other view returns null
    const allowed = ['admin_transactions', 'admin_customers', 'approval_requests', 'admin_cash', 'admin_overview'];

    // Pins that TARGETS maps ONLY the five views (EX-4 review gap: a sixth mapping goes red)
    expect(Object.keys(TARGETS).sort()).toEqual(allowed.slice().sort());
    expect(Object.keys(TARGETS).length).toBe(5);
    expect(targetMenu('admin_transactions')).toBe('transactions');
    expect(targetMenu('admin_customers')).toBe('customers');
    expect(targetMenu('approval_requests')).toBe('approvals');
    expect(targetMenu('admin_cash')).toBe('cash');
    expect(targetMenu('admin_overview')).toBe('overview');

    // Views outside the five return null
    expect(targetMenu('cash_counts')).toBeNull();
    expect(targetMenu('unknown_view')).toBeNull();
    expect(targetMenu('admin_reports')).toBeNull();
    expect(targetMenu('admin_users')).toBeNull();
    expect(targetMenu('admin_rates')).toBeNull();
    expect(targetMenu('')).toBeNull();

    expect(drilldownHash(drill('unknown_view', { branch: 'B1' }))).toBeNull();

    // 2. Component rendering: a KPI tile with an unknown drill-down view renders NO drilldown button/link
    replyData = sampleData({
      kpis: [
        metric('buy_volume_thb', '1000.00', null, drill('unknown_view', { branch: 'B1' })),
        metric('sell_volume_thb', '2000.00', null, drill('admin_transactions', { branch: 'B1' })),
      ],
    });
    const el = mount();
    await settle();

    const buyTile = tile(el, 'buy_volume_thb');
    const sellTile = tile(el, 'sell_volume_thb');

    expect(buyTile.querySelector('[data-drilldown]'), 'unknown view renders no link').toBeNull();
    expect(sellTile.querySelector('[data-drilldown]'), 'known view renders link').not.toBeNull();
  });

  // Row 5: TH/EN key parity
  it('row 5: TH/EN key parity and correct label translation for reversed', async () => {
    const thKeys = Object.keys(i18n.th).sort();
    const enKeys = Object.keys(i18n.en).sort();
    expect(thKeys).toEqual(enKeys);

    // Both include 'reversed'
    expect(thKeys).toContain('reversed');
    expect(enKeys).toContain('reversed');

    expect((i18n.th as Record<string, string>)['reversed']).toBe('คืนเงินแล้ว');
    expect((i18n.en as Record<string, string>)['reversed']).toBe('Reversed');

    // All strings are non-empty
    for (const key of thKeys) {
      expect((i18n.th as Record<string, string>)[key]?.length, `th.${key} is non-empty`).toBeGreaterThan(0);
      expect((i18n.en as Record<string, string>)[key]?.length, `en.${key} is non-empty`).toBeGreaterThan(0);
    }

    // Verify EN rendering in UI
    setLang('en');
    replyData = sampleData({ buyReversedThb: '120.00', sellReversedThb: '0.00' });
    const el = mount();
    await settle();

    const buyTile = tile(el, 'buy_volume_thb');
    const buyReversedEl = buyTile.querySelector('.executive-kpi-reversed');
    expect(buyReversedEl).not.toBeNull();
    expect(text(buyReversedEl)).toBe('Reversed: 120.00');
  });
});
