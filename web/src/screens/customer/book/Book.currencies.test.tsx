// FE-T2c (#5907, rerun gap G2 on #564): C02 /app/book offers only the currencies the chosen branch publishes, on the
// side the customer picked, the same rule the desk follows since FE-T2b (docs/design/fe-triage-1101.md:86).
// Written before the implementation by the FE sub-controller claude/01TxaRJG. Do not edit, delete or weaken these
// tests; add your own elsewhere.
//
// [required] by these tests:
// - The currency <select name="currency"> lists the codes of the branch's public rates
//   (`/api/public/orgs/<org>/branches/<code>/rates`, `PublishedRatesData.lines`) that have a rate on the chosen side:
//   shopSide BUY reads `buyRate`, SELL reads `sellRate`. Publication order, each code once. No hard-coded list.
// - When the chosen currency is no longer offered (side switch, new branch, new rates), the select moves to the first
//   offered code.
// - With a branch chosen and nothing offered, no option is shown and `[data-hint="no-published-currency"]` says so.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingCreateOutput,
  makePlannerPreviewAnswer,
  makePublishedLineDto,
  makePublishedRatesData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { BookScreen } from './Book';

type Line = ReturnType<typeof makePublishedLineDto>;

const BRANCH_ID = 'XC-B01';
const SLOT_ID = '00000000-0000-4000-8000-000000000001';

const BRANCHES = {
  branches: [{ id: BRANCH_ID, code: BRANCH_ID, name: 'สาขาสยามพารากอน', timezone: 'Asia/Bangkok', bookingEnabled: true }],
};
const SLOTS = {
  version: 1,
  changed: true,
  data: {
    slots: [{ id: SLOT_ID, start: '2026-11-05T03:00:00+00:00', end: '2026-11-05T03:30:00+00:00', available: 3 }],
  },
};

const line = (currency: string, buyRate: string | null, sellRate: string | null): Line =>
  makePublishedLineDto({ currency, buyRate, sellRate, lineId: `line-${currency}` });

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('Customer Book: the currency list is the branch’s published currencies on the chosen side (FE-T2c)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let lines: Line[];
  let bookings: number;
  let fetchStub: (input: string, init?: RequestInit) => Promise<Response>;

  beforeAll(() => warmModules(() => import('./Book')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    window.history.replaceState({}, '', '/app/book');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    lines = [];
    bookings = 0;
    fetchStub = vi.fn(async (input: string, init?: RequestInit): Promise<Response> => {
      const u = new URL(input, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (u.pathname === '/api/public/host') return json(200, { org: 'XC', env: 'test' });
      if (u.pathname === `/api/public/orgs/XC/branches/${BRANCH_ID}/rates`) {
        return json(200, makePublishedRatesData({ lines }));
      }
      if (u.pathname.startsWith('/api/public/orgs/')) return json(200, BRANCHES);
      if (u.pathname === '/api/v3/views/booking_slots') return json(200, SLOTS);
      if (u.pathname === '/api/v3/compute/planner_preview' && method === 'POST') {
        return json(200, makePlannerPreviewAnswer({ thb: '3542.00', rate: '35.42000000', unitSize: '1' }));
      }
      if (u.pathname === '/api/bookings' && method === 'POST') {
        bookings++;
        return json(200, makeBookingCreateOutput({ branchId: BRANCH_ID }));
      }
      return json(404, { error: { code: 'notFound', message: u.pathname } });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    window.history.replaceState({}, '', '/app/book');
    vi.restoreAllMocks();
    setAudience('customer');
    setLang('th');
  });

  const currencySelect = () => host.querySelector<HTMLSelectElement>('select[name="currency"]')!;
  const offered = () => Array.from(currencySelect().options).map((o) => o.value);

  function choose(name: string, value: string): void {
    const sel = host.querySelector<HTMLSelectElement>(`select[name="${name}"]`)!;
    sel.value = value;
    sel.dispatchEvent(new Event('change', { bubbles: true }));
  }

  async function mountWithBranch(): Promise<void> {
    dispose = render(() => <BookScreen fetch={fetchStub} ratesIntervalMs={60_000} />, host);
    await flush();
    await vi.waitFor(() => expect(host.querySelector(`option[value="${BRANCH_ID}"]`)).not.toBeNull());
    choose('branchId', BRANCH_ID);
    await flush();
  }

  it('C1 the branch publishes only USD: USD is the only currency offered (no EUR, JPY, GBP or SGD)', async () => {
    lines = [line('USD', '35.10', '35.40')];
    await mountWithBranch();
    await vi.waitFor(() => expect(offered()).toEqual(['USD']));
    expect(currencySelect().value).toBe('USD');
  });

  it('C2 only codes with a rate on the chosen side, in publication order: BUY reads buyRate, SELL reads sellRate', async () => {
    lines = [line('USD', '35.10', '35.40'), line('EUR', '38.00', null), line('JPY', null, '0.24'), line('CNY', '4.90', '5.00')];
    await mountWithBranch();
    choose('shopSide', 'BUY');
    await vi.waitFor(() => expect(offered()).toEqual(['USD', 'EUR', 'CNY']));
    choose('shopSide', 'SELL');
    await vi.waitFor(() => expect(offered()).toEqual(['USD', 'JPY', 'CNY']));
  });

  it('C3 a chosen currency the new side does not offer moves to the first offered code', async () => {
    lines = [line('EUR', '38.00', null), line('JPY', null, '0.24'), line('USD', '35.10', '35.40')];
    await mountWithBranch();
    choose('shopSide', 'BUY');
    await vi.waitFor(() => expect(offered()).toEqual(['EUR', 'USD']));
    choose('currency', 'EUR');
    choose('shopSide', 'SELL');
    await vi.waitFor(() => expect(offered()).toEqual(['JPY', 'USD']));
    expect(currencySelect().value).toBe('JPY');
  });

  it('C4 nothing published on the side: no option, and a hint says so', async () => {
    lines = [line('USD', null, '35.40')];
    await mountWithBranch();
    choose('shopSide', 'BUY');
    await vi.waitFor(() => expect(host.querySelector('[data-hint="no-published-currency"]')).not.toBeNull());
    expect(offered()).toEqual([]);
    expect(currencySelect().value).toBe('');
    expect(host.querySelector('[data-hint="no-published-currency"]')!.textContent).toBe(
      'สาขานี้ยังไม่มีสกุลเงินที่ประกาศเรทสำหรับรายการนี้',
    );
  });

  it('C5 a currency published on the side hides the hint', async () => {
    lines = [line('USD', '35.10', '35.40')];
    await mountWithBranch();
    await vi.waitFor(() => expect(offered()).toEqual(['USD']));
    expect(host.querySelector('[data-hint="no-published-currency"]')).toBeNull();
  });
});
