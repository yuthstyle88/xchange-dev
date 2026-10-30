// XC-BOOKING-ESTIMATE FE-3 (#5390): the desk new-booking dialog shows the BOOKING quote's estimate and books with its id.
// Design docs/design/blueprint/marketing-campaigns.md §6.4.3, §6.4.5 row 25, §6.4.6 FE-3, decision E-2 and O-3 (the desk's
// estimate is the BOOKING quote's own answer, indicative, no reservation). Written before the implementation by the FE
// lane controller claude/e92db6. Do not edit, delete or weaken these tests; add your own elsewhere.
//
// [required] by these tests:
// - When the dialog has a slot, a side, a currency and a valid whole quantity, it sends POST /api/quotes with
//   `channel: "BOOKING"`, the form's branch as `branch` and `branchId`, `customerId: null` and one line
//   `{ currency, shopSide, quantity: "<n>", feeThb: null, denominationGroup: null }`, with an Idempotency-Key. It sends
//   again when one of those fields changes. Only the answer to the latest request is kept.
// - The answer's benefits show in the dialog inside `[data-row="booking-estimate"]` through `BenefitLines`, with
//   `basis` from the answer's `benefitBasis` (so a BOOKING quote shows the "ประมาณการ" badge).
// - Submit sends the kept quote's `id` as `quoteId` in POST /api/bookings. With no kept quote, submit asks for one
//   first and books with its id; a refused quote books nothing and shows the refusal.
// - A booking refused with `validationFailed` on field `quoteId` drops the kept quote, so the next submit asks again.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeMe,
  makeMeRole,
  makeQuoteBenefitDto,
  makeQuoteDto,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { withBookingSlots } from './booking-slots.testing';

interface Call {
  path: string;
  key: string | null;
  body: Record<string, unknown>;
}

type Answer = { status: number; body: unknown };

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const STAFF_TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: 'XC-B01' })],
  branch: 'XC-B01',
  branchName: 'XC-B01',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const DESK_BOOKINGS_DATA = makeDeskBookingsData({
  items: [
    makeDeskBookingItemDto({
      id: 'bk-1',
      bookingNo: 'B261015-0001',
      status: 'CONFIRMED',
      slotId: 'slot-1',
      startAt: '2026-10-15T09:00:00Z',
      endAt: '2026-10-15T09:30:00Z',
    }),
  ],
});

function bookingQuote(id: string, o: Parameters<typeof makeQuoteDto>[0] = {}) {
  return makeQuoteDto({
    id,
    channel: 'BOOKING',
    benefitBasis: 'INDICATIVE',
    benefits: [makeQuoteBenefitDto({ benefitClass: 'PRICE', rateBefore: '35.10', rateAfter: '35.20', feeBefore: '0.00', feeAfter: '0.00' })],
    ...o,
  });
}

function makeServer() {
  const quotes: Call[] = [];
  const bookings: Call[] = [];
  const quoteAnswers: Array<(n: number) => Answer | Promise<Answer>> = [];
  let bookingAnswer: (n: number) => Answer = () => ({
    status: 200,
    body: { id: 'bk-new', bookingNo: 'B261015-0002', status: 'CONFIRMED' },
  });

  vi.stubGlobal(
    'fetch',
    withBookingSlots(vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const key = new Headers(init?.headers).get('idempotency-key');
      const body = typeof init?.body === 'string' ? (JSON.parse(init.body) as Record<string, unknown>) : {};

      if (method === 'GET' && u.pathname === '/api/v3/views/me') {
        return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
      }
      if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
        return jsonResponse(200, { version: 1, changed: true, data: DESK_BOOKINGS_DATA });
      }
      if (method === 'POST' && u.pathname === '/api/quotes') {
        quotes.push({ path: u.pathname, key, body });
        const n = quotes.length;
        const pick = quoteAnswers[n - 1] ?? quoteAnswers[quoteAnswers.length - 1];
        const a = pick ? await pick(n) : { status: 200, body: bookingQuote(`q-${n}`) };
        return jsonResponse(a.status, a.body);
      }
      if (method === 'POST' && u.pathname === '/api/bookings') {
        bookings.push({ path: u.pathname, key, body });
        const a = bookingAnswer(bookings.length);
        return jsonResponse(a.status, a.body);
      }
      return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
    }),
  ));

  return {
    quotes,
    bookings,
    /** The n-th quote answer (1-based); the last one given repeats. */
    onQuote(...answers: Array<(n: number) => Answer | Promise<Answer>>) {
      quoteAnswers.push(...answers);
    },
    onBooking(f: (n: number) => Answer) {
      bookingAnswer = f;
    },
  };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await new Promise((r) => setTimeout(r, 0));
}

