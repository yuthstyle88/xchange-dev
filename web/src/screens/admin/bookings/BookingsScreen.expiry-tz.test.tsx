// #2520: Admin bookings — formatExpiry must format expiresAt in Asia/Bangkok timezone
// rather than the device's local timezone.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import type { AdminBookingRow } from '../../../shared/api/generated/admin_bookings';
import { closeDialog, Dialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';
import { BookingsScreen } from './BookingsScreen';

const ISO_EXPIRES = '2026-10-15T18:30:00Z'; // 16 Oct 2026 01:30:00 in Asia/Bangkok (+07:00)

function makeBookingRow(over: Partial<AdminBookingRow> = {}): AdminBookingRow {
  return {
    id: 'BK-2520',
    branchId: 'MAIN',
    name: 'Anan Walkin',
    phone: '02 111 2222',
    member: false,
    type: 'SELL',
    currency: 'EUR',
    amount: '250.50',
    rate: '40.1000',
    status: 'CONFIRMED',
    createdAt: '2026-10-15T18:00:00Z',
    expiresAt: ISO_EXPIRES,
    transactionRef: null,
    counterRate: null,
    counterPrincipal: null,
    ...over,
  };
}

const DEFAULT_SETTINGS = {
  rules: { enabled: true, minutes: 30 },
  memberCount: 2,
  publicUrl: '',
  counterRates: [{ currency: 'EUR', buy: '39.5000', sell: '40.1000' }],
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('BookingsScreen expiry column Asia/Bangkok timezone (#2520)', () => {
  let container: HTMLDivElement | undefined;
  let dispose: (() => void) | undefined;
  let serverRows: AdminBookingRow[];

  beforeEach(() => {
    vi.stubEnv('TZ', 'UTC');
    setLang('th');
    serverRows = [makeBookingRow()];

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input), 'http://desk.test');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (PAGE_WIDE.includes(url.pathname)) {
          return jsonResponse({
            version: 1,
            changed: true,
            data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN' },
          });
        }
        if (method === 'GET' && url.pathname === '/api/v3/views/admin_bookings') {
          return jsonResponse({ version: 1, changed: true, data: serverRows });
        }
        if (method === 'GET' && url.pathname === '/api/v3/views/admin_booking_settings') {
          return jsonResponse({ version: 1, changed: true, data: DEFAULT_SETTINGS });
        }
        if (method === 'GET' && url.pathname === '/api/v3/views/admin_branches') {
          return jsonResponse({
            version: 1,
            changed: true,
            data: { branches: [{ id: 'MAIN', name: 'XC-B' }] },
          });
        }
        return jsonResponse({ error: { code: 'notFound', message: 'not found' } }, 404);
      }),
    );
  });

  afterEach(() => {
    closeDialog();
    dispose?.();
    dispose = undefined;
    container?.remove();
    container = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
    setLang('th');
  });

  async function mountAndSettle(): Promise<HTMLDivElement> {
    dispose?.();
    container?.remove();
    const [branch] = createSignal('MAIN');
    const [shown] = createSignal(true);
    const period = () => ({ from: '', to: '' });
    const el = document.createElement('div');
    container = el;
    document.body.appendChild(el);
    dispose = render(
      () =>
        (
          <>
            <BookingsScreen branch={branch} period={period} shown={shown} />
            <Dialog />
            <Toast />
          </>
        ) as JSX.Element,
      el,
    );
    await vi.waitFor(() => {
      expect(el.querySelectorAll('tbody tr').length).toBe(serverRows.length);
    });
    return el;
  }

  function expiryCellOf(el: HTMLElement, bookingId: string): string {
    const rows = [...el.querySelectorAll<HTMLTableRowElement>('tbody tr')];
    const row = rows.find((r) => (r.textContent ?? '').includes(bookingId));
    expect(row, `row for ${bookingId}`).toBeDefined();
    const cells = [...row!.querySelectorAll('td')].map((td) => (td.textContent ?? '').replace(/\s+/g, ' ').trim());
    return cells[3] ?? '';
  }

  it('row 1: Booking expiresAt 2026-10-15T18:30:00Z, device TZ UTC, Thai shows 16/10/2569 01:30:00', async () => {
    const el = await mountAndSettle();
    expect(expiryCellOf(el, 'BK-2520')).toBe('16/10/2569 01:30:00');
  });

  it('row 2: Same booking with device TZ Asia/Bangkok shows the identical 16/10/2569 01:30:00 as device TZ UTC', async () => {
    const elUtc = await mountAndSettle();
    const textUnderUtc = expiryCellOf(elUtc, 'BK-2520');

    vi.stubEnv('TZ', 'Asia/Bangkok');
    const elBkk = await mountAndSettle();
    const textUnderBangkok = expiryCellOf(elBkk, 'BK-2520');

    expect(textUnderBangkok).toBe('16/10/2569 01:30:00');
    expect(textUnderUtc).toBe(textUnderBangkok);
  });

  it('row 3: expiresAt null shows the pending-staff text alongside Bangkok-formatted non-null expiresAt', async () => {
    serverRows = [
      makeBookingRow({
        id: 'BK-null',
        status: 'PENDING',
        expiresAt: null,
      }),
      makeBookingRow({
        id: 'BK-2520',
        status: 'CONFIRMED',
        expiresAt: ISO_EXPIRES,
      }),
    ];
    const el = await mountAndSettle();
    expect(expiryCellOf(el, 'BK-null')).toBe('รอพนักงานกำหนด');
    expect(expiryCellOf(el, 'BK-2520')).toBe('16/10/2569 01:30:00');
  });

  it('edge: English locale formats 2026-10-15T18:30:00Z as Bangkok wall-clock time 16/10/2026, 01:30:00', async () => {
    setLang('en');
    const el = await mountAndSettle();
    expect(expiryCellOf(el, 'BK-2520')).toBe('16/10/2026, 01:30:00');
  });
});
