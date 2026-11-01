import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeDeskConfig,
  makeDeskRate,
  makeMe,
  makeMeRole,
  makeQuoteDto,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from './i18n';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { withBookingSlots } from './booking-slots.testing';

interface WriteCall {
  path: string;
  body: Record<string, unknown>;
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const STAFF_TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: 'XC-B01' })],
  branch: 'XC-B01',
  branchName: 'XC-B01',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const BOOKINGS_DATA = makeDeskBookingsData({
  items: [
    makeDeskBookingItemDto({
      id: 'bk-1',
      bookingNo: 'B261015-0001',
      status: 'CONFIRMED',
      slotId: 'slot-1',
      startAt: '2026-10-15T09:00:00Z',
      endAt: '2026-10-15T09:30:00Z',
    }),
  ],
});

function makeServer(configResponse: { status: number; body: unknown } = {
  status: 200,
  body: {
    version: 1,
    changed: true,
    data: makeDeskConfig({
      rates: [
        makeDeskRate({ code: 'USD', buy: '35.00', sell: '35.50' }),
        makeDeskRate({ code: 'EUR', buy: '38.00', sell: null }),
      ],
    }),
  },
}) {
  const quotes: WriteCall[] = [];

  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(rawUrl, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();

    if (method === 'GET' && u.pathname === '/api/v3/views/me') {
      return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
      return jsonResponse(200, { version: 1, changed: true, data: BOOKINGS_DATA });
    }
    if (method === 'GET' && (u.pathname === '/api/v3/views/desk_config' || u.pathname === '/api/v3/views/staff_desk_config')) {
      return jsonResponse(configResponse.status, configResponse.body);
    }
    if (method === 'POST' && u.pathname === '/api/quotes') {
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : {};
      quotes.push({ path: u.pathname, body });
      return jsonResponse(200, makeQuoteDto({ id: 'q-booking', channel: 'BOOKING' }));
    }
    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  vi.stubGlobal('fetch', withBookingSlots(fetchFn));

  return { quotes };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function setValue(el: HTMLInputElement | HTMLSelectElement, val: string): void {
  el.value = val;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('DeskBookingsScreen §8 published-currency (FE-T2b #5882)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('staff');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function openDialog(): Promise<void> {
    dispose = render(() => <DeskBookingsScreen />, host);
    await vi.waitFor(() => expect(host.querySelector('[data-booking-id="bk-1"]')).not.toBeNull());
    host.querySelector<HTMLButtonElement>('button[data-action="new-booking"]')!.click();
    await vi.waitFor(() => expect(host.querySelector('select[name="currency"]')).not.toBeNull());
    await flush();
  }

  it('P1 config USD{buy,sell}, EUR{buy only}; side SELL -> options = [USD]', async () => {
    makeServer();
    await openDialog();
    const currencySelect = host.querySelector<HTMLSelectElement>('select[name="currency"]')!;
    const options = Array.from(currencySelect.options).map((o) => o.value);
    expect(options).toEqual(['USD']);
  });

  it('P2 the same config; side BUY -> options = [USD, EUR]', async () => {
    makeServer();
    await openDialog();
    const sideSelect = host.querySelector<HTMLSelectElement>('select[name="shopSide"]')!;
    setValue(sideSelect, 'BUY');
    await flush();
    const currencySelect = host.querySelector<HTMLSelectElement>('select[name="currency"]')!;
    const options = Array.from(currencySelect.options).map((o) => o.value);
    expect(options).toEqual(['USD', 'EUR']);
  });

  it('P3 side BUY with EUR chosen; switch to SELL -> EUR is no longer offered; the currency resets to USD; requestBookingQuote fires with USD', async () => {
    const server = makeServer();
    await openDialog();
    const sideSelect = host.querySelector<HTMLSelectElement>('select[name="shopSide"]')!;
    const currencySelect = host.querySelector<HTMLSelectElement>('select[name="currency"]')!;
    const qtyInput = host.querySelector<HTMLInputElement>('input[name="quantity"]')!;

    setValue(sideSelect, 'BUY');
    await flush();
    setValue(currencySelect, 'EUR');
    setValue(qtyInput, '100');
    await vi.waitFor(() => expect(server.quotes.length).toBeGreaterThan(0));

    setValue(sideSelect, 'SELL');
    await flush();

    const options = Array.from(currencySelect.options).map((o) => o.value);
    expect(options).toEqual(['USD']);
    expect(currencySelect.value).toBe('USD');

    await vi.waitFor(() => {
      const last = server.quotes[server.quotes.length - 1];
      expect(last?.body.lines).toEqual([expect.objectContaining({ currency: 'USD', shopSide: 'SELL' })]);
    });
  });

  it('P4 no code published for the chosen side -> no options, and the page does not crash; currency is empty', async () => {
    makeServer({
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: makeDeskConfig({
          rates: [makeDeskRate({ code: 'EUR', buy: '38.00', sell: null })],
        }),
      },
    });
    await openDialog();
    const currencySelect = host.querySelector<HTMLSelectElement>('select[name="currency"]')!;
    const options = Array.from(currencySelect.options).map((o) => o.value);
    expect(options).toEqual([]);
    expect(currencySelect.value).toBe('');
    expect(host.textContent).toContain(th.noPublishedRate);
  });

  it('P5 config fails to load -> today\'s behaviour (the list stays at its default, no throw)', async () => {
    makeServer({ status: 500, body: { error: 'Internal Server Error' } });
    await openDialog();
    const currencySelect = host.querySelector<HTMLSelectElement>('select[name="currency"]')!;
    const options = Array.from(currencySelect.options).map((o) => o.value);
    expect(options).toEqual(['USD']);
    expect(currencySelect.value).toBe('USD');
  });
});
