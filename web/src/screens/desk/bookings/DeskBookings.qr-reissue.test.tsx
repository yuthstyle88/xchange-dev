import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { BookingQrReissueOutput } from '../../../shared/api/generated/bookings';
import type { DeskBookingsData } from '../../../shared/api/generated/desk_bookings';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { en } from './i18n';

const BRANCH_ID = 'XC-B01';
const SLOT_1_ID = '00000000-0000-0000-0000-000000000001';

const STAFF_TELLER: StaffMeView = {
  userId: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [{ role: 'teller', branchId: BRANCH_ID }],
  branchId: BRANCH_ID,
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
};

const BOOKINGS_DATA: DeskBookingsData = {
  items: [
    {
      id: 'bk-confirmed',
      bookingNo: 'B261014-0001',
      status: 'CONFIRMED',
      customerDisplayName: 'Alice Confirmed',
      phoneLast4: '1111',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: SLOT_1_ID,
      startAt: '2026-10-14T09:00:00Z',
      endAt: '2026-10-14T09:30:00Z',
    },
    {
      id: 'bk-checked-in',
      bookingNo: 'B261014-0002',
      status: 'CHECKED_IN',
      customerDisplayName: 'Bob CheckedIn',
      phoneLast4: '2222',
      readiness: 'unknown',
      ticketNumber: 'T-002',
      slotId: SLOT_1_ID,
      startAt: '2026-10-14T09:00:00Z',
      endAt: '2026-10-14T09:30:00Z',
    },
    {
      id: 'bk-pending',
      bookingNo: 'B261014-0003',
      status: 'PENDING_CUSTOMER_CONFIRMATION',
      customerDisplayName: 'Charlie Pending',
      phoneLast4: '3333',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: SLOT_1_ID,
      startAt: '2026-10-14T09:00:00Z',
      endAt: '2026-10-14T09:30:00Z',
    },
    {
      id: 'bk-cancelled',
      bookingNo: 'B261014-0004',
      status: 'CANCELLED',
      customerDisplayName: 'Dave Cancelled',
      phoneLast4: '4444',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: SLOT_1_ID,
      startAt: '2026-10-14T09:00:00Z',
      endAt: '2026-10-14T09:30:00Z',
    },
    {
      id: 'bk-noshow',
      bookingNo: 'B261014-0005',
      status: 'NO_SHOW',
      customerDisplayName: 'Eve NoShow',
      phoneLast4: '5555',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: SLOT_1_ID,
      startAt: '2026-10-14T09:00:00Z',
      endAt: '2026-10-14T09:30:00Z',
    },
    {
      id: 'bk-completed',
      bookingNo: 'B261014-0006',
      status: 'COMPLETED',
      customerDisplayName: 'Frank Completed',
      phoneLast4: '6666',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: SLOT_1_ID,
      startAt: '2026-10-14T09:00:00Z',
      endAt: '2026-10-14T09:30:00Z',
    },
  ],
};