function setValue(el: HTMLInputElement | HTMLSelectElement, val: string): void {
  el.value = val;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('DeskBookingsScreen new-booking dialog: the BOOKING quote (FE-3, design §6.4.5 row 25)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let server: ReturnType<typeof makeServer>;

  beforeEach(() => {
    setAudience('staff');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    server = makeServer();
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

  const field = <T extends HTMLElement>(name: string) => host.querySelector<T>(`[name="${name}"]`);

  async function openDialog(): Promise<void> {
    dispose = render(() => <DeskBookingsScreen />, host);
    await vi.waitFor(() => expect(host.querySelector('[data-booking-id="bk-1"]')).not.toBeNull());
    host.querySelector<HTMLButtonElement>('button[data-action="new-booking"]')!.click();
    await vi.waitFor(() => expect(field<HTMLInputElement>('quantity')).not.toBeNull());
  }

  /** Fills the contact fields so that submit passes the dialog's own checks. */
  function fillContact(): void {
    setValue(field<HTMLInputElement>('displayName')!, 'Somchai Jaidee');
    setValue(field<HTMLInputElement>('phone')!, '0812345678');
  }

  function submit(): void {
    host.querySelector<HTMLButtonElement>('form button[type="submit"]')!.click();
  }

  it('Q1 a valid quantity sends one BOOKING quote with the form line and an Idempotency-Key', async () => {
    await openDialog();
    setValue(field<HTMLInputElement>('quantity')!, '5');
    await vi.waitFor(() => expect(server.quotes.length).toBeGreaterThan(0));
    await flush();
    const last = server.quotes[server.quotes.length - 1]!;
    expect(last.key, 'an Idempotency-Key on the quote').toBeTruthy();
    expect(last.body.channel).toBe('BOOKING');
    expect(last.body.branchId).toBe('XC-B01');
    expect(last.body.customerId).toBeNull();
    expect(last.body.lines).toEqual([
      {
        currency: field<HTMLSelectElement>('currency')!.value,
        shopSide: field<HTMLSelectElement>('shopSide')!.value,
        quantity: '5',
        feeThb: null,
        denominationGroup: null,
      },
    ]);
  });

  it('Q2 an invalid quantity sends no quote', async () => {
    await openDialog();
    setValue(field<HTMLInputElement>('quantity')!, '0');
    await flush();
    setValue(field<HTMLInputElement>('quantity')!, 'abc');
    await flush();
    expect(server.quotes).toEqual([]);
  });

  it('Q3 the BOOKING answer shows its benefit with the indicative badge inside the dialog', async () => {
    await openDialog();
    setValue(field<HTMLInputElement>('quantity')!, '5');
    await vi.waitFor(() => {
      const box = host.querySelector('[data-row="booking-estimate"]');
      expect(box, 'the estimate block').not.toBeNull();
      expect(box!.querySelector('[data-row="benefit-indicative"]')?.textContent).toBe('ประมาณการ');
      expect(box!.textContent).toContain('35.10 → 35.20');
    });
  });

  it('Q4 submit books with the shown quote id', async () => {
    await openDialog();
    setValue(field<HTMLInputElement>('quantity')!, '5');
    fillContact();
    await vi.waitFor(() => expect(host.querySelector('[data-row="booking-estimate"]')).not.toBeNull());
    const quotesBefore = server.quotes.length;
    submit();
    await vi.waitFor(() => expect(server.bookings.length).toBe(1));
    expect(server.bookings[0]!.body.quoteId).toBe(`q-${quotesBefore}`);
    expect(server.quotes.length, 'no new quote at submit when one is kept').toBe(quotesBefore);
  });

  it('Q5 changing the quantity asks again and books with the newer quote id', async () => {
    await openDialog();
    setValue(field<HTMLInputElement>('quantity')!, '5');
    await vi.waitFor(() => expect(server.quotes.length).toBeGreaterThan(0));
    await flush();
    const first = server.quotes.length;
    setValue(field<HTMLInputElement>('quantity')!, '7');
    await vi.waitFor(() => expect(server.quotes.length).toBeGreaterThan(first));
    await flush();
    expect(server.quotes[server.quotes.length - 1]!.body.lines).toEqual([
      expect.objectContaining({ quantity: '7' }),
    ]);
    fillContact();
    submit();
    await vi.waitFor(() => expect(server.bookings.length).toBe(1));
    expect(server.bookings[0]!.body.quoteId).toBe(`q-${server.quotes.length}`);
  });

  it('Q6 an older answer that arrives last is not kept', async () => {
    let releaseFirst!: () => void;
    server.onQuote(
      () =>
        new Promise<Answer>((resolve) => {
          releaseFirst = () => resolve({ status: 200, body: bookingQuote('q-old') });
        }),
      (n) => ({ status: 200, body: bookingQuote(`q-${n}`) }),
    );
    await openDialog();
    setValue(field<HTMLInputElement>('quantity')!, '5');
    await vi.waitFor(() => expect(server.quotes.length).toBe(1));
    setValue(field<HTMLInputElement>('quantity')!, '7');
    await vi.waitFor(() => expect(server.quotes.length).toBe(2));
    await flush();
    releaseFirst();
    await flush();
    fillContact();
    submit();
    await vi.waitFor(() => expect(server.bookings.length).toBe(1));
    expect(server.bookings[0]!.body.quoteId).toBe('q-2');
  });

  it('Q7 with no kept quote, submit asks for one first and books with its id', async () => {
    server.onQuote(
      () => ({ status: 503, body: { error: { code: 'serviceUnavailable', message: 'down' } } }),
      (n) => ({ status: 200, body: bookingQuote(`q-${n}`) }),
    );
    await openDialog();
    setValue(field<HTMLInputElement>('quantity')!, '5');
    await vi.waitFor(() => expect(server.quotes.length).toBe(1));
    await flush();
    expect(host.querySelector('[data-row="booking-estimate"]'), 'no estimate after a failed quote').toBeNull();
    fillContact();
    submit();
    await vi.waitFor(() => expect(server.bookings.length).toBe(1));
    expect(server.quotes.length).toBe(2);
    expect(server.bookings[0]!.body.quoteId).toBe('q-2');
  });

  it('Q8 a quote refused at submit books nothing and shows the refusal', async () => {
    server.onQuote(() => ({ status: 409, body: { error: { code: 'rateNotPublished', message: 'no published rate' } } }));
    await openDialog();
    setValue(field<HTMLInputElement>('quantity')!, '5');
    fillContact();
    await vi.waitFor(() => expect(server.quotes.length).toBeGreaterThan(0));
    await flush();
    submit();
    await vi.waitFor(() => expect(host.querySelector('[role="alert"]')).not.toBeNull());
    await flush();
    expect(server.bookings).toEqual([]);
  });

  it('Q9 a booking refused on field quoteId drops the kept quote; the next submit asks again', async () => {
    server.onBooking((n) =>
      n === 1
        ? { status: 400, body: { error: { code: 'validationFailed', message: 'bad quote', field: 'quoteId' } } }
        : { status: 200, body: { id: 'bk-new', bookingNo: 'B261015-0002', status: 'CONFIRMED' } },
    );
    await openDialog();
    setValue(field<HTMLInputElement>('quantity')!, '5');
    fillContact();
    await vi.waitFor(() => expect(host.querySelector('[data-row="booking-estimate"]')).not.toBeNull());
    const before = server.quotes.length;
    submit();
    await vi.waitFor(() => expect(server.bookings.length).toBe(1));
    await flush();
    submit();
    await vi.waitFor(() => expect(server.bookings.length).toBe(2));
    expect(server.quotes.length, 'a fresh quote before the second booking').toBe(before + 1);
    expect(server.bookings[1]!.body.quoteId).toBe(`q-${before + 1}`);
  });
});
