// #2338: Admin Executive (A01) — poll race while a filter, period, or re-show load is in flight (§8, §9).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type {
  AlertDto,
  BranchRowDto,
  DefinitionDto,
  DrilldownDto,
  ExecutiveOverviewData,
  MetricDto,
} from '../../../shared/api/generated/executive_overview';
import { errorText, setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { PAGE_WIDE, QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { menu } from './menu';

const ENDPOINT = '/api/v3/views/executive_overview';

const FROM = '2026-10-05';
const TO = '2026-10-06';

const DEFINITIONS: DefinitionDto[] = [
  { key: 'buy_volume_thb', th: 'มูลค่าบาทของรายการซื้อที่สำเร็จ ไม่ใช่รายได้', en: 'THB value of completed buys; not revenue' },
  { key: 'sell_volume_thb', th: 'มูลค่าบาทของรายการขายที่สำเร็จ', en: 'THB value of completed sells' },
  { key: 'realized_margin_thb', th: 'เงินบาทที่ได้รับลบต้นทุนที่ปันส่วน', en: 'THB received less allocated cost' },
  { key: 'repeat_customers', th: 'ลูกค้าที่เคยมีรายการสำเร็จมาก่อน', en: 'Customers with an earlier completed transaction' },
  { key: 'booking_conversion', th: 'การจองที่สำเร็จหารด้วยการจองที่ยืนยัน', en: 'Completed bookings over confirmed bookings' },
  { key: 'commission_thb', th: 'ค่าคอมมิชชันของช่วงเวลา', en: 'Commission expense of the period' },
];

function drill(view: string, filter: Record<string, string>, dropped: string[] = []): DrilldownDto {
  return { view, filter, dropped };
}

function metric(
  key: string,
  value: string | null,
  unavailable: MetricDto['unavailable'] = null,
  drilldown: DrilldownDto | null = null,
): MetricDto {
  return { key, value, unavailable, drilldown };
}

function kpis(buyThb = '1234567.89', sellThb = '765.40'): MetricDto[] {
  return [
    metric('buy_volume_thb', buyThb, null, drill('admin_transactions', { from: FROM, to: TO, type: 'BUY' }, ['currency', 'channel'])),
    metric('sell_volume_thb', sellThb, null, drill('admin_transactions', { from: FROM, to: TO, type: 'SELL' })),
    metric('realized_margin_thb', null, 'awaitsLedger'),
    metric('repeat_customers', '12', null, drill('admin_customers', {}, ['repeat', 'from', 'to'])),
    metric('booking_conversion', null, 'noBookingsInPeriod' as unknown as MetricDto['unavailable']),
    metric('commission_thb', null, 'awaitsBroker'),
  ];
}

function branchRow(id: string, over: Partial<BranchRowDto> = {}): BranchRowDto {
  return {
    branchId: id,
    name: id === 'B1' ? 'สาขาหนึ่ง' : 'สาขาสอง',
    volumeThb: id === 'B1' ? '2000.50' : '3100.25',
    availableCash: metric('available_cash', id === 'B1' ? '150000.00' : '98765.43'),
    inTransit: metric('in_transit', id === 'B1' ? '5000.00' : '4321.09'),
    shifts: { open: 1, closedVerified: 2, closedUnverified: 3 },
    asOf: id === 'B1' ? '2026-10-06T03:00:00Z' : '2026-10-06T01:00:00Z',
    pendingEvents: 0,
    drilldown: drill('admin_overview', { branch: id, from: FROM, to: TO }, ['currency', 'channel']),
    ...over,
  };
}

function alerts(): AlertDto[] {
  return [
    {
      kind: 'ratePending',
      branchId: 'B1',
      since: '2026-10-06T02:00:00Z',
      detail: 'RQ-77',
      drilldown: drill('approval_requests', { branch: 'B1', type: 'RATE_PUBLICATION' }),
    },
  ];
}

function data(over: Partial<ExecutiveOverviewData> = {}): ExecutiveOverviewData {
  return {
    period: { from: FROM, to: TO },
    kpis: kpis(),
    branches: [branchRow('B1'), branchRow('B2')],
    alerts: alerts(),
    definitions: DEFINITIONS,
    buyReversedThb: '0.00',
    sellReversedThb: '0.00',
    ...over,
  };
}

const VERSIONS = { B1: 4, B2: 9 };

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
  credentials: RequestCredentials | undefined;
}

type Reply = { status: number; body: unknown };

let sent: Sent[];
let reply: (params: URLSearchParams, index: number) => Reply | Promise<Reply>;

function answered(versions: Record<string, number>, d: ExecutiveOverviewData): Reply {
  return { status: 200, body: { changed: true, versions, data: d } };
}

function unchanged(versions: Record<string, number>): Reply {
  return { status: 200, body: { changed: false, versions } };
}

function refused(status: number, code: string, message: string): Reply {
  return { status, body: { error: { code, message } } };
}

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

async function fakeFetch(input: string | URL | Request, init?: RequestInit): Promise<Response> {
  const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  const url = new URL(raw, 'http://localhost');
  const method = (init?.method ?? (input instanceof Request ? input.method : 'GET')).toUpperCase();
  const credentials = init?.credentials ?? (input instanceof Request ? input.credentials : undefined);
  sent.push({ path: url.pathname, params: url.searchParams, method, credentials });
  let r: Reply;
  if (url.pathname === '/api/v3/views/me') {
    r = { status: 200, body: { version: 1, changed: false } };
  } else if (url.pathname === QUEUE_CARD_PATH) {
    r = { status: 200, body: queueCardUnchanged() };
  } else if (url.pathname === ENDPOINT) {
    const idx = reads().length - 1;
    r = await reply(url.searchParams, idx);
  } else {
    r = refused(500, 'serverError', `unexpected ${url.pathname}`);
  }
  return new Response(JSON.stringify(r.body), {
    status: r.status,
    headers: { 'content-type': 'application/json' },
  });
}

function reads(): Sent[] {
  return sent.filter((s) => s.path === ENDPOINT && !PAGE_WIDE.includes(s.path));
}

const realSetTimeout = globalThis.setTimeout;

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

let dispose: (() => void) | undefined;
let root: HTMLElement;
let hidden = false;

interface Mounted {
  el: HTMLElement;
  setPeriod: (p: { from: string; to: string }) => void;
  setShown: (s: boolean) => void;
}

function mount(init: { shown?: boolean } = {}): Mounted {
  const [branch] = createSignal('MAIN');
  const [period, setPeriod] = createSignal({ from: FROM, to: TO });
  const [shown, setShown] = createSignal(init.shown ?? true);
  const Component = menu.Component;
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(() => (<Component branch={branch} period={period} shown={shown} />) as JSX.Element, root);
  return { el: root, setPeriod, setShown };
}

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function tile(el: HTMLElement, key: string): HTMLElement {
  const found = el.querySelectorAll<HTMLElement>(`[data-kpi="${key}"]`);
  expect(found.length, `one KPI tile [data-kpi="${key}"]`).toBe(1);
  return found[0]!;
}

function row(el: HTMLElement, id: string): HTMLElement {
  const found = el.querySelectorAll<HTMLElement>(`[data-branch="${id}"]`);
  expect(found.length, `one branch row [data-branch="${id}"]`).toBe(1);
  return found[0]!;
}

function banner(el: HTMLElement): string | null {
  const b = el.querySelector('.error-banner');
  return b ? text(b) : null;
}

function change(el: HTMLInputElement | HTMLSelectElement): void {
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('EX-4 A01: poll race during in-flight filter or period load (#2338)', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
    sent = [];
    reply = () => answered(VERSIONS, data());
    hidden = false;
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
    vi.stubGlobal('fetch', vi.fn(fakeFetch));
    location.hash = '#executive';
    setLang('th');
  });

  afterEach(async () => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    hidden = false;
    location.hash = '';
    setLang('th');
    await settle();
  });

  it("row 1: currency changed; poll fires before the load answers; poll would answer changed:false -> screen shows the new currency's figures, never the old ones; the poll sends no versions", async () => {
    const usdData = data({
      kpis: kpis('555000.00', '321.00'),
      branches: [branchRow('B1', { volumeThb: '8800.00' }), branchRow('B2', { volumeThb: '4400.00' })],
    });
    const inFlightLoad = deferred<Reply>();

    reply = (params, idx) => {
      if (idx === 0) return answered(VERSIONS, data());
      if (idx === 1) return inFlightLoad.promise;
      // The server compares versions per branch regardless of filter, so a request with versions answers changed:false
      if (params.has('versions')) return unchanged(VERSIONS);
      return answered(VERSIONS, usdData);
    };

    const m = mount();
    await settle();
    expect(reads().length).toBe(1);
    expect(text(tile(m.el, 'buy_volume_thb'))).toContain(formatMoney('1234567.89'));

    // Change currency to USD; its load (idx 1) stays in flight
    const currencyInput = m.el.querySelector<HTMLInputElement>('input[name="currency"]')!;
    currencyInput.value = 'USD';
    change(currencyInput);
    await settle();
    expect(reads().length).toBe(2);
    expect(reads()[1]!.params.get('currency')).toBe('USD');
    expect(reads()[1]!.params.has('versions')).toBe(false);

    // 60-second poll fires while the currency load is still in flight
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(reads().length).toBe(3);

    // Now the earlier in-flight load also finishes
    inFlightLoad.resolve(answered(VERSIONS, usdData));
    await settle();

    expect(reads()[2]!.params.get('currency')).toBe('USD');
    expect(reads()[2]!.params.has('versions'), 'the poll sends no versions while load has not answered').toBe(false);
    expect(text(tile(m.el, 'buy_volume_thb'))).toContain(formatMoney('555000.00'));
    expect(text(tile(m.el, 'buy_volume_thb'))).not.toContain(formatMoney('1234567.89'));
    expect(text(row(m.el, 'B1'))).toContain(formatMoney('8800.00'));
    expect(text(row(m.el, 'B1'))).not.toContain(formatMoney('2000.50'));
  });

  it("row 2: period changed, same race -> new period's figures shown", async () => {
    const newPeriod = { from: '2026-09-01', to: '2026-09-30' };
    const septData = data({
      period: newPeriod,
      kpis: kpis('910000.00', '410.25'),
      branches: [branchRow('B1', { volumeThb: '7200.00' }), branchRow('B2', { volumeThb: '6100.00' })],
    });
    const inFlightLoad = deferred<Reply>();

    reply = (params, idx) => {
      if (idx === 0) return answered(VERSIONS, data());
      if (idx === 1) return inFlightLoad.promise;
      if (params.has('versions')) return unchanged(VERSIONS);
      return answered(VERSIONS, septData);
    };

    const m = mount();
    await settle();
    expect(reads().length).toBe(1);
    expect(text(tile(m.el, 'buy_volume_thb'))).toContain(formatMoney('1234567.89'));

    // Change period; its load (idx 1) stays in flight
    m.setPeriod(newPeriod);
    await settle();
    expect(reads().length).toBe(2);
    expect(reads()[1]!.params.get('from')).toBe('2026-09-01');
    expect(reads()[1]!.params.get('to')).toBe('2026-09-30');
    expect(reads()[1]!.params.has('versions')).toBe(false);

    // 60-second poll fires before the period load answers
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(reads().length).toBe(3);

    inFlightLoad.resolve(answered(VERSIONS, septData));
    await settle();

    expect(reads()[2]!.params.get('from')).toBe('2026-09-01');
    expect(reads()[2]!.params.get('to')).toBe('2026-09-30');
    expect(reads()[2]!.params.has('versions')).toBe(false);
    expect(text(tile(m.el, 'buy_volume_thb'))).toContain(formatMoney('910000.00'));
    expect(text(tile(m.el, 'buy_volume_thb'))).not.toContain(formatMoney('1234567.89'));
    expect(text(row(m.el, 'B1'))).toContain(formatMoney('7200.00'));
  });

  it('edge case 1: hidden tab or hidden menu sends nothing on poll; when shown again and poll races the load, new figures show with no versions', async () => {
    const refreshedData = data({
      kpis: kpis('777000.00', '888.00'),
      branches: [branchRow('B1', { volumeThb: '6500.00' }), branchRow('B2', { volumeThb: '3100.25' })],
    });
    const inFlightLoad = deferred<Reply>();

    reply = (params, idx) => {
      if (idx === 0) return answered(VERSIONS, data());
      if (idx === 1) return inFlightLoad.promise;
      if (params.has('versions')) return unchanged(VERSIONS);
      return answered(VERSIONS, refreshedData);
    };

    const m = mount();
    await settle();
    expect(reads().length).toBe(1);

    // Hidden tab and hidden menu: poll sends nothing
    hidden = true;
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(reads().length).toBe(1);

    hidden = false;
    m.setShown(false);
    await settle();
    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(reads().length).toBe(1);

    // Menu shown again -> triggers load (idx 1, held in flight); 60s poll fires before it answers
    m.setShown(true);
    await settle();
    expect(reads().length).toBe(2);

    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(reads().length).toBe(3);

    inFlightLoad.resolve(answered(VERSIONS, refreshedData));
    await settle();

    expect(reads()[2]!.params.has('versions')).toBe(false);
    expect(text(tile(m.el, 'buy_volume_thb'))).toContain(formatMoney('777000.00'));
    expect(text(tile(m.el, 'buy_volume_thb'))).not.toContain(formatMoney('1234567.89'));
  });

  it('edge case 2: a refusal on the load still clears and shows the banner even if the 60s poll fires while the load is in flight', async () => {
    const inFlightLoad = deferred<Reply>();

    reply = (params, idx) => {
      if (idx === 0) return answered(VERSIONS, data());
      if (idx === 1) return inFlightLoad.promise;
      if (params.has('versions')) return unchanged(VERSIONS);
      return refused(403, 'outOfScope', 'scope changed');
    };

    const m = mount();
    await settle();
    expect(reads().length).toBe(1);
    expect(m.el.querySelectorAll('[data-kpi]').length).toBe(6);

    // Filter change triggers a load that will be refused, held in flight when the 60s poll fires
    const select = m.el.querySelector<HTMLSelectElement>('select[name="channel"]')!;
    select.value = 'Booking';
    change(select);
    await settle();
    expect(reads().length).toBe(2);

    await vi.advanceTimersByTimeAsync(60_000);
    await settle();
    expect(reads().length).toBe(3);

    inFlightLoad.resolve(refused(403, 'outOfScope', 'scope changed'));
    await settle();

    expect(banner(m.el)).toBe(errorText('outOfScope', 'scope changed'));
    expect(m.el.querySelectorAll('[data-kpi]').length).toBe(0);
    expect(m.el.querySelectorAll('[data-branch]').length).toBe(0);
    expect(m.el.querySelectorAll('[data-alert-kind]').length).toBe(0);
  });
});
