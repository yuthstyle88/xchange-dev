import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { AlertDto, ExecutiveOverviewData } from '../../../shared/api/generated/executive_overview';
import { setLang } from '../../../shared/i18n';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { Executive } from './Executive';
import * as table from './i18n';

const ENDPOINT = '/api/v3/views/executive_overview';
const FROM = '2026-10-05';
const TO = '2026-10-06';

let alertsFixture: AlertDto[];
let dispose: (() => void) | undefined;
let root: HTMLElement;

function overview(alerts: AlertDto[]): ExecutiveOverviewData {
  return {
    period: { from: FROM, to: TO },
    kpis: [],
    branches: [
      {
        branchId: 'B1',
        name: 'สาขาหนึ่ง',
        volumeThb: '1000.00',
        availableCash: { key: 'available_cash', value: '50000.00', unavailable: null, drilldown: null },
        inTransit: { key: 'in_transit', value: '0.00', unavailable: null, drilldown: null },
        shifts: { open: 1, closedVerified: 0, closedUnverified: 0 },
        asOf: '2026-10-06T03:00:00Z',
        pendingEvents: 0,
        drilldown: { view: 'admin_overview', filter: { branch: 'B1', from: FROM, to: TO }, dropped: [] },
      },
    ],
    alerts,
    definitions: [],
    buyReversedThb: '0.00',
    sellReversedThb: '0.00',
  };
}

function fakeFetch(input: string | URL | Request): Promise<Response> {
  const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  const url = new URL(raw, 'http://localhost');
  if (url.pathname === QUEUE_CARD_PATH) {
    return Promise.resolve(
      new Response(JSON.stringify(queueCardUnchanged()), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );
  }
  if (url.pathname === ENDPOINT) {
    return Promise.resolve(
      new Response(
        JSON.stringify({ changed: true, versions: { B1: 1 }, data: overview(alertsFixture) }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );
  }
  return Promise.resolve(new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } }));
}

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function mount(): HTMLElement {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: FROM, to: TO });
  const [shown] = createSignal(true);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(() => (<Executive branch={branch} period={period} shown={shown} />) as JSX.Element, root);
  return root;
}

function kindText(el: HTMLElement, kind: string): string {
  const item = el.querySelector<HTMLElement>(`[data-alert-kind="${kind}"]`);
  expect(item, `alert [data-alert-kind="${kind}"] exists`).not.toBeNull();
  return (item!.querySelector('.executive-alert-kind')?.textContent ?? '').trim();
}

beforeEach(() => {
  alertsFixture = [];
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  location.hash = '#executive';
  setLang('th');
});

afterEach(async () => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
  await settle();
});

describe('A01 executive low-stock alert label (#3319)', () => {
  it('row 1: a lowStock alert in Thai reads สต็อกต่ำ, never lowStock', async () => {
    alertsFixture = [
      {
        kind: 'lowStock',
        branchId: 'B1',
        since: '2026-10-06T02:00:00Z',
        detail: 'USD',
        drilldown: { view: 'admin_cash', filter: { branch: 'B1', currency: 'USD' }, dropped: [] },
      },
    ];
    const el = mount();
    await settle();
    expect(table.th.alertLowStock).toBe('สต็อกต่ำ');
    expect(kindText(el, 'lowStock')).toBe('สต็อกต่ำ');
    expect(kindText(el, 'lowStock')).not.toBe('lowStock');
  });

  it('row 2: a lowStock alert in English reads Low stock', async () => {
    setLang('en');
    alertsFixture = [
      {
        kind: 'lowStock',
        branchId: 'B1',
        since: '2026-10-06T02:00:00Z',
        detail: 'USD',
        drilldown: null,
      },
    ];
    const el = mount();
    await settle();
    expect(table.en.alertLowStock).toBe('Low stock');
    expect(kindText(el, 'lowStock')).toBe('Low stock');
  });

  it('row 3: ratePending, staleData and countDifference labels remain unchanged', async () => {
    alertsFixture = [
      { kind: 'ratePending', branchId: 'B1', since: null, detail: 'RQ-1', drilldown: null },
      { kind: 'staleData', branchId: 'B1', since: null, detail: 'TX-1', drilldown: null },
      { kind: 'countDifference', branchId: 'B1', since: null, detail: 'EXC-1', drilldown: null },
    ];
    const el = mount();
    await settle();
    expect(kindText(el, 'ratePending')).toBe('เรทรออนุมัติ');
    expect(kindText(el, 'staleData')).toBe('ข้อมูลค้าง');
    expect(kindText(el, 'countDifference')).toBe('เงินนับไม่ตรง');
  });

  it('row 4: an unknown alert kind falls through to its raw key', async () => {
    alertsFixture = [
      {
        kind: 'futureKind' as unknown as AlertDto['kind'],
        branchId: 'B1',
        since: null,
        detail: 'X-1',
        drilldown: null,
      },
    ];
    const el = mount();
    await settle();
    expect(kindText(el, 'futureKind')).toBe('futureKind');
  });

  it("row 5: a low-stock alert's drill-down navigates to its target menu and filter unchanged", async () => {
    alertsFixture = [
      {
        kind: 'lowStock',
        branchId: 'B1',
        since: '2026-10-06T02:00:00Z',
        detail: 'USD',
        drilldown: { view: 'admin_cash', filter: { branch: 'B1', currency: 'USD' }, dropped: [] },
      },
    ];
    const el = mount();
    await settle();
    const item = el.querySelector<HTMLElement>('[data-alert-kind="lowStock"]')!;
    const btn = item.querySelector<HTMLElement>('[data-drilldown]');
    expect(btn).not.toBeNull();
    btn!.click();
    await settle();
    expect(location.hash).toBe('#cash?branch=B1&currency=USD');
  });
});
