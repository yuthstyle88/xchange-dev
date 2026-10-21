// #4327 Customer /app/book success screen shows check-in QR, desk hint, download button, and link to /app/bookings.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingCreateOutput,
  makeBookingSlotItemDto,
  makeBookingSlotsData,
  makeMyBookingItemDto,
  makeMyBookingsData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { BookingsScreen } from '../bookings/Bookings';
import * as bookingsI18n from '../bookings/i18n';
import { BookScreen } from './Book';
import * as bookI18n from './i18n';

const BRANCH_ID = 'XC-B01';
const SLOT_1_ID = '00000000-0000-4000-8000-000000000001';

const SAMPLE_BRANCHES = {
  branches: [
    {
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

describe('BookScreen success QR, download & bookings link (#4327)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let createResponse: unknown;
  const originalCreateObjectURL = URL.createObjectURL;
  const originalRevokeObjectURL = URL.revokeObjectURL;

  beforeAll(() => warmModules(() => import('./Book')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    window.history.replaceState({}, '', '/app/book');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    window.history.replaceState({}, '', '/app/book');
    URL.createObjectURL = originalCreateObjectURL;
    URL.revokeObjectURL = originalRevokeObjectURL;
    vi.unstubAllGlobals();
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
        return json(200, createResponse);
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

  it('renders .xc-booking-qr[data-qr-token], desk instruction, download button (booking-<bookingNo>.png), and /app/bookings link when qrToken is non-null', async () => {
    createResponse = makeBookingCreateOutput({
      id: 'BK-4327',
      bookingNo: 'B261021-0099',
      status: 'CONFIRMED',
      qrToken: 'qr-tok-99',
    });

    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
      drawImage: vi.fn(),
      fillRect: vi.fn(),
      fillStyle: '',
    } as unknown as CanvasRenderingContext2D);
    vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation((cb: BlobCallback) =>
      cb(new Blob(['png'], { type: 'image/png' })),
    );
    URL.createObjectURL = vi.fn(() => 'blob:http://xc.test/book-qr') as unknown as typeof URL.createObjectURL;
    let downloadedFile = '';
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      downloadedFile = this.download;
    });

    await mountAndSubmit();

    const banner = host.querySelector('.xc-book-success');
    expect(banner).not.toBeNull();

    const qrWrap = banner!.querySelector('.xc-booking-qr');
    expect(qrWrap).not.toBeNull();
    expect(qrWrap!.getAttribute('data-qr-token')).toBe('qr-tok-99');
    expect(qrWrap!.querySelector('svg')).not.toBeNull();

    expect(banner!.textContent).toContain(bookI18n.th.qrShowAtDesk);

    const downloadBtn = qrWrap!.querySelector<HTMLButtonElement>('button.xc-qr-download');
    expect(downloadBtn).not.toBeNull();
    expect(downloadBtn!.textContent).toBe(bookI18n.th.downloadQr);
    downloadBtn!.click();
    expect(downloadedFile).toBe('booking-B261021-0099.png');

    const link = banner!.querySelector<HTMLAnchorElement>('a[href="/app/bookings"]');
    expect(link).not.toBeNull();
    expect(link!.textContent).toBe('ดูการจองของฉัน');
  });

  it('renders no QR and no download button when qrToken is null, but shows the /app/bookings link', async () => {
    createResponse = makeBookingCreateOutput({
      id: 'BK-null-qr',
      bookingNo: 'B261021-0100',
      status: 'CONFIRMED',
      qrToken: null,
    });

    await mountAndSubmit();

    const banner = host.querySelector('.xc-book-success');
    expect(banner).not.toBeNull();
    expect(banner!.querySelector('.xc-booking-qr')).toBeNull();
    expect(banner!.querySelector('svg')).toBeNull();
    expect(banner!.querySelector('button.xc-qr-download')).toBeNull();
    expect(banner!.textContent).not.toContain(bookI18n.th.qrShowAtDesk);

    const link = banner!.querySelector<HTMLAnchorElement>('a[href="/app/bookings"]');
    expect(link).not.toBeNull();
    expect(link!.textContent).toBe('ดูการจองของฉัน');
  });

  it('renders the QR download button on booking cards in Bookings.tsx saving booking-<bookingNo>.png', async () => {
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
      drawImage: vi.fn(),
      fillRect: vi.fn(),
      fillStyle: '',
    } as unknown as CanvasRenderingContext2D);
    vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation((cb: BlobCallback) =>
      cb(new Blob(['png'], { type: 'image/png' })),
    );
    URL.createObjectURL = vi.fn(() => 'blob:http://xc.test/bookings-qr') as unknown as typeof URL.createObjectURL;
    let downloadedFile = '';
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      downloadedFile = this.download;
    });

    const fetchStub = vi.fn(async (input: string): Promise<Response> => {
      const u = new URL(input, 'http://xc.test');
      if (u.pathname === '/api/public/host') return json(200, { org: 'XC' });
      if (u.pathname === '/api/public/orgs/XC/branches') return json(200, SAMPLE_BRANCHES);
      if (u.pathname === '/api/v3/views/my_bookings') {
        return json(200, {
          version: 1,
          changed: true,
          data: makeMyBookingsData({
            bookings: [
              makeMyBookingItemDto({
                id: 'bk-card-1',
                bookingNo: 'B261021-0777',
                status: 'CONFIRMED',
                branchId: BRANCH_ID,
                slotId: SLOT_1_ID,
                startAt: '2026-10-21T10:00:00Z',
                endAt: '2026-10-21T10:30:00Z',
                qrToken: 'qr-card-777',
              }),
            ],
          }),
        });
      }
      return json(404, { error: { code: 'notFound' } });
    });

    dispose = render(() => <BookingsScreen fetch={fetchStub} />, host);
    await flush();

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-booking-qr[data-qr-token="qr-card-777"]')).not.toBeNull();
    });

    const qrWrap = host.querySelector('.xc-booking-qr[data-qr-token="qr-card-777"]')!;
    const downloadBtn = qrWrap.querySelector<HTMLButtonElement>('button.xc-qr-download');
    expect(downloadBtn).not.toBeNull();
    expect(downloadBtn!.textContent).toBe(bookingsI18n.th.downloadQr);
    downloadBtn!.click();
    expect(downloadedFile).toBe('booking-B261021-0777.png');
  });

  it('provides th, en and zh translations for all new keys in book and bookings i18n', () => {
    for (const table of [bookI18n.th, bookI18n.en, bookI18n.zh]) {
      expect(table.qrCode.trim().length).toBeGreaterThan(0);
      expect(table.qrShowAtDesk.trim().length).toBeGreaterThan(0);
      expect(table.downloadQr.trim().length).toBeGreaterThan(0);
      expect(table.viewMyBookings.trim().length).toBeGreaterThan(0);
    }
    for (const table of [bookingsI18n.th, bookingsI18n.en, bookingsI18n.zh]) {
      expect(table.downloadQr.trim().length).toBeGreaterThan(0);
    }
  });
});