interface RecordedRequest {
  method: string;
  url: string;
  path: string;
  headers: Headers;
  rawBody: string | undefined;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewEnvelope(data: unknown, version = 1, changed = true): Response {
  return json(200, { version, changed, data });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('FE-desk F1 (B02): QR reissue row action and modal (#3335)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: RecordedRequest[];
  let qrResponder: (req: RecordedRequest) => Response | Promise<Response>;
  let deskBookingsFetchCount: number;

  beforeAll(() => warmModules(() => import('./DeskBookingsScreen')), 60_000);

  beforeEach(() => {
    setAudience('staff');
    setLang('en');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    sentRequests = [];
    deskBookingsFetchCount = 0;

    qrResponder = () => {
      return json(200, {
        id: 'bk-confirmed',
        qrToken: 'test-qr-token-xyz',
        qrVersion: 2,
      } satisfies BookingQrReissueOutput);
    };

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        const headers = new Headers(init?.headers);
        const rawBody = typeof init?.body === 'string' ? init.body : undefined;

        const rec: RecordedRequest = {
          method,
          url: rawUrl,
          path: u.pathname,
          headers,
          rawBody,
        };
        sentRequests.push(rec);

        if (u.pathname === '/api/v3/views/staff_me') {
          return viewEnvelope(STAFF_TELLER);
        }
        if (u.pathname === '/api/v3/views/desk_bookings') {
          deskBookingsFetchCount++;
          return viewEnvelope(BOOKINGS_DATA);
        }
        if (u.pathname.startsWith('/api/bookings/') && u.pathname.endsWith('/qr')) {
          return qrResponder(rec);
        }

        return json(404, { error: { code: 'notFound', message: u.pathname } });
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('row status CONFIRMED, CHECKED_IN or PENDING_CUSTOMER_CONFIRMATION -> button[name=reissueQr] shows', async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const items = host.querySelectorAll('.xc-booking-item');
    expect(items.length).toBe(6);

    const confirmedItem = items[0];
    const checkedInItem = items[1];
    const pendingItem = items[2];

    expect(confirmedItem.querySelector('button[name="reissueQr"]')).not.toBeNull();
    expect(checkedInItem.querySelector('button[name="reissueQr"]')).not.toBeNull();
    expect(pendingItem.querySelector('button[name="reissueQr"]')).not.toBeNull();
  });

  it('any other status (CANCELLED, NO_SHOW, COMPLETED, …) -> no reissueQr button', async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const items = host.querySelectorAll('.xc-booking-item');
    const cancelledItem = items[3];
    const noShowItem = items[4];
    const completedItem = items[5];

    expect(cancelledItem.querySelector('button[name="reissueQr"]')).toBeNull();
    expect(noShowItem.querySelector('button[name="reissueQr"]')).toBeNull();
    expect(completedItem.querySelector('button[name="reissueQr"]')).toBeNull();
  });

  it('click, answer qrToken: "tok" -> one POST /api/bookings/<id>/qr with an Idempotency-Key; a QR of tok shows; the list reads again once', async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const initialFetchCount = deskBookingsFetchCount;
    const items = host.querySelectorAll('.xc-booking-item');
    const btn = items[0].querySelector<HTMLButtonElement>('button[name="reissueQr"]')!;
    expect(btn).not.toBeNull();

    btn.click();
    await flush();

    const qrRequests = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === '/api/bookings/bk-confirmed/qr',
    );
    expect(qrRequests.length).toBe(1);
    const req = qrRequests[0];
    expect(req.headers.get('x-xchange-request')).toBe('1');
    expect(req.headers.get('idempotency-key')).toBeTruthy();
    expect(req.rawBody).toBeTruthy();
    const parsedBody = JSON.parse(req.rawBody!);
    expect(parsedBody.id).toBe('bk-confirmed');
    expect(parsedBody.branch).toBe(BRANCH_ID);

    // Modal with QR should be displayed
    const modal = host.querySelector('.xc-modal');
    expect(modal).not.toBeNull();
    expect(modal?.textContent).toContain('B261014-0001');

    // QR SVG image should be present
    const qrImg = modal?.querySelector('[role="img"]');
    expect(qrImg).not.toBeNull();

    // List reads again once after reissue
    expect(deskBookingsFetchCount).toBe(initialFetchCount + 1);
  });

  it('answer qrToken: null (replay) -> no QR; the "already issued" text shows', async () => {
    qrResponder = () => {
      return json(200, {
        id: 'bk-confirmed',
        qrToken: null,
        qrVersion: 2,
      } satisfies BookingQrReissueOutput);
    };

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const items = host.querySelectorAll('.xc-booking-item');
    const btn = items[0].querySelector<HTMLButtonElement>('button[name="reissueQr"]')!;
    btn.click();
    await flush();

    const modal = host.querySelector('.xc-modal');
    expect(modal).not.toBeNull();
    expect(modal?.textContent).toContain('B261014-0001');

    // No QR code rendered
    const qrImg = modal?.querySelector('[role="img"]');
    expect(qrImg).toBeNull();

    // The "already issued" text is present
    expect(modal?.textContent?.toLowerCase()).toContain('already issued');
  });

  it('refusal staleVersion / notFound -> the screen\'s error mapping shows it; no QR', async () => {
    qrResponder = () => {
      return json(409, { error: { code: 'staleVersion', message: 'stale' } });
    };

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const items = host.querySelectorAll('.xc-booking-item');
    const btn = items[0].querySelector<HTMLButtonElement>('button[name="reissueQr"]')!;
    btn.click();
    await flush();

    // No modal dialog
    const modal = host.querySelector('.xc-modal');
    expect(modal).toBeNull();

    // Error banner displayed with mapped message
    const errorBanner = host.querySelector('.xc-error-banner');
    expect(errorBanner).not.toBeNull();
    expect(errorBanner?.textContent).toContain(en['error.staleVersion']);
  });

  it('a second click while one is in flight -> no second request', async () => {
    let resolveWrite!: (resp: Response) => void;
    qrResponder = () =>
      new Promise<Response>((r) => {
        resolveWrite = r;
      });

    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const items = host.querySelectorAll('.xc-booking-item');
    const btn = items[0].querySelector<HTMLButtonElement>('button[name="reissueQr"]')!;

    btn.click();
    await flush();

    const qrRequests1 = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === '/api/bookings/bk-confirmed/qr',
    );
    expect(qrRequests1.length).toBe(1);

    // Click again while first request is in flight
    btn.click();
    await flush();

    const qrRequests2 = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === '/api/bookings/bk-confirmed/qr',
    );
    expect(qrRequests2.length).toBe(1);

    // Resolve the in-flight request
    resolveWrite(
      json(200, {
        id: 'bk-confirmed',
        qrToken: 'resolved-token',
        qrVersion: 2,
      }),
    );
    await flush();

    const qrRequests3 = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === '/api/bookings/bk-confirmed/qr',
    );
    expect(qrRequests3.length).toBe(1);
  });
});
