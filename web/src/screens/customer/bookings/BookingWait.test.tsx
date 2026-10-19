// C03 behavior tests: BookingWait (#3451 §8 Behaviour table).
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { BookingWait, type BookingWaitProps } from './BookingWait';

function booking(over: Partial<BookingWaitProps['booking']> = {}): BookingWaitProps['booking'] {
  return {
    id: 'bk-1',
    bookingNo: 'B261015-0001',
    status: 'CONFIRMED',
    branchId: 'bkk-1',
    slotId: 'slot-1',
    startAt: '2099-10-15T10:00:00Z',
    endAt: '2099-10-15T10:30:00Z',
    qrToken: 'QR-TOKEN',
    quoteSummary: '',
    cancelCutoffAt: null,
    ticketNumber: 'B003',
    readiness: 'unknown',
    ...over,
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mount(b: BookingWaitProps['booking']) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <BookingWait booking={b} />, host);
}

const waitAhead = () => host.querySelector('[data-testid="booking-wait-ahead"]');
const waitEstimate = () => host.querySelector('[data-testid="booking-wait-estimate"]');

beforeEach(() => setLang('en'));
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('BookingWait (C03 #3451)', () => {
  it('ticketsAhead: 2, low 420, high 780: booking-wait-ahead contains 2; booking-wait-estimate reads 7 … 13', () => {
    mount(booking({ ticketsAhead: 2, estimateLowSeconds: 420, estimateHighSeconds: 780 }));
    expect(waitAhead()).not.toBeNull();
    expect(waitAhead()!.textContent).toContain('2');
    expect(waitEstimate()).not.toBeNull();
    const estText = waitEstimate()?.textContent ?? '';
    expect(estText).toContain('7');
    expect(estText).toContain('13');
  });

  it('ticketsAhead: 1, estimates null (no active counter): booking-wait-ahead contains 1; no estimate', () => {
    mount(booking({ ticketsAhead: 1, estimateLowSeconds: null, estimateHighSeconds: null }));
    expect(waitAhead()).not.toBeNull();
    expect(waitAhead()!.textContent).toContain('1');
    expect(waitEstimate()).toBeNull();
  });

  it('ticketsAhead: 0, estimates null (ticket CALLED): nothing', () => {
    mount(booking({ ticketsAhead: 0, estimateLowSeconds: null, estimateHighSeconds: null }));
    expect(waitAhead()).toBeNull();
    expect(waitEstimate()).toBeNull();
    expect(host.textContent?.trim()).toBe('');
  });

  it('ticketsAhead: 0, low 0, high 0 (only walk-ins waiting): nothing', () => {
    mount(booking({ ticketsAhead: 0, estimateLowSeconds: 0, estimateHighSeconds: 0 }));
    expect(waitAhead()).toBeNull();
    expect(waitEstimate()).toBeNull();
    expect(host.textContent?.trim()).toBe('');
  });

  it('ticketsAhead: null or absent (no ticket; today back end): nothing', () => {
    mount(booking({ ticketsAhead: null }));
    expect(waitAhead()).toBeNull();
    expect(waitEstimate()).toBeNull();
    expect(host.textContent?.trim()).toBe('');

    dispose?.();
    host.remove();

    mount(booking({}));
    expect(waitAhead()).toBeNull();
    expect(waitEstimate()).toBeNull();
    expect(host.textContent?.trim()).toBe('');
  });
});
