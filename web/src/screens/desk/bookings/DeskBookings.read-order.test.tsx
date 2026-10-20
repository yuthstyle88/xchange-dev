// #2216: Desk bookings (B02) — out-of-order read sequence guard in fetchBookings (§9 behaviour table).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeStaffMeView,
  makeStaffRoleDto,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { DeskBookingsScreen } from './DeskBookingsScreen';

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const STAFF_TELLER = makeStaffMeView({
  userId: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeStaffRoleDto({ role: 'teller', branchId: 'XC-B01' })],
  branchId: 'XC-B01',
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const INITIAL_BOOKINGS = makeDeskBookingsData({
  items: [
    makeDeskBookingItemDto({
      id: 'bk-1',
      bookingNo: 'B261016-0001',
      status: 'CONFIRMED',
      customerDisplayName: 'Alice Customer',
      phoneLast4: '1234',
      readiness: 'ready',
      ticketNumber: null,
      slotId: 'slot-1',
      startAt: '2026-10-16T09:00:00Z',
      endAt: '2026-10-16T09:30:00Z',
    }),
    makeDeskBookingItemDto({
      id: 'bk-2',
      bookingNo: 'B261016-0002',
      status: 'CONFIRMED',
      customerDisplayName: 'Bob Customer',
      phoneLast4: '5678',
      readiness: 'ready',
      ticketNumber: null,
      slotId: 'slot-1',
      startAt: '2026-10-16T09:00:00Z',
      endAt: '2026-10-16T09:30:00Z',
    }),
  ],
});

const QR_FILTERED_BOOKINGS = makeDeskBookingsData({
  items: [
    makeDeskBookingItemDto({
      id: 'bk-2',
      bookingNo: 'B261016-0002',
      status: 'CONFIRMED',
      customerDisplayName: 'Bob Customer',
      phoneLast4: '5678',
      readiness: 'ready',
      ticketNumber: null,
      slotId: 'slot-1',
      startAt: '2026-10-16T09:00:00Z',
      endAt: '2026-10-16T09:30:00Z',
    }),
  ],
});

const NEWER_BOOKINGS = makeDeskBookingsData({
  items: [
    ...INITIAL_BOOKINGS.items,
    makeDeskBookingItemDto({
      id: 'bk-3',
      bookingNo: 'B261016-0003',
      status: 'CONFIRMED',
      customerDisplayName: 'Charlie Customer',
      phoneLast4: '9012',
      readiness: 'ready',
      ticketNumber: null,
      slotId: 'slot-1',
      startAt: '2026-10-16T09:00:00Z',
      endAt: '2026-10-16T09:30:00Z',
    }),
  ],
});

interface PendingBookingRead {
  since: string | null;
  qr: string | null;
  resolve: (res: Response) => void;
  reject: (err: unknown) => void;
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('DeskBookingsScreen (B02) — §9 read-order sequence guard (#2216)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let pendingReads: PendingBookingRead[];

  beforeEach(() => {
    setAudience('staff');
    setLang('th');
    pendingReads = [];
    host = document.createElement('div');
    document.body.appendChild(host);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
          return Promise.resolve(jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER }));
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
          return new Promise<Response>((resolve, reject) => {
            pendingReads.push({
              since: u.searchParams.get('since'),
              qr: u.searchParams.get('qr'),
              resolve,
              reject,
            });
          });
        }
        return Promise.resolve(jsonResponse(404, { error: { code: 'notFound', message: u.pathname } }));
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAudience('staff');
    setLang('th');
  });

  it("row 1: older read succeeds after a newer read failed -> the newer read's error stays", async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    expect(pendingReads).toHaveLength(1);

    // 1. Initial load succeeds
    pendingReads[0]!.resolve(jsonResponse(200, { version: 1, changed: true, data: INITIAL_BOOKINGS }));
    await flush();
    expect(host.querySelector('.xc-error-banner')).toBeNull();

    // 2. Start an older Refresh read (held open)
    const refreshBtn = host.querySelector<HTMLButtonElement>('button[data-action="refresh"]')!;
    refreshBtn.click();
    await flush();
    expect(pendingReads).toHaveLength(2);
    const olderRefreshRead = pendingReads[1]!;
    expect(olderRefreshRead.since).toBe('1');

    // 3. Start a newer QR lookup read and let it fail first with 500
    const qrInput = host.querySelector<HTMLInputElement>('input[data-action="scan-qr"]')!;
    qrInput.value = 'QR-FAIL-999';
    qrInput.dispatchEvent(new Event('input', { bubbles: true }));
    const lookupBtn = host.querySelector<HTMLButtonElement>('button[data-action="lookup-qr"]')!;
    lookupBtn.click();
    await flush();
    expect(pendingReads).toHaveLength(3);
    const newerQrRead = pendingReads[2]!;
    expect(newerQrRead.qr).toBe('QR-FAIL-999');

    newerQrRead.resolve(jsonResponse(500, { error: { code: 'serverError', message: 'qr lookup failed' } }));
    await flush();
    expect(host.querySelector('.xc-error-banner[role="alert"]')?.textContent).toContain(
      sharedTh['error.serverError'],
    );

    // 4. Now the older held Refresh read answers 200 {version: 1, changed: false}
    olderRefreshRead.resolve(jsonResponse(200, { version: 1, changed: false }));
    await flush();

    // The newer read's error banner must stay visible
    const banner = host.querySelector('.xc-error-banner[role="alert"]');
    expect(banner, 'newer read error banner must stay after older read succeeds').not.toBeNull();
    expect(banner!.textContent).toContain(sharedTh['error.serverError']);
  });

  it('row 2: older read fails after a newer read succeeded -> no error banner; the newer items stay', async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    expect(pendingReads).toHaveLength(1);

    // 1. Initial load succeeds
    pendingReads[0]!.resolve(jsonResponse(200, { version: 1, changed: true, data: INITIAL_BOOKINGS }));
    await flush();
    expect(host.textContent).toContain('B261016-0001');
    expect(host.textContent).not.toContain('B261016-0003');

    // 2. Start an older Refresh read (held open)
    const refreshBtn = host.querySelector<HTMLButtonElement>('button[data-action="refresh"]')!;
    refreshBtn.click();
    await flush();
    expect(pendingReads).toHaveLength(2);
    const olderRead = pendingReads[1]!;

    // 3. Start a newer Refresh read and let it succeed first with NEWER_BOOKINGS
    refreshBtn.click();
    await flush();
    expect(pendingReads).toHaveLength(3);
    const newerRead = pendingReads[2]!;

    newerRead.resolve(jsonResponse(200, { version: 2, changed: true, data: NEWER_BOOKINGS }));
    await flush();
    expect(host.querySelector('.xc-error-banner')).toBeNull();
    expect(host.textContent).toContain('B261016-0003');

    // 4. Now the older Refresh read fails with 500
    olderRead.resolve(jsonResponse(500, { error: { code: 'serverError', message: 'stale failure' } }));
    await flush();

    // No error banner shown; newer items stay
    expect(host.querySelector('.xc-error-banner')).toBeNull();
    expect(host.textContent).toContain('B261016-0003');
  });

  it('row 3: older read with data answers after a newer QR-filtered read -> the QR-filtered items stay', async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    expect(pendingReads).toHaveLength(1);

    // 1. Initial load succeeds with both bookings
    pendingReads[0]!.resolve(jsonResponse(200, { version: 1, changed: true, data: INITIAL_BOOKINGS }));
    await flush();
    expect(host.textContent).toContain('B261016-0001');
    expect(host.textContent).toContain('B261016-0002');

    // 2. Start an older unfiltered Refresh read (held open)
    const refreshBtn = host.querySelector<HTMLButtonElement>('button[data-action="refresh"]')!;
    refreshBtn.click();
    await flush();
    expect(pendingReads).toHaveLength(2);
    const olderUnfilteredRead = pendingReads[1]!;
    expect(olderUnfilteredRead.qr).toBeNull();

    // 3. Start a newer QR-filtered read and let it answer first with only B261016-0002
    const qrInput = host.querySelector<HTMLInputElement>('input[data-action="scan-qr"]')!;
    qrInput.value = 'QR-BOB-0002';
    qrInput.dispatchEvent(new Event('input', { bubbles: true }));
    const lookupBtn = host.querySelector<HTMLButtonElement>('button[data-action="lookup-qr"]')!;
    lookupBtn.click();
    await flush();
    expect(pendingReads).toHaveLength(3);
    const newerQrRead = pendingReads[2]!;
    expect(newerQrRead.qr).toBe('QR-BOB-0002');

    newerQrRead.resolve(jsonResponse(200, { version: 2, changed: true, data: QR_FILTERED_BOOKINGS }));
    await flush();
    expect(host.textContent).toContain('B261016-0002');
    expect(host.textContent).not.toContain('B261016-0001');

    // 4. Now the older unfiltered read answers with INITIAL_BOOKINGS (both items)
    olderUnfilteredRead.resolve(jsonResponse(200, { version: 2, changed: true, data: INITIAL_BOOKINGS }));
    await flush();

    // The QR-filtered items stay on screen
    expect(host.textContent).toContain('B261016-0002');
    expect(host.textContent).not.toContain('B261016-0001');
  });

  it('row 4 (stale Retry): single-read #2163 error/retry flow stays unchanged while ignoring a stale earlier Retry', async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    expect(pendingReads).toHaveLength(1);

    // 1. Initial load fails -> banner + retry button shown (#2163 row 1)
    pendingReads[0]!.resolve(jsonResponse(500, { error: { code: 'serverError', message: 'initial load failed' } }));
    await flush();
    expect(host.querySelector('.xc-error-banner[role="alert"]')?.textContent).toContain(
      sharedTh['error.serverError'],
    );
    const retryBtn = host.querySelector<HTMLButtonElement>('button[data-action="retry"]');
    expect(retryBtn).not.toBeNull();

    // 2. Click Retry (read #2, held), then click Retry again (read #3) which succeeds sequentially
    retryBtn!.click();
    await flush();
    expect(pendingReads).toHaveLength(2);
    const staleRetry = pendingReads[1]!;

    retryBtn!.click();
    await flush();
    expect(pendingReads).toHaveLength(3);
    const latestRetry = pendingReads[2]!;

    latestRetry.resolve(jsonResponse(200, { version: 1, changed: true, data: INITIAL_BOOKINGS }));
    await flush();
    expect(host.querySelector('.xc-error-banner')).toBeNull();
    expect(host.textContent).toContain('B261016-0001');

    // 3. When the earlier retry fails late, it does not bring back the error banner
    staleRetry.resolve(jsonResponse(500, { error: { code: 'serverError', message: 'late retry failure' } }));
    await flush();
    expect(host.querySelector('.xc-error-banner')).toBeNull();
    expect(host.textContent).toContain('B261016-0001');
  });

  it('row 5: QR lookup, then Refresh before it answers, Refresh answers changed:false -> the QR-filtered items show', async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    expect(pendingReads).toHaveLength(1);

    // 1. Initial unfiltered load succeeds at version 1 with both bookings
    pendingReads[0]!.resolve(jsonResponse(200, { version: 1, changed: true, data: INITIAL_BOOKINGS }));
    await flush();
    expect(host.textContent).toContain('B261016-0001');
    expect(host.textContent).toContain('B261016-0002');

    // 2. Trigger QR lookup (held open) — changing QR clears the held unfiltered version
    const qrInput = host.querySelector<HTMLInputElement>('input[data-action="scan-qr"]')!;
    qrInput.value = 'QR-BOB-0002';
    qrInput.dispatchEvent(new Event('input', { bubbles: true }));
    const lookupBtn = host.querySelector<HTMLButtonElement>('button[data-action="lookup-qr"]')!;
    lookupBtn.click();
    await flush();
    expect(pendingReads).toHaveLength(2);
    const qrLookupRead = pendingReads[1]!;
    expect(qrLookupRead.qr).toBe('QR-BOB-0002');
    expect(qrLookupRead.since).toBeNull();

    // 3. Click Refresh before the QR lookup answers — must not send the unfiltered since=1
    const refreshBtn = host.querySelector<HTMLButtonElement>('button[data-action="refresh"]')!;
    refreshBtn.click();
    await flush();
    expect(pendingReads).toHaveLength(3);
    const refreshRead = pendingReads[2]!;
    expect(refreshRead.qr).toBe('QR-BOB-0002');

    // 4. Refresh answers changed:false first; it must not stand in for the unreceived QR filter data
    refreshRead.resolve(jsonResponse(200, { version: 1, changed: false }));
    await flush();

    // 5. The in-flight QR lookup now answers with QR_FILTERED_BOOKINGS -> the QR-filtered items show
    qrLookupRead.resolve(jsonResponse(200, { version: 2, changed: true, data: QR_FILTERED_BOOKINGS }));
    await flush();

    expect(host.textContent).toContain('B261016-0002');
    expect(host.textContent).not.toContain('B261016-0001');
    expect(refreshRead.since).toBeNull();
  });
});
