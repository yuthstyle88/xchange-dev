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

describe('Customer Book (C02) slot reset on date, branch, and slot reload (#2192)', () => {
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

  it('Row 1: Branch -> date A -> slot S1 -> Book sends S1, and a subsequent date change does not re-send S1', async () => {
    const createBodies: Record<string, unknown>[] = [];

    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        const u = new URL(url, 'http://xc.test');
        if (u.searchParams.get('date') === '2099-10-16') {
          return new Response(JSON.stringify(SLOTS_DATE_B), { status: 200 });
        }
        return new Response(JSON.stringify(SLOTS_DATE_A), { status: 200 });
      }
      if (url === '/api/v3/compute/planner_preview') {
        return new Response(JSON.stringify({ thb: '3500.00' }), { status: 200 });
      }
      if (url === '/api/bookings' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body ?? '{}')) as Record<string, unknown>;
        createBodies.push(body);
        return new Response(
          JSON.stringify({
            id: `bk-${createBodies.length}`,
            status: 'CONFIRMED',
            branchId: body.branchId,
            slotId: body.slotId,
          }),
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

    const dateInput = container.querySelector<HTMLInputElement>('input[name="serviceDate"]')!;
    dateInput.value = '2099-10-15';
    dateInput.dispatchEvent(new Event('change', { bubbles: true }));

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
    });
    expect(createBodies[0]!.slotId).toBe('slot-a1');

    // Changing to date B without picking a new slot must clear slotId
    dateInput.value = '2099-10-16';
    dateInput.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="slot-b1"]')).not.toBeNull();
    });

    submitBtn.click();
    await vi.waitFor(() => {
      expect(createBodies.length).toBe(2);
    });
    expect(createBodies[1]!.slotId).toBe('');
  });

  it('Row 2: Slot S1 on date A -> change date to B -> Book without choosing sends slotId "" (not S1)', async () => {
    const createBodies: Record<string, unknown>[] = [];

    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        const u = new URL(url, 'http://xc.test');
        if (u.searchParams.get('date') === '2099-10-16') {
          return new Response(JSON.stringify(SLOTS_DATE_B), { status: 200 });
        }
        return new Response(JSON.stringify(SLOTS_DATE_A), { status: 200 });
      }
      if (url === '/api/v3/compute/planner_preview') {
        return new Response(JSON.stringify({ thb: '3500.00' }), { status: 200 });
      }
      if (url === '/api/bookings' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body ?? '{}')) as Record<string, unknown>;
        createBodies.push(body);
        return new Response(
          JSON.stringify({ id: 'bk-1', status: 'CONFIRMED' }),
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

    const dateInput = container.querySelector<HTMLInputElement>('input[name="serviceDate"]')!;
    dateInput.value = '2099-10-15';
    dateInput.dispatchEvent(new Event('input', { bubbles: true }));

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="slot-a1"]')).not.toBeNull();
    });

    const slotSel = container.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = 'slot-a1';
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));
    expect(slotSel.value).toBe('slot-a1');

    // Change date to B
    dateInput.value = '2099-10-16';
    dateInput.dispatchEvent(new Event('input', { bubbles: true }));
    expect(slotSel.value).toBe('');

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="slot-b1"]')).not.toBeNull();
    });

    const qtyInput = container.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    submitBtn.click();

    await vi.waitFor(() => {
      expect(createBodies.length).toBe(1);
    });
    expect(createBodies[0]!.slotId).toBe('');
    expect(createBodies[0]!.slotId).not.toBe('slot-a1');
  });

  it('Row 3: Slot chosen -> change branch -> Book carries no slot of the old branch', async () => {
    const createBodies: Record<string, unknown>[] = [];
    let delayBranch2Slots = false;

    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        const u = new URL(url, 'http://xc.test');
        if (u.searchParams.get('branch') === 'XC-B02') {
          if (delayBranch2Slots) {
            await new Promise((r) => setTimeout(r, 30));
          }
          return new Response(JSON.stringify(SLOTS_BRANCH_2), { status: 200 });
        }
        return new Response(JSON.stringify(SLOTS_DATE_A), { status: 200 });
      }
      if (url === '/api/v3/compute/planner_preview') {
        return new Response(JSON.stringify({ thb: '3500.00' }), { status: 200 });
      }
      if (url === '/api/bookings' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body ?? '{}')) as Record<string, unknown>;
        createBodies.push(body);
        return new Response(
          JSON.stringify({ id: 'bk-branch', status: 'CONFIRMED' }),
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
    expect(slotSel.value).toBe('slot-a1');

    const qtyInput = container.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    // Change branch to XC-B02; slotId is cleared immediately even before XC-B02 slots arrive
    delayBranch2Slots = true;
    branchSel.value = 'XC-B02';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));
    expect(slotSel.value).toBe('');

    const submitBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    submitBtn.click();

    await vi.waitFor(() => {
      expect(createBodies.length).toBe(1);
    });
    expect(createBodies[0]!.branchId).toBe('XC-B02');
    expect(createBodies[0]!.slotId).toBe('');
    expect(createBodies[0]!.slotId).not.toBe('slot-a1');
  });

  it('Row 4: capacityFull / slotUnavailable re-reads booking_slots and clears a slot no longer listed', async () => {
    let slotCalls = 0;
    let firstAttempt = true;
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
        if (firstAttempt) {
          return new Response(JSON.stringify(SLOTS_DATE_A), { status: 200 });
        }
        // On re-read after capacityFull / slotUnavailable, slot-a1 is no longer listed
        return new Response(
          JSON.stringify({
            data: {
              slots: [SLOTS_DATE_A.data.slots[1]],
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
        if (firstAttempt) {
          firstAttempt = false;
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
          JSON.stringify({ id: 'bk-2', status: 'CONFIRMED' }),
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

    // Wait for booking_slots re-read where slot-a1 is no longer in the list
    await vi.waitFor(() => {
      expect(slotCalls).toBe(2);
      expect(container.querySelector('option[value="slot-a1"]')).toBeNull();
      expect(slotSel.value).toBe('');
    });

    // Submitting again without picking a new slot sends '' instead of stale slot-a1
    submitBtn.click();
    await vi.waitFor(() => {
      expect(createBodies.length).toBe(2);
    });
    expect(createBodies[1]!.slotId).toBe('');
  });

  it('Edge: The date changes to B and back to A — the user must pick a slot again (no restore)', async () => {
    const createBodies: Record<string, unknown>[] = [];

    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (url === '/api/public/host') {
        return new Response(JSON.stringify({ org: 'XC' }), { status: 200 });
      }
      if (url.startsWith('/api/public/orgs/XC/branches')) {
        return new Response(JSON.stringify(BRANCHES), { status: 200 });
      }
      if (url.includes('/api/v3/views/booking_slots')) {
        const u = new URL(url, 'http://xc.test');
        if (u.searchParams.get('date') === '2099-10-16') {
          // Delay Date B so switching back to Date A tests that the date change itself cleared slotId
          await new Promise((r) => setTimeout(r, 30));
          return new Response(JSON.stringify(SLOTS_DATE_A), { status: 200 });
        }
        return new Response(JSON.stringify(SLOTS_DATE_A), { status: 200 });
      }
      if (url === '/api/v3/compute/planner_preview') {
        return new Response(JSON.stringify({ thb: '3500.00' }), { status: 200 });
      }
      if (url === '/api/bookings' && init?.method === 'POST') {
        const body = JSON.parse(String(init.body ?? '{}')) as Record<string, unknown>;
        createBodies.push(body);
        return new Response(
          JSON.stringify({ id: 'bk-edge', status: 'CONFIRMED' }),
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

    const dateInput = container.querySelector<HTMLInputElement>('input[name="serviceDate"]')!;
    dateInput.value = '2099-10-15';
    dateInput.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="slot-a1"]')).not.toBeNull();
    });

    const slotSel = container.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = 'slot-a1';
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));
    expect(slotSel.value).toBe('slot-a1');

    // Change date to B, then back to A
    dateInput.value = '2099-10-16';
    dateInput.dispatchEvent(new Event('change', { bubbles: true }));
    expect(slotSel.value).toBe('');

    dateInput.value = '2099-10-15';
    dateInput.dispatchEvent(new Event('change', { bubbles: true }));
    expect(slotSel.value).toBe('');

    await vi.waitFor(() => {
      expect(container.querySelector('option[value="slot-a1"]')).not.toBeNull();
    });
    expect(slotSel.value).toBe('');

    const qtyInput = container.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    submitBtn.click();

    await vi.waitFor(() => {
      expect(createBodies.length).toBe(1);
    });
    expect(createBodies[0]!.slotId).toBe('');
  });
});
