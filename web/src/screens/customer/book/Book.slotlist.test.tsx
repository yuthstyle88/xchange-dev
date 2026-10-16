import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { BookScreen } from './Book';

const BRANCHES = {
  branches: [
    {
      code: 'XC-B01',
      name: 'สาขาสยามพารากอน',
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
    },
    {
      code: 'XC-B02',
      name: 'สาขาอารีย์',
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
    },
  ],
};

const SLOTS_DATE_A = {
  data: {
    slots: [
      {
        id: 'slot-a1',
        start: '2099-10-15T03:00:00Z',
        end: '2099-10-15T03:30:00Z',
        available: 3,
      },
      {
        id: 'slot-a2',
        start: '2099-10-15T03:30:00Z',
        end: '2099-10-15T04:00:00Z',
        available: 2,
      },
    ],
  },
};

const SLOTS_DATE_B = {
  data: {
    slots: [
      {
        id: 'slot-b1',
        start: '2099-10-16T04:00:00Z',
        end: '2099-10-16T04:30:00Z',
        available: 4,
      },
    ],
  },
};

const SLOTS_BRANCH_2 = {
  data: {
    slots: [
      {
        id: 'slot-b2-1',
        start: '2099-10-15T05:00:00Z',
        end: '2099-10-15T05:30:00Z',
        available: 5,
      },
    ],
  },
};

