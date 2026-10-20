// C03 up-front tests (feat/fe-customer skeleton): BookingExtras, a booking card's confirm button, QR re-issue
// and cancel deadline (design customer-app.md §4.5; bookingConfirm / bookingQrReissue). The implementer makes
// these pass and must not edit them.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import { makeMyBookingItemDto, type MyBookingItemDto } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { BookingExtras } from './BookingExtras';

type Reply = { status: number; body: unknown } | 'network';

interface Write {
  path: string;
  key: string | null;
  body: Record<string, unknown>;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function booking(over: Partial<MyBookingItemDto> = {}) {
  return makeMyBookingItemDto({
    id: 'bk-1',
    bookingNo: 'B261015-0001',
    status: 'CONFIRMED',
    branchId: 'bkk-1',
    slotId: 'slot-1',
    startAt: '2099-10-15T10:00:00Z',
    endAt: '2099-10-15T10:30:00Z',
    qrToken: 'QR-OLD',
    quoteSummary: '',
    cancelCutoffAt: '2099-10-15T09:00:00Z',
    ticketNumber: null,
    ticketsAhead: null,
    estimateSeconds: null,
    estimateLowSeconds: null,
    estimateHighSeconds: null,
    estimateAsOf: null,
    readiness: 'unknown',
    ...over,
  });
}

function harness(replies: { confirm?: Reply[]; qr?: Reply[] } = {}) {
  const writes: Write[] = [];
  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const u = new URL(input, 'http://localhost');
    if ((init?.method ?? 'GET').toUpperCase() !== 'POST') return json(404, {});
    const body = JSON.parse(typeof init?.body === 'string' ? init.body : '{}');
    writes.push({ path: u.pathname, key: new Headers(init?.headers).get('idempotency-key'), body });
    let r: Reply | undefined;
    if (u.pathname === '/api/bookings/bk-1/confirm') {
      r = replies.confirm?.shift() ?? { status: 200, body: { id: 'bk-1', status: 'CONFIRMED', bookingNo: 'B261015-0001' } };
    } else if (u.pathname === '/api/bookings/bk-1/qr') {
      r = replies.qr?.shift() ?? { status: 200, body: { id: 'bk-1', qrToken: 'QR-NEW', qrVersion: 2 } };
    } else {
      return json(404, { error: { code: 'notFound', message: 'Not found' } });
    }
    if (r === 'network') throw new TypeError('Failed to fetch');
    return json(r.status, r.body);
  });
  return { api: new ApiClient(fetchFn), writes };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mount(b: MyBookingItemDto, api: ApiClient, onChange = vi.fn()) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <BookingExtras booking={b} api={api} onChange={onChange} />, host);
  return onChange;
}

const confirmBtn = () => host.querySelector('button[data-action="confirm"]') as HTMLButtonElement | null;
const reissueBtn = () => host.querySelector('button[data-action="reissue-qr"]') as HTMLButtonElement | null;
const deadline = () => host.querySelector('[data-testid="cancel-deadline"]');
const errorBox = () => host.querySelector('[data-testid="extras-error"]');

beforeEach(() => setLang('en'));
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('BookingExtras (C03)', () => {
  it('a PENDING_CUSTOMER_CONFIRMATION booking has a confirm button; a CONFIRMED one does not', () => {
    mount(booking({ status: 'PENDING_CUSTOMER_CONFIRMATION' }), harness().api);
    expect(confirmBtn()).not.toBeNull();
    dispose?.();
    host.remove();
    mount(booking({ status: 'CONFIRMED' }), harness().api);
    expect(confirmBtn()).toBeNull();
  });

  it('confirm posts {id, branch} to /api/bookings/{id}/confirm with an Idempotency-Key and hands back the new status', async () => {
    const h = harness();
    const onChange = mount(booking({ status: 'PENDING_CUSTOMER_CONFIRMATION' }), h.api);
    confirmBtn()!.click();
    await flush();
    expect(h.writes).toEqual([{ path: '/api/bookings/bk-1/confirm', key: expect.any(String), body: { id: 'bk-1', branch: 'bkk-1' } }]);
    expect(h.writes[0].key).toBeTruthy();
    expect(onChange).toHaveBeenCalledWith({ status: 'CONFIRMED' });
  });

  it('a confirm that fails on the network shows the error, changes nothing, and the retry reuses the same key', async () => {
    const h = harness({ confirm: ['network'] });
    const onChange = mount(booking({ status: 'PENDING_CUSTOMER_CONFIRMATION' }), h.api);
    confirmBtn()!.click();
    await flush();
    expect(errorBox()?.textContent?.trim()).toBeTruthy();
    expect(onChange).not.toHaveBeenCalled();
    confirmBtn()!.click();
    await flush();
    expect(h.writes).toHaveLength(2);
    expect(h.writes[1].key).toBe(h.writes[0].key);
    expect(onChange).toHaveBeenCalledWith({ status: 'CONFIRMED' });
  });

  it.each(['PENDING_CUSTOMER_CONFIRMATION', 'CONFIRMED', 'CHECKED_IN'])('a %s booking offers a QR re-issue', (status) => {
    mount(booking({ status }), harness().api);
    expect(reissueBtn()).not.toBeNull();
  });

  it.each(['COMPLETED', 'CANCELLED', 'EXPIRED', 'NO_SHOW'])('a %s booking offers no QR re-issue and no confirm', (status) => {
    mount(booking({ status }), harness().api);
    expect(reissueBtn()).toBeNull();
    expect(confirmBtn()).toBeNull();
  });

  it('QR re-issue posts {id, branch} to /api/bookings/{id}/qr and hands back the new token', async () => {
    const h = harness();
    const onChange = mount(booking(), h.api);
    reissueBtn()!.click();
    await flush();
    expect(h.writes).toEqual([{ path: '/api/bookings/bk-1/qr', key: expect.any(String), body: { id: 'bk-1', branch: 'bkk-1' } }]);
    expect(onChange).toHaveBeenCalledWith({ qrToken: 'QR-NEW' });
  });

  it('a refused QR re-issue shows the worded error and changes nothing', async () => {
    const h = harness({ qr: [{ status: 409, body: { error: { code: 'invalidState', message: 'no' } } }] });
    const onChange = mount(booking(), h.api);
    reissueBtn()!.click();
    await flush();
    expect(errorBox()?.textContent?.trim()).toBeTruthy();
    expect(onChange).not.toHaveBeenCalled();
  });

  it('shows the cancel deadline while it is in the future and the booking is open', () => {
    mount(booking(), harness().api);
    expect(deadline()?.textContent?.trim()).toBeTruthy();
    expect(deadline()?.textContent).toContain('2099');
  });

  it('shows no cancel deadline when there is none, when it has passed, or when the booking is finished', () => {
    for (const b of [
      booking({ cancelCutoffAt: null }),
      booking({ cancelCutoffAt: '2000-01-01T00:00:00Z' }),
      booking({ status: 'CANCELLED' }),
    ]) {
      mount(b, harness().api);
      expect(deadline()).toBeNull();
      dispose?.();
      host.remove();
    }
  });
});
