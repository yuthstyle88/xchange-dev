import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { makeBookingSlotsData, makeBookingSlotItemDto } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { BookScreen } from './Book';

describe('Customer Book: keys and sends the branch by id (CB-Book, #4830)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  const BRANCH_FIXTURE = {
    id: 'br-1',
    code: 'MAIN',
    name: 'Siam',
    timezone: 'Asia/Bangkok',
    bookingEnabled: true,
  };

  const BRANCHES = {
    branches: [BRANCH_FIXTURE],
  };

  const SLOTS = makeBookingSlotsData({
    slots: [
      makeBookingSlotItemDto({
        id: 'slot-1',
        start: '2026-10-15T03:00:00+00:00',
        end: '2026-10-15T03:30:00+00:00',
        available: 3,
      }),
    ],
  });

  const PLANNER_PREVIEW = {
    thb: '3542.00',
    rate: '35.42000000',
    unitSize: '1',
    stale: false,
    sourceDate: '2026-10-15',
  };

  const BOOKING_CREATE_SUCCESS = {
    id: 'bk-1',
    status: 'CONFIRMED',
    branchId: 'br-1',
    slotId: 'slot-1',
    shopSide: 'BUY',
    currency: 'USD',
    quantity: 100,
  };

  interface RecordedCall {
    url: string;
    pathname: string;
    searchParams: URLSearchParams;
    method: string;
    body: any;
  }

  let calls: RecordedCall[];

  function setupFetch(customBranches?: { branches: any[] }) {
    calls = [];
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const urlObj = new URL(rawUrl, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      let parsedBody: any = null;
      if (init?.body && typeof init.body === 'string') {
        try {
          parsedBody = JSON.parse(init.body);
        } catch {
          parsedBody = init.body;
        }
      }
      calls.push({
        url: rawUrl,
        pathname: urlObj.pathname,
        searchParams: urlObj.searchParams,
        method,
        body: parsedBody,
      });

      if (urlObj.pathname === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      if (urlObj.pathname.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(customBranches ?? BRANCHES), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      if (urlObj.pathname === '/api/v3/views/booking_slots') {
        return new Response(JSON.stringify({ data: SLOTS }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      if (urlObj.pathname === '/api/v3/compute/planner_preview') {
        return new Response(JSON.stringify(PLANNER_PREVIEW), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      if (urlObj.pathname === '/api/bookings' && method === 'POST') {
        return new Response(JSON.stringify(BOOKING_CREATE_SUCCESS), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      return new Response('Not found', { status: 404 });
    });
    vi.stubGlobal('fetch', fetchMock);
    return fetchMock;
  }

  beforeEach(() => {
    setLang('en');
    setAudience('customer');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    window.history.replaceState({}, '', '/app/book');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('B1: ?branch=br-1 selects br-1, shows Siam (MAIN), carries branch=br-1 in booking_slots request', async () => {
    window.history.replaceState({}, '', '/app/book?branch=br-1');
    const fetchMock = setupFetch();
    dispose = render(() => <BookScreen fetch={fetchMock} />, host);

    await vi.waitFor(() => {
      const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(branchSel!.value).toBe('br-1');
    });

    const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    const selectedOption = branchSel.options[branchSel.selectedIndex];
    expect(selectedOption.textContent?.trim()).toBe('Siam (MAIN)');

    await vi.waitFor(() => {
      const slotCall = calls.find((c) => c.pathname === '/api/v3/views/booking_slots');
      expect(slotCall).toBeDefined();
      expect(slotCall!.searchParams.get('branch')).toBe('br-1');
    });
  });

  it('B2: ?branch=MAIN (a printed code link) selects br-1, carries branch=br-1 in booking_slots request, never MAIN', async () => {
    window.history.replaceState({}, '', '/app/book?branch=MAIN');
    const fetchMock = setupFetch();
    dispose = render(() => <BookScreen fetch={fetchMock} />, host);

    await vi.waitFor(() => {
      const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(branchSel!.value).toBe('br-1');
    });

    await vi.waitFor(() => {
      const slotCall = calls.find((c) => c.pathname === '/api/v3/views/booking_slots');
      expect(slotCall).toBeDefined();
      expect(slotCall!.searchParams.get('branch')).toBe('br-1');
      expect(slotCall!.searchParams.get('branch')).not.toBe('MAIN');
    });
  });

  it('B3: no ?branch=; user picks Siam (MAIN), a slot and a quantity, then submits', async () => {
    window.history.replaceState({}, '', '/app/book');
    const fetchMock = setupFetch();
    dispose = render(() => <BookScreen fetch={fetchMock} />, host);

    await vi.waitFor(() => {
      const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(branchSel!.options.length).toBeGreaterThan(1);
    });

    const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'br-1';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      const slotSel = host.querySelector<HTMLSelectElement>('select[name="slotId"]');
      expect(slotSel).not.toBeNull();
      const slotOption = host.querySelector<HTMLOptionElement>('option[value="slot-1"]');
      expect(slotOption).not.toBeNull();
    });

    const slotSel = host.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = 'slot-1';
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));

    const qtyInput = host.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    await vi.waitFor(() => {
      const previewCall = calls.find((c) => c.pathname === '/api/v3/compute/planner_preview');
      expect(previewCall).toBeDefined();
      expect(previewCall!.body.branch).toBe('br-1');
    });

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    expect(submitBtn.disabled).toBe(false);
    submitBtn.click();

    await vi.waitFor(() => {
      const createCall = calls.find((c) => c.pathname === '/api/bookings' && c.method === 'POST');
      expect(createCall).toBeDefined();
      expect(createCall!.body.branch).toBe('br-1');
      expect(createCall!.body.branchId).toBe('br-1');
    });
  });

  it('B4: ?branch=br-1, slots answered: slot options render, with times in Asia/Bangkok', async () => {
    window.history.replaceState({}, '', '/app/book?branch=br-1');
    const fetchMock = setupFetch();
    dispose = render(() => <BookScreen fetch={fetchMock} />, host);

    await vi.waitFor(() => {
      const slotOption = host.querySelector<HTMLOptionElement>('option[value="slot-1"]');
      expect(slotOption).not.toBeNull();
      // UTC 03:00-03:30 in Asia/Bangkok (UTC+7) is 10:00 - 10:30
      expect(slotOption!.textContent).toContain('10:00');
      expect(slotOption!.textContent).toContain('10:30');
    });
  });

  it('B5: list also holds { id: br-2, code: br-1 }; ?branch=br-1 selects br-1 (Siam), not br-2 (id beats code)', async () => {
    window.history.replaceState({}, '', '/app/book?branch=br-1');
    const twoBranches = {
      branches: [
        BRANCH_FIXTURE,
        {
          id: 'br-2',
          code: 'br-1',
          name: 'Other',
          timezone: 'Asia/Bangkok',
          bookingEnabled: true,
        },
      ],
    };
    const fetchMock = setupFetch(twoBranches);
    dispose = render(() => <BookScreen fetch={fetchMock} />, host);

    await vi.waitFor(() => {
      const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      expect(branchSel!.value).toBe('br-1');
    });

    const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    const selectedOption = branchSel.options[branchSel.selectedIndex];
    expect(selectedOption.textContent?.trim()).toBe('Siam (MAIN)');
  });

  it('B6: ?branch=GONE: no branch is selected and no booking_slots request is sent', async () => {
    window.history.replaceState({}, '', '/app/book?branch=GONE');
    const fetchMock = setupFetch();
    dispose = render(() => <BookScreen fetch={fetchMock} />, host);

    await vi.waitFor(() => {
      const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]');
      expect(branchSel).not.toBeNull();
      // branches finished loading
      expect(branchSel!.options.length).toBeGreaterThan(1);
    });

    const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    expect(branchSel.value).toBe('');

    const slotCall = calls.find((c) => c.pathname === '/api/v3/views/booking_slots');
    expect(slotCall).toBeUndefined();
  });
});
