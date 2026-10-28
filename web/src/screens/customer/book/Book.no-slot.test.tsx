// /app/book refuses a submit with no slot in code (qa/e2e flow-validation-customer:83 on xc-v3 f57224db, #564
// issuecomment-6468244767 cause D). When every slot of the day is closed, the slot <select> is disabled, and a disabled
// `required` control is barred from constraint validation, so the browser submits the form anyway and the screen sent
// POST /api/bookings with no slot. Written before the fix by the FE lane controller claude/e92db6. Do not edit, delete
// or weaken these tests; add your own elsewhere.
//
// [required] by these tests: `handleSubmit` refuses before any request when no slot is chosen, and shows the slot field's
// error (`[data-field-error="slotId"]`, role="alert"); with a slot chosen it books as today.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { makeBookingCreateOutput } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { BookScreen } from './Book';

const BRANCH_ID = 'XC-B01';
const SLOT_1_ID = '00000000-0000-4000-8000-000000000001';

const BRANCHES = {
  branches: [{ id: BRANCH_ID, code: BRANCH_ID, name: 'สาขาสยามพารากอน', timezone: 'Asia/Bangkok', bookingEnabled: true }],
};

function slotsAnswer(slots: Array<{ id: string; start: string; end: string; available: number }>) {
  return { version: 1, changed: true, data: { slots } };
}

const OPEN_SLOT = { id: SLOT_1_ID, start: '2026-10-15T03:00:00+00:00', end: '2026-10-15T03:30:00+00:00', available: 3 };
const FULL_SLOT = { ...OPEN_SLOT, available: 0 };

const PREVIEW = { thb: '3542.00', rate: '35.42000000', unitSize: '1', stale: false, sourceDate: '2026-10-15' };

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('Customer Book: no slot chosen means no booking request (cause D)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let bookingBodies: Array<Record<string, unknown>>;
  let slots: ReturnType<typeof slotsAnswer>;
  let fetchStub: (input: string, init?: RequestInit) => Promise<Response>;

  beforeAll(() => warmModules(() => import('./Book')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    window.history.replaceState({}, '', '/app/book');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    bookingBodies = [];
    slots = slotsAnswer([]);
    fetchStub = vi.fn(async (input: string, init?: RequestInit): Promise<Response> => {
      const u = new URL(input, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (u.pathname === '/api/public/host') return json(200, { org: 'XC', env: 'test' });
      if (u.pathname.startsWith('/api/public/orgs/')) return json(200, BRANCHES);
      if (u.pathname === '/api/v3/views/booking_slots') return json(200, slots);
      if (u.pathname === '/api/v3/compute/planner_preview' && method === 'POST') return json(200, PREVIEW);
      if (u.pathname === '/api/bookings' && method === 'POST') {
        bookingBodies.push(typeof init?.body === 'string' ? JSON.parse(init.body) : {});
        return json(200, makeBookingCreateOutput({ branchId: BRANCH_ID, slotId: SLOT_1_ID }));
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
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAudience('customer');
    setLang('th');
  });

  async function mountOnBranch(): Promise<void> {
    dispose = render(() => <BookScreen fetch={fetchStub} />, host);
    await flush();
    await vi.waitFor(() => expect(host.querySelector(`option[value="${BRANCH_ID}"]`)).not.toBeNull());
    const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = BRANCH_ID;
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();
    const qty = host.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qty.value = '100';
    qty.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();
  }

  /** Submits the form the way a browser does when the slot <select> is disabled: no constraint check stops it. */
  function submitBypassingValidation(): void {
    host.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  }

  it('D1 every slot of the day closed (no slots): submit sends no POST /api/bookings and shows the slot error', async () => {
    slots = slotsAnswer([]);
    await mountOnBranch();
    expect(host.querySelector<HTMLSelectElement>('select[name="slotId"]')!.disabled, 'the slot select is disabled').toBe(
      true,
    );
    submitBypassingValidation();
    await flush();
    expect(bookingBodies).toEqual([]);
    const err = host.querySelector('[data-field-error="slotId"]');
    expect(err, 'the slot field error').not.toBeNull();
    expect(err!.getAttribute('role')).toBe('alert');
    expect(err!.textContent ?? '').not.toBe('');
  });

  it('D2 slots exist but none is chosen: submit sends no POST /api/bookings and shows the slot error', async () => {
    slots = slotsAnswer([FULL_SLOT]);
    await mountOnBranch();
    submitBypassingValidation();
    await flush();
    expect(bookingBodies).toEqual([]);
    expect(host.querySelector('[data-field-error="slotId"]')).not.toBeNull();
  });

  it('D3 with a slot chosen, submit books with that slot as today', async () => {
    slots = slotsAnswer([OPEN_SLOT]);
    await mountOnBranch();
    await vi.waitFor(() => expect(host.querySelector(`option[value="${SLOT_1_ID}"]`)).not.toBeNull());
    const slotSel = host.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = SLOT_1_ID;
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();
    submitBypassingValidation();
    await vi.waitFor(() => expect(bookingBodies).toHaveLength(1));
    expect(bookingBodies[0]!.slotId).toBe(SLOT_1_ID);
    expect(host.querySelector('[data-field-error="slotId"]')).toBeNull();
  });
});
