// C1 FE up-front tests (docs/design/walkin-queue.md §12.3 C1 and its behaviour table, rows F1-F7; the owner's ruling C1
// relayed on #564, issuecomment-6543033385). The customer step (index 1) runs before the booking step (index 2), so it
// reads the teller's held ticket itself and sends `bookingId` with `reserveCustomer`.
// Written before the implementation by the ticket author claude/01TxaRJG. Do not edit, delete or weaken; add tests in a
// file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { OWNER_DESK } from '../../desk-path';
import { createDeskStore, type DeskStore } from '../../desk-store';
import { step as customerStep } from './step';

const ME = () => ({
  id: 'u-1',
  userId: 'u-1',
  username: 'ploy',
  name: 'Ploy',
  roles: [],
  permissions: [],
  branch: 'MAIN',
  branchId: 'MAIN',
  branchName: 'XC-B',
  branches: [],
});
const NO_ME = () => undefined;

const NOT_THIS_CUSTOMER = 'พาสปอร์ตนี้ไม่ใช่ของลูกค้าที่จองคิวนี้ กรุณาตรวจพาสปอร์ตอีกครั้ง';
const JPEG = new File([new Uint8Array([0xff, 0xd8, 0xff, 1])], 'passport.jpg', { type: 'image/jpeg' });

type Call = { method: string; path: string; body: any };
type Answer = { status: number; body?: unknown };

interface CalledRow {
  id: string;
  displayNumber: string;
  serviceType: 'WALK_IN' | 'BOOKING';
  status: 'CALLED' | 'SERVING';
  tellerId: string | null;
}

interface BookingItem {
  id: string;
  bookingNo: string;
  status: string;
  ticketNumber: string | null;
}

function stubServer(opts: {
  called?: CalledRow[];
  boardStatus?: number;
  bookings?: BookingItem[];
  reserve?: (payload: any) => Answer;
}) {
  const calls: Call[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0]!;
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    calls.push({ method, path, body });
    let a: Answer;
    if (method === 'GET' && path === '/api/v3/views/queue_board') {
      const status = opts.boardStatus ?? 200;
      a =
        status === 200
          ? {
              status,
              body: {
                version: 1,
                changed: true,
                data: {
                  waiting: [],
                  called: (opts.called ?? []).map((c) => ({ ...c, tellerName: null, calledAt: null })),
                  myCounter: { status: 'ACTIVE' },
                },
              },
            }
          : { status, body: { error: { code: 'serverError', message: 'serverError' } } };
    } else if (method === 'GET' && path === '/api/v3/views/desk_bookings') {
      a = {
        status: 200,
        body: {
          version: 1,
          changed: true,
          data: {
            items: (opts.bookings ?? []).map((b) => ({
              ...b,
              customerDisplayName: null,
              phoneLast4: null,
              readiness: 'READY',
              slotId: 'slot-1',
              startAt: '2026-10-31T09:00:00Z',
              endAt: '2026-10-31T09:30:00Z',
            })),
          },
        },
      };
    } else if (method === 'POST' && path === '/api/rpc' && body?.action === 'reserveCustomer') {
      a = opts.reserve ? opts.reserve(body.payload) : { status: 200, body: { customerId: 'CU-APP-1', existing: true } };
    } else {
      a = { status: 404, body: { error: { code: 'notFound', message: 'not found' } } };
    }
    return new Response(JSON.stringify(a.body ?? {}), {
      status: a.status,
      headers: { 'content-type': 'application/json' },
    });
  });
  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    reserves: () => calls.filter((c) => c.method === 'POST' && c.body?.action === 'reserveCustomer'),
    boardReads: () => calls.filter((c) => c.path === '/api/v3/views/queue_board'),
  };
}

const HELD_BOOKING: CalledRow = { id: 't-9', displayNumber: 'B003', serviceType: 'BOOKING', status: 'SERVING', tellerId: 'u-1' };
const BK9: BookingItem = { id: 'bk-9', bookingNo: 'B261031-0003', status: 'IN_SERVICE', ticketNumber: 'B003' };

let disposers: (() => void)[] = [];
let host: HTMLDivElement;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  vi.stubGlobal('confirm', vi.fn(() => true));
});

afterEach(() => {
  for (const d of disposers) d();
  disposers = [];
  host.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  setLang('th');
});

function atCustomerStep(s: ReturnType<typeof stubServer>, me: () => unknown = ME): DeskStore {
  let store!: DeskStore;
  createRoot((dispose) => {
    disposers.push(dispose);
    store = createDeskStore({ api: s.api, path: OWNER_DESK, me: me as never });
  });
  store.go(1);
  disposers.push(render(() => customerStep.component({ store }), host));
  return store;
}

function fillPassport(store: DeskStore): void {
  store.setPassport(JPEG);
  store.setField('name', 'JOHN DOE', 'user');
  store.setField('passportNo', 'P2000001', 'user');
  store.setField('nationality', 'DEU', 'user');
}