describe('Customer Book (C02) slot list sequencing, failed re-read cleanup, and select value retention (#2257)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    setAudience('customer');
    container = document.createElement('div');
    document.body.appendChild(container);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    container.remove();
    window.history.replaceState({}, '', '/app/book');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("Row 1: Date A -> B, A's slots answer after B's -> B's list shown; A's answer ignored", async () => {
    let resolveDateA: ((res: Response) => void) | undefined;
    const dateAPromise = new Promise<Response>((resolve) => {
      resolveDateA = resolve;
    });

    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        const u = new URL(url, 'http://xc.test');
        const d = u.searchParams.get('date');
        if (d === '2099-10-15') {
          return dateAPromise;
        }
        if (d === '2099-10-16') {
          return new Response(JSON.stringify(SLOTS_DATE_B), { status: 200 });
        }
        return new Response(JSON.stringify({ data: { slots: [] } }), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    const dateInput = container.querySelector<HTMLInputElement>('input[name="serviceDate"]')!;
    // Pick Date A (held in-flight)
    dateInput.value = '2099-10-15';
    dateInput.dispatchEvent(new Event('change', { bubbles: true }));

    // Switch to Date B before Date A answers
    dateInput.value = '2099-10-16';
    dateInput.dispatchEvent(new Event('change', { bubbles: true }));

    // Date B's list arrives first
    await vi.waitFor(() => {
      expect(container.querySelector('option[value="slot-b1"]')).not.toBeNull();
    });

    // Now Date A's late response arrives
    resolveDateA!(new Response(JSON.stringify(SLOTS_DATE_A), { status: 200 }));
    await new Promise((r) => setTimeout(r, 20));

    // B's list must still be shown; A's late answer must be ignored
    expect(container.querySelector('option[value="slot-b1"]')).not.toBeNull();
    expect(container.querySelector('option[value="slot-a1"]')).toBeNull();
  });

  it("Row 2: capacityFull, then the booking_slots re-read fails -> slotId is ''; Book sends no hidden slot", async () => {
    let slotCalls = 0;
    const createBodies: Record<string, unknown>[] = [];

    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        slotCalls++;
        if (slotCalls === 1) {
          return new Response(JSON.stringify(SLOTS_DATE_A), { status: 200 });
        }
        // Re-read after capacityFull fails with HTTP 500
        return new Response(JSON.stringify({ error: 'slots backend down' }), { status: 500 });
      }
      if (url === '/api/v3/compute/planner_preview') {
        return new Response(JSON.stringify({ thb: '3500.00' }), { status: 200 });
      }
      if (url === '/api/bookings' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body ?? '{}')) as Record<string, unknown>;
        createBodies.push(body);
        if (createBodies.length === 1) {
          return new Response(
            JSON.stringify({
              error: {
                code: 'capacityFull',
                message: 'ที่นั่งในช่วงเวลานี้เต็มแล้ว',
              },
            }),
            { status: 400, headers: { 'Content-Type': 'application/json' } },
          );
        }
        return new Response(
          JSON.stringify({ id: 'bk-after-fail', status: 'CONFIRMED' }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        );
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="slot-a1"]')).not.toBeNull();
    });

    const slotSel = container.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = 'slot-a1';
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));

    const qtyInput = container.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    submitBtn.click();

    // Wait for capacityFull and the failed booking_slots re-read
    await vi.waitFor(() => {
      expect(slotCalls).toBe(2);
      expect(container.querySelector('.xc-book-slots-error')).not.toBeNull();
    });

    expect(slotSel.value).toBe('');

    // Next Book must send '' and NOT the hidden/refused slot-a1
    submitBtn.click();
    await vi.waitFor(() => {
      expect(createBodies.length).toBe(2);
    });
    expect(createBodies[1]!.slotId).toBe('');
    expect(createBodies[1]!.slotId).not.toBe('slot-a1');
  });

  it('Row 3: Re-read still lists the chosen slot -> The select shows that slot', async () => {
    let slotCalls = 0;

    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        slotCalls++;
        return new Response(
          JSON.stringify({
            data: {
              slots: [
                {
                  id: 'slot-a1',
                  start: '2099-10-15T03:00:00Z',
                  end: '2099-10-15T03:30:00Z',
                  available: slotCalls === 1 ? 3 : 1,
                },
                {
                  id: 'slot-a2',
                  start: '2099-10-15T03:30:00Z',
                  end: '2099-10-15T04:00:00Z',
                  available: 2,
                },
              ],
            },
          }),
          { status: 200 },
        );
      }
      if (url === '/api/v3/compute/planner_preview') {
        return new Response(JSON.stringify({ thb: '3500.00' }), { status: 200 });
      }
      if (url === '/api/bookings' && init?.method === 'POST') {
        return new Response(
          JSON.stringify({
            error: {
              code: 'capacityFull',
              message: 'ที่นั่งในช่วงเวลานี้เต็มแล้ว',
            },
          }),
          { status: 400, headers: { 'Content-Type': 'application/json' } },
        );
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="slot-a1"]')).not.toBeNull();
    });

    const slotSel = container.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = 'slot-a1';
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));
    expect(slotSel.value).toBe('slot-a1');

    const qtyInput = container.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    submitBtn.click();

    // Wait for re-read to rebuild options with updated availability (1 available)
    await vi.waitFor(() => {
      expect(slotCalls).toBe(2);
      const opt = container.querySelector<HTMLOptionElement>('option[value="slot-a1"]');
      expect(opt?.textContent).toContain('ว่าง 1 ที่');
    });

    // The select must still show slot-a1, not the '' placeholder
    expect(slotSel.value).toBe('slot-a1');
  });

  it('Row 4: Normal pick and Book -> POST with the picked slot (and keeps select value across slot list rebuild)', async () => {
    let slotCalls = 0;
    const createBodies: Record<string, unknown>[] = [];

    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        slotCalls++;
        return new Response(
          JSON.stringify({
            data: {
              slots: [
                {
                  id: 'slot-a1',
                  start: '2099-10-15T03:00:00Z',
                  end: '2099-10-15T03:30:00Z',
                  available: slotCalls === 1 ? 3 : 2,
                },
              ],
            },
          }),
          { status: 200 },
        );
      }
      if (url === '/api/v3/compute/planner_preview') {
        return new Response(JSON.stringify({ thb: '3500.00' }), { status: 200 });
      }
      if (url === '/api/bookings' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body ?? '{}')) as Record<string, unknown>;
        createBodies.push(body);
        if (createBodies.length === 1) {
          return new Response(
            JSON.stringify({
              error: {
                code: 'slotUnavailable',
                message: 'ช่วงเวลานี้ไม่ว่างแล้ว',
              },
            }),
            { status: 400, headers: { 'Content-Type': 'application/json' } },
          );
        }
        return new Response(
          JSON.stringify({ id: 'bk-normal', status: 'CONFIRMED', slotId: body.slotId }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        );
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="slot-a1"]')).not.toBeNull();
    });

    const slotSel = container.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = 'slot-a1';
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));

    const qtyInput = container.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    submitBtn.click();

    await vi.waitFor(() => {
      expect(createBodies.length).toBe(1);
      expect(slotCalls).toBe(2);
    });
    expect(createBodies[0]!.slotId).toBe('slot-a1');
    expect(slotSel.value).toBe('slot-a1');

    submitBtn.click();
    await vi.waitFor(() => {
      expect(createBodies.length).toBe(2);
    });
    expect(createBodies[1]!.slotId).toBe('slot-a1');
  });

  it('Row 5: POST with no valid slot -> Still allowed after a network-failed slot re-read clears slotId', async () => {
    let slotCalls = 0;
    const createBodies: Record<string, unknown>[] = [];

    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        slotCalls++;
        if (slotCalls === 1) {
          return new Response(JSON.stringify(SLOTS_DATE_A), { status: 200 });
        }
        throw new Error('network error during slot re-read');
      }
      if (url === '/api/v3/compute/planner_preview') {
        return new Response(JSON.stringify({ thb: '3500.00' }), { status: 200 });
      }
      if (url === '/api/bookings' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body ?? '{}')) as Record<string, unknown>;
        createBodies.push(body);
        if (createBodies.length === 1) {
          return new Response(
            JSON.stringify({
              error: {
                code: 'slotUnavailable',
                message: 'ช่วงเวลานี้ไม่ว่างแล้ว',
              },
            }),
            { status: 400, headers: { 'Content-Type': 'application/json' } },
          );
        }
        return new Response(
          JSON.stringify({ id: 'bk-no-slot', status: 'CONFIRMED' }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        );
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="slot-a1"]')).not.toBeNull();
    });

    const slotSel = container.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = 'slot-a1';
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));

    const qtyInput = container.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    submitBtn.click();

    await vi.waitFor(() => {
      expect(slotCalls).toBe(2);
      expect(container.querySelector('.xc-book-slots-error')).not.toBeNull();
    });

    // POST with no valid slot is still allowed and sends ''
    submitBtn.click();
    await vi.waitFor(() => {
      expect(createBodies.length).toBe(2);
    });
    expect(createBodies[1]!.slotId).toBe('');
  });

  it("Edge: Branch change while slots load -> new branch's list shown; old branch's late answer ignored", async () => {
    let resolveBranch1: ((res: Response) => void) | undefined;
    const branch1Promise = new Promise<Response>((resolve) => {
      resolveBranch1 = resolve;
    });

    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        const u = new URL(url, 'http://xc.test');
        if (u.searchParams.get('branch') === 'XC-B01') {
          return branch1Promise;
        }
        return new Response(JSON.stringify(SLOTS_BRANCH_2), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    // Select XC-B01 (held in-flight)
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    // Switch to XC-B02 while XC-B01 slots are still loading
    branchSel.value = 'XC-B02';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="slot-b2-1"]')).not.toBeNull();
    });

    // Now XC-B01's late slot response resolves
    resolveBranch1!(new Response(JSON.stringify(SLOTS_DATE_A), { status: 200 }));
    await new Promise((r) => setTimeout(r, 20));

    expect(container.querySelector('option[value="slot-b2-1"]')).not.toBeNull();
    expect(container.querySelector('option[value="slot-a1"]')).toBeNull();
  });

  it('Edge: Clear date while slots read is pending -> no loading text, no slots, and late answer does not show them', async () => {
    let resolvePendingDate: ((res: Response) => void) | undefined;
    const pendingDatePromise = new Promise<Response>((resolve) => {
      resolvePendingDate = resolve;
    });

    const fetchMock = vi.fn(async (url: string) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        const u = new URL(url, 'http://xc.test');
        if (u.searchParams.get('date') === '2099-10-16') {
          return pendingDatePromise;
        }
        return new Response(JSON.stringify(SLOTS_DATE_A), { status: 200 });
      }
      return new Response('Not found', { status: 404 });
    });

    dispose = render(() => <BookScreen fetch={fetchMock} />, container);

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });

    const branchSel = container.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    // Initial date slots arrive so we also verify old slots are cleared when date is cleared
    await vi.waitFor(() => {
      expect(container.querySelector('option[value="slot-a1"]')).not.toBeNull();
    });

    const dateInput = container.querySelector<HTMLInputElement>('input[name="serviceDate"]')!;
    // Pick a date whose slot read stays pending
    dateInput.value = '2099-10-16';
    dateInput.dispatchEvent(new Event('change', { bubbles: true }));

    expect(container.querySelector('.xc-book-slots-loading')).not.toBeNull();

    // Clear the date while the read is pending
    dateInput.value = '';
    dateInput.dispatchEvent(new Event('change', { bubbles: true }));

    // Loading indicator must clear immediately and no slots may remain pickable
    expect(container.querySelector('.xc-book-slots-loading')).toBeNull();
    expect(container.querySelector('option[value="slot-a1"]')).toBeNull();
    expect(container.querySelector('option[value="slot-b1"]')).toBeNull();

    // Now the late answer for 2099-10-16 resolves
    resolvePendingDate!(new Response(JSON.stringify(SLOTS_DATE_B), { status: 200 }));
    await new Promise((r) => setTimeout(r, 20));

    expect(container.querySelector('.xc-book-slots-loading')).toBeNull();
    expect(container.querySelector('option[value="slot-b1"]')).toBeNull();
  });
});
