// #6466 (UX-6): booking success scrolls into view once in DOM; safe when scrollIntoView is missing.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingCreateOutput,
  makeBookingSlotItemDto,
  makeBookingSlotsData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { BookScreen } from './Book';

const BRANCH_ID = 'XC-B01';
const SLOT_1_ID = '00000000-0000-4000-8000-000000000001';

const SAMPLE_BRANCHES = {
  branches: [
    {
      id: BRANCH_ID,
      code: BRANCH_ID,
      name: 'สาขาสยามพารากอน',
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
    },
  ],
};

const SAMPLE_SLOTS = {
  version: 1,
  changed: true,
  data: makeBookingSlotsData({
    slots: [
      makeBookingSlotItemDto({
        id: SLOT_1_ID,
        start: '2026-10-15T03:00:00+00:00',
        end: '2026-10-15T03:30:00+00:00',
        available: 3,
      }),
    ],
  }),
};

const SAMPLE_PREVIEW = {
  thb: '3542.00',
  rate: '35.42000000',
  unitSize: '1',
  stale: false,
  sourceDate: '2026-10-15',
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('BookScreen success scrollIntoView (#6466)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let createStatus = 200;
  let createResponse: unknown;
  const originalScrollIntoView = HTMLElement.prototype.scrollIntoView;

  beforeAll(() => warmModules(() => import('./Book')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    window.history.replaceState({}, '', '/app/book');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    createStatus = 200;
    createResponse = makeBookingCreateOutput({
      id: 'BK-6466',
      bookingNo: 'B261106-0001',
      status: 'CONFIRMED',
      qrToken: 'qr-tok-6466',
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    window.history.replaceState({}, '', '/app/book');
    if (originalScrollIntoView === undefined) {
      delete (HTMLElement.prototype as { scrollIntoView?: unknown }).scrollIntoView;
    } else {
      HTMLElement.prototype.scrollIntoView = originalScrollIntoView;
    }
    vi.restoreAllMocks();
  });

  async function mountAndSubmit(): Promise<void> {
    const fetchStub = vi.fn(async (input: string, init?: RequestInit): Promise<Response> => {
      const u = new URL(input, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (u.pathname === '/api/public/host') {
        return json(200, { org: 'XC', env: 'test' });
      }
      if (u.pathname.startsWith('/api/public/orgs/')) {
        return json(200, SAMPLE_BRANCHES);
      }
      if (u.pathname === '/api/v3/views/booking_slots') {
        return json(200, SAMPLE_SLOTS);
      }
      if (u.pathname === '/api/v3/compute/planner_preview' && method === 'POST') {
        return json(200, SAMPLE_PREVIEW);
      }
      if (u.pathname === '/api/bookings' && method === 'POST') {
        return json(createStatus, createResponse);
      }
      return json(404, { error: { code: 'notFound', message: u.pathname } });
    });

    dispose = render(() => <BookScreen fetch={fetchStub} />, host);
    await flush();

    await vi.waitFor(() => {
      expect(host.querySelector(`option[value="${BRANCH_ID}"]`)).not.toBeNull();
    });

    const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = BRANCH_ID;
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(host.querySelector(`option[value="${SLOT_1_ID}"]`)).not.toBeNull();
    });

    const slotSel = host.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = SLOT_1_ID;
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));

    const qtyInput = host.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    submitBtn.click();
    await flush();
  }

  it('calls scrollIntoView once on .xc-book-success with { block: "start", behavior: "smooth" } when a booking succeeds', async () => {
    const scrollSpy = vi.fn();
    HTMLElement.prototype.scrollIntoView = scrollSpy;

    await mountAndSubmit();

    const banner = host.querySelector('.xc-book-success');
    expect(banner).not.toBeNull();
    expect(scrollSpy).toHaveBeenCalledTimes(1);
    expect(scrollSpy).toHaveBeenCalledWith({ block: 'start', behavior: 'smooth' });
    expect(scrollSpy.mock.instances[0]).toBe(banner);

    const link = banner!.querySelector<HTMLAnchorElement>('a.xc-book-bookings-link');
    expect(link).not.toBeNull();
    expect(link!.getAttribute('href')).toBe('/app/bookings');
    expect(link!.textContent).toBe('ดูการจองของฉัน');
  });

  it('does not call scrollIntoView when the booking fails', async () => {
    const scrollSpy = vi.fn();
    HTMLElement.prototype.scrollIntoView = scrollSpy;
    createStatus = 500;
    createResponse = { error: { code: 'internalError', message: 'failed' } };

    await mountAndSubmit();

    expect(host.querySelector('.xc-book-success')).toBeNull();
    expect(scrollSpy).not.toHaveBeenCalled();
  });

  it('does not throw when scrollIntoView is undefined', async () => {
    delete (HTMLElement.prototype as { scrollIntoView?: unknown }).scrollIntoView;

    await expect(mountAndSubmit()).resolves.toBeUndefined();

    const banner = host.querySelector('.xc-book-success');
    expect(banner).not.toBeNull();
  });
});