const submit = () => host.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();

describe('C1: the customer step sends the held booking with reserveCustomer', () => {
  it('F1: a held BOOKING ticket with its booking sends bookingId and keeps heldBooking', async () => {
    const s = stubServer({ called: [HELD_BOOKING], bookings: [BK9] });
    const store = atCustomerStep(s);
    fillPassport(store);
    submit();
    await vi.waitFor(() => expect(store.step()).toBe(2));
    expect(s.reserves()).toHaveLength(1);
    expect(s.reserves()[0]!.body.payload).toMatchObject({ passportNo: 'P2000001', bookingId: 'bk-9' });
    expect(store.heldBooking()).toEqual({ bookingId: 'bk-9', ticketId: 't-9', displayNumber: 'B003' });
    expect(store.customerId()).toBe('CU-APP-1');
  });

  it('F2: a heldBooking already in the store is sent without reading the board again', async () => {
    const s = stubServer({});
    const store = atCustomerStep(s);
    store.setHeldBooking({ bookingId: 'bk-7', ticketId: 't-7', displayNumber: 'B001' });
    fillPassport(store);
    submit();
    await vi.waitFor(() => expect(store.step()).toBe(2));
    expect(s.boardReads()).toHaveLength(0);
    expect(s.reserves()[0]!.body.payload.bookingId).toBe('bk-7');
  });

  it.each([
    ['a held walk-in ticket', { called: [{ ...HELD_BOOKING, serviceType: 'WALK_IN' as const, displayNumber: 'W004' }] }],
    ['no held ticket', { called: [] }],
    ['a BOOKING ticket held by another teller', { called: [{ ...HELD_BOOKING, tellerId: 'u-2' }], bookings: [BK9] }],
    ['a held BOOKING ticket with no matching booking', { called: [HELD_BOOKING], bookings: [] }],
    ['a board that cannot be read', { boardStatus: 500 }],
  ])('F3: %s sends today\'s payload, with no bookingId key', async (_what, opts) => {
    const s = stubServer(opts);
    const store = atCustomerStep(s);
    fillPassport(store);
    submit();
    await vi.waitFor(() => expect(store.step()).toBe(2));
    expect(s.reserves()).toHaveLength(1);
    expect(Object.keys(s.reserves()[0]!.body.payload).sort()).toEqual(
      ['birthDate', 'mode', 'name', 'nationality', 'passportExpiry', 'passportNo'],
    );
    expect(store.heldBooking()).toBeUndefined();
  });

  it('F4: a sale with an entry key reads no held ticket and sends no bookingId', async () => {
    const s = stubServer({ called: [HELD_BOOKING], bookings: [BK9] });
    const store = atCustomerStep(s);
    store.setEntry({
      kind: 'kiosk',
      payloadExtras: () => ({ kioskRequestId: 'kr-1' }),
    } as never);
    fillPassport(store);
    submit();
    await vi.waitFor(() => expect(store.step()).toBe(2));
    expect(s.boardReads()).toHaveLength(0);
    expect(s.reserves()[0]!.body.payload).not.toHaveProperty('bookingId');
  });

  it('F5: the skip (WALK-IN) never reads the held ticket and never sends bookingId', async () => {
    const s = stubServer({ called: [HELD_BOOKING], bookings: [BK9] });
    atCustomerStep(s);
    const skip = [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) =>
      b.textContent?.includes('ข้ามข้อมูลลูกค้า'),
    )!;
    skip.click();
    await vi.waitFor(() => expect(s.reserves()).toHaveLength(1));
    expect(s.reserves()[0]!.body.payload).toMatchObject({ passportNo: 'WALKIN' });
    expect(s.reserves()[0]!.body.payload).not.toHaveProperty('bookingId');
    expect(s.boardReads()).toHaveLength(0);
  });

  it('F6: bookingNotThisCustomer is shown on the passport field and the step stays', async () => {
    const s = stubServer({
      called: [HELD_BOOKING],
      bookings: [BK9],
      reserve: () => ({ status: 400, body: { error: NOT_THIS_CUSTOMER, field: 'passportNo' } }),
    });
    const store = atCustomerStep(s);
    fillPassport(store);
    submit();
    await vi.waitFor(() => expect(host.textContent).toContain(NOT_THIS_CUSTOMER));
    expect(store.step()).toBe(1);
    expect(store.customerId()).toBe('');
  });

  it('F7: with no signed-in teller id the board is not read', async () => {
    const s = stubServer({ called: [HELD_BOOKING], bookings: [BK9] });
    const store = atCustomerStep(s, NO_ME);
    fillPassport(store);
    submit();
    await vi.waitFor(() => expect(store.step()).toBe(2));
    expect(s.boardReads()).toHaveLength(0);
    expect(s.reserves()[0]!.body.payload).not.toHaveProperty('bookingId');
  });
});
