import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeStaffMeView,
  makeStaffRoleDto,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { DeskBookingsScreen } from './DeskBookingsScreen';

const BRANCH_ID = 'XC-B01';
const SLOT_ID = '00000000-0000-0000-0000-000000000001';

const STAFF_TELLER = makeStaffMeView({
  userId: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeStaffRoleDto({ role: 'teller', branchId: BRANCH_ID })],
  branchId: BRANCH_ID,
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const STATUS_CASES = [
  { status: 'PENDING_CUSTOMER_CONFIRMATION', checkIn: false, cancel: true, reschedule: true, reissueQr: true },
  { status: 'CONFIRMED', checkIn: true, cancel: true, reschedule: true, reissueQr: true },
  { status: 'CHECKED_IN', checkIn: false, cancel: false, reschedule: false, reissueQr: true },
  { status: 'CANCELLED', checkIn: false, cancel: false, reschedule: false, reissueQr: false },
  { status: 'EXPIRED', checkIn: false, cancel: false, reschedule: false, reissueQr: false },
  { status: 'NO_SHOW', checkIn: false, cancel: false, reschedule: false, reissueQr: false },
  { status: 'COMPLETED', checkIn: false, cancel: false, reschedule: false, reissueQr: false },
] as const;

const BOOKINGS_DATA = makeDeskBookingsData({
  items: STATUS_CASES.map((c, i) =>
    makeDeskBookingItemDto({
      id: `bk-${c.status.toLowerCase()}`,
      bookingNo: `B261020-000${i + 1}`,
      status: c.status,
      customerDisplayName: `Customer ${c.status}`,
      phoneLast4: `${1000 + i}`,
      readiness: 'unknown',
      ticketNumber: c.status === 'CHECKED_IN' ? 'T-001' : null,
      slotId: SLOT_ID,
      startAt: '2026-10-20T09:00:00Z',
      endAt: '2026-10-20T09:30:00Z',
    }),
  ),
});

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

describe('DeskBookingsScreen status-based action buttons (#3849)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeAll(() => warmModules(() => import('./DeskBookingsScreen')), 60_000);

  beforeEach(() => {
    setAudience('staff');
    setLang('en');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (u.pathname === '/api/v3/views/staff_me') {
          return viewEnvelope(STAFF_TELLER);
        }
        if (u.pathname === '/api/v3/views/desk_bookings') {
          return viewEnvelope(BOOKINGS_DATA);
        }
        if (method === 'POST' && u.pathname === '/api/bookings/bk-confirmed/check-in') {
          return json(200, {
            id: 'bk-confirmed',
            bookingNo: 'B261020-0002',
            status: 'CHECKED_IN',
            ticketNumber: 'T-002',
          });
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

  it('renders one booking per status and asserts which buttons exist per §9 behaviour table', async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    for (const c of STATUS_CASES) {
      const item = host.querySelector(`[data-booking-id="bk-${c.status.toLowerCase()}"]`);
      expect(item, `booking row for ${c.status} exists`).not.toBeNull();

      const checkInBtn = item!.querySelector('button[data-action="check-in"]');
      const cancelBtn = item!.querySelector('button[data-action="cancel"]');
      const rescheduleBtn = item!.querySelector('button[data-action="reschedule"]');
      const reissueQrBtn = item!.querySelector('button[data-action="reissue-qr"]');

      if (c.checkIn) {
        expect(checkInBtn, `${c.status} should have check-in button`).not.toBeNull();
      } else {
        expect(checkInBtn, `${c.status} should NOT have check-in button`).toBeNull();
      }

      if (c.cancel) {
        expect(cancelBtn, `${c.status} should have cancel button`).not.toBeNull();
      } else {
        expect(cancelBtn, `${c.status} should NOT have cancel button`).toBeNull();
      }

      if (c.reschedule) {
        expect(rescheduleBtn, `${c.status} should have reschedule button`).not.toBeNull();
      } else {
        expect(rescheduleBtn, `${c.status} should NOT have reschedule button`).toBeNull();
      }

      if (c.reissueQr) {
        expect(reissueQrBtn, `${c.status} should have reissue-qr button`).not.toBeNull();
      } else {
        expect(reissueQrBtn, `${c.status} should NOT have reissue-qr button`).toBeNull();
      }
    }
  });

  it('after a check-in on this screen, cancel and reschedule disappear', async () => {
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();

    const item = host.querySelector('[data-booking-id="bk-confirmed"]');
    expect(item).not.toBeNull();

    expect(item!.querySelector('button[data-action="check-in"]')).not.toBeNull();
    expect(item!.querySelector('button[data-action="cancel"]')).not.toBeNull();
    expect(item!.querySelector('button[data-action="reschedule"]')).not.toBeNull();
    expect(item!.querySelector('button[data-action="reissue-qr"]')).not.toBeNull();

    const checkInBtn = item!.querySelector('button[data-action="check-in"]') as HTMLButtonElement;
    checkInBtn.click();
    await flush();

    const updatedItem = host.querySelector('[data-booking-id="bk-confirmed"]');
    expect(updatedItem).not.toBeNull();
    expect(updatedItem!.querySelector('button[data-action="check-in"]')).toBeNull();
    expect(updatedItem!.querySelector('button[data-action="cancel"]')).toBeNull();
    expect(updatedItem!.querySelector('button[data-action="reschedule"]')).toBeNull();
    expect(updatedItem!.querySelector('button[data-action="reissue-qr"]')).not.toBeNull();
  });
});
