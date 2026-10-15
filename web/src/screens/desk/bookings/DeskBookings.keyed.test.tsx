// #2023 KW-4 Phase 1: DeskBookingsScreen idempotency-key retry behaviour (§9 rows 1–5) through HTTP only.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { DeskBookingsData } from '../../../shared/api/generated/desk_bookings';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import { setAudience, setLang } from '../../../shared/i18n';
import { DeskBookingsScreen } from './DeskBookingsScreen';

interface WriteCall {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

type StepAnswer =
  | { kind: 'network' }
  | { kind: 'http'; status: number; body: unknown };

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const STAFF_TELLER: StaffMeView = {
  userId: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [{ role: 'teller', branchId: 'XC-B01' }],
  branchId: 'XC-B01',
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
};

const DESK_BOOKINGS_DATA: DeskBookingsData = {
  items: [
    {
      id: 'bk-1',
      bookingNo: 'B261015-0001',
      status: 'CONFIRMED',
      customerDisplayName: 'Somchai Jaidee',
      phoneLast4: '5678',
      readiness: 'unknown',
      ticketNumber: null,
      slotId: 'slot-1',
      startAt: '2026-10-15T09:00:00Z',
      endAt: '2026-10-15T09:30:00Z',
    },
  ],
};

function makeHarness(steps: StepAnswer[]) {
  const writes: WriteCall[] = [];
  let stepIdx = 0;

  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(rawUrl, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();

    if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
      return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
      return jsonResponse(200, { version: 1, changed: true, data: DESK_BOOKINGS_DATA });
    }

    if (method === 'POST' && u.pathname === '/api/bookings/bk-1/cancel') {
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? init.body : '';
      writes.push({
        method,
        path: u.pathname,
        key: headers.get('idempotency-key'),
        body,
      });
      const step = steps[stepIdx++] ?? {
        kind: 'http',
        status: 200,
        body: { id: 'bk-1', status: 'CANCELLED', bookingNo: 'B261015-0001' },
      };
      if (step.kind === 'network') {
        throw new TypeError('Failed to fetch');
      }
      return jsonResponse(step.status, step.body);
    }

    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  vi.stubGlobal('fetch', fetchFn);

  return { writes };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('DeskBookingsScreen.tsx — §9 idempotency-key retry behaviour (#2023 KW-4)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('staff');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
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

  async function mountAndOpenCancel(): Promise<void> {
    dispose = render(() => <DeskBookingsScreen />, host);
    await vi.waitFor(() => {
      expect(host.querySelector('[data-booking-id="bk-1"] button[data-action="cancel"]')).not.toBeNull();
    });
    const cancelBtn = host.querySelector('[data-booking-id="bk-1"] button[data-action="cancel"]') as HTMLButtonElement;
    cancelBtn.click();
    await vi.waitFor(() => {
      expect(host.querySelector('#cancel-reason-input')).not.toBeNull();
    });
  }

  function fillReason(value: string): void {
    const input = host.querySelector('#cancel-reason-input') as HTMLInputElement;
    expect(input).not.toBeNull();
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function clickConfirmCancel(): void {
    const btn = host.querySelector('button[data-action="confirm-cancel"]') as HTMLButtonElement;
    expect(btn).not.toBeNull();
    btn.click();
  }

  it('row 1: submit → network TypeError → submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    const { writes } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: { id: 'bk-1', status: 'CANCELLED', bookingNo: 'B261015-0001' } },
    ]);

    await mountAndOpenCancel();
    fillReason('ลูกค้าแจ้งยกเลิก');

    clickConfirmCancel();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    clickConfirmCancel();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 2: submit → 500 → submit again sends 2 requests with the same key', async () => {
    const { writes } = makeHarness([
      { kind: 'http', status: 500, body: { error: { code: 'internalError', message: 'Server error' } } },
      { kind: 'http', status: 200, body: { id: 'bk-1', status: 'CANCELLED', bookingNo: 'B261015-0001' } },
    ]);

    await mountAndOpenCancel();
    fillReason('ลูกค้าแจ้งยกเลิก');

    clickConfirmCancel();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    clickConfirmCancel();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 3: submit → 4xx refusal → submit again sends a new key', async () => {
    const { writes } = makeHarness([
      { kind: 'http', status: 400, body: { error: { code: 'validationFailed', message: 'Invalid reason' } } },
      { kind: 'http', status: 200, body: { id: 'bk-1', status: 'CANCELLED', bookingNo: 'B261015-0001' } },
    ]);

    await mountAndOpenCancel();
    fillReason('ลูกค้าแจ้งยกเลิก');

    clickConfirmCancel();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    clickConfirmCancel();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 4: submit → network error → edit the input → submit sends a new key', async () => {
    const { writes } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: { id: 'bk-1', status: 'CANCELLED', bookingNo: 'B261015-0001' } },
    ]);

    await mountAndOpenCancel();
    fillReason('ลูกค้าแจ้งยกเลิก');

    clickConfirmCancel();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    fillReason('ลูกค้าเปลี่ยนใจ');
    clickConfirmCancel();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).not.toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 5: submit → 2xx → submit the same input again sends a new key', async () => {
    const { writes } = makeHarness([
      { kind: 'http', status: 200, body: { id: 'bk-1', status: 'CANCELLED', bookingNo: 'B261015-0001' } },
      { kind: 'http', status: 200, body: { id: 'bk-1', status: 'CANCELLED', bookingNo: 'B261015-0001' } },
    ]);

    await mountAndOpenCancel();
    fillReason('ลูกค้าแจ้งยกเลิก');

    clickConfirmCancel();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await vi.waitFor(() => {
      expect(host.querySelector('#cancel-reason-input')).toBeNull();
    });

    const cancelBtn = host.querySelector('[data-booking-id="bk-1"] button[data-action="cancel"]') as HTMLButtonElement;
    cancelBtn.click();
    await vi.waitFor(() => {
      expect(host.querySelector('#cancel-reason-input')).not.toBeNull();
    });
    fillReason('ลูกค้าแจ้งยกเลิก');
    clickConfirmCancel();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
