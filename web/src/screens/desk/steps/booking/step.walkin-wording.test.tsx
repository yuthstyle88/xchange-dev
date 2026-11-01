// FE-T8 #5856 (a): step 2 speaks of a transaction, not a booking, when the sale is a walk-in
// (design docs/design/fe-triage-1101.md §1 row FE-T8, §3 W5). Written before the implementation by claude/01TxaRJG
// (FE sub-controller). Do not edit, delete or weaken these tests; add your own elsewhere.
//
// [required] by these tests:
// - A walk-in sale is one the step serves a walk-in queue ticket for: no entry key (`held-ticket.ts` `hasEntryKey`:
//   `bookingId`, `onlineBookingId` or `kioskRequestId` in `entry().payloadExtras()`) and no `heldBooking()`, or one that
//   already holds `walkInTicket()`. It shows heading "ข้อมูลรายการ" / "Transaction details" and submit
//   "ยืนยันข้อมูล →" / "Confirm details →".
// - A sale started from a booking or kiosk entry, or one holding a called booking (`heldBooking()`), keeps today's v2
//   text: "ข้อมูลการจอง" / "ยืนยันการจอง →".
// - The walk-in step still serves the ticket (`POST /api/queue/tickets/serve`) before step 3.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import { makeQueueTicketDto } from '../../../../shared/testing/fixtures';
import { step } from './step';

const WALKIN_HEADING_TH = 'ข้อมูลรายการ';
const WALKIN_SUBMIT_TH = 'ยืนยันข้อมูล →';
const WALKIN_HEADING_EN = 'Transaction details';
const WALKIN_SUBMIT_EN = 'Confirm details →';
const BOOKING_HEADING_TH = 'ข้อมูลการจอง';
const BOOKING_SUBMIT_TH = 'ยืนยันการจอง →';

type Init = {
  extras?: Record<string, string | boolean>;
  heldBooking?: { bookingId: string; ticketId: string; displayNumber: string };
  walkInTicket?: unknown;
};

function stubStore(init: Init = {}) {
  const [fields] = createSignal({ name: 'JOHN DOE', passportNo: 'AA1', nationality: 'USA', birthDate: '', passportExpiry: '' });
  const [valueDate, setValueDate] = createSignal('2026-11-01');
  const [walkInTicket, setWalkInTicket] = createSignal<unknown>(init.walkInTicket);
  const entry = init.extras ? { kind: 'kiosk', id: 'e-1', locked: true, payloadExtras: () => init.extras! } : undefined;
  return {
    api: {
      command: vi.fn(async () => makeQueueTicketDto({ displayNumber: 'W001', status: 'SERVING' })),
      view: vi.fn(async () => ({ version: 1, changed: true, data: undefined })),
    },
    path: { kind: 'staff' },
    step: () => 2,
    fields,
    staff: () => 'ploy',
    valueDate,
    setValueDate: vi.fn((v: string) => setValueDate(v)),
    booking: () => '',
    setBooking: vi.fn(),
    customerId: () => '',
    walkInTicket,
    setWalkInTicket: vi.fn((t: unknown) => setWalkInTicket(t)),
    heldBooking: () => init.heldBooking,
    setHeldBooking: vi.fn(),
    branch: () => 'MAIN',
    meId: () => 'u-1',
    serveKey: () => 'serve-key-1',
    entry: () => entry,
    go: vi.fn(),
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  setLang('th');
});

function mount(store: ReturnType<typeof stubStore>) {
  dispose = render(() => step.component({ store: store as any }), host);
}

const heading = () => host.querySelector('h2')?.textContent?.trim();
const submit = () => host.querySelector<HTMLButtonElement>('button[type="submit"]');

describe('FE-T8 W5 step 2 wording', () => {
  it('T8-1 a walk-in sale shows "ข้อมูลรายการ" and "ยืนยันข้อมูล →", never the booking words', () => {
    mount(stubStore());
    expect(heading()).toBe(WALKIN_HEADING_TH);
    expect(submit()?.textContent?.trim()).toBe(WALKIN_SUBMIT_TH);
    expect(host.textContent).not.toContain(BOOKING_HEADING_TH);
    expect(host.textContent).not.toContain('ยืนยันการจอง');
  });

  it('T8-2 a walk-in sale in English shows "Transaction details" and "Confirm details →"', async () => {
    mount(stubStore());
    setLang('en');
    await vi.waitFor(() => expect(heading()).toBe(WALKIN_HEADING_EN));
    expect(submit()?.textContent?.trim()).toBe(WALKIN_SUBMIT_EN);
  });

  it('T8-3 a walk-in submit still serves the queue ticket before step 3', async () => {
    const store = stubStore();
    mount(store);
    submit()!.click();
    await vi.waitFor(() => expect(store.go).toHaveBeenCalledWith(3));
    expect(store.api.command).toHaveBeenCalledWith('POST', '/api/queue/tickets/serve', { customerId: null }, 'serve-key-1');
  });

  it('T8-4 a walk-in that already holds its served ticket keeps the walk-in words', () => {
    mount(stubStore({ walkInTicket: makeQueueTicketDto({ displayNumber: 'W001', status: 'SERVING' }) }));
    expect(heading()).toBe(WALKIN_HEADING_TH);
    expect(submit()?.textContent?.trim()).toBe(WALKIN_SUBMIT_TH);
  });

  for (const [name, extras] of [
    ['bookingId', { bookingId: 'b-1' }],
    ['onlineBookingId', { onlineBookingId: 'ob-1' }],
    ['kioskRequestId', { kioskRequestId: 'k-1' }],
  ] as const) {
    it(`T8-5 a sale from an entry with ${name} keeps today's "ข้อมูลการจอง" and "ยืนยันการจอง →"`, () => {
      mount(stubStore({ extras }));
      expect(heading()).toBe(BOOKING_HEADING_TH);
      expect(submit()?.textContent?.trim()).toBe(BOOKING_SUBMIT_TH);
    });
  }

  it("T8-6 a sale holding a called booking keeps today's booking words", () => {
    mount(stubStore({ heldBooking: { bookingId: 'b-1', ticketId: 't-1', displayNumber: 'B001' } }));
    expect(heading()).toBe(BOOKING_HEADING_TH);
    expect(submit()?.textContent?.trim()).toBe(BOOKING_SUBMIT_TH);
  });
});
