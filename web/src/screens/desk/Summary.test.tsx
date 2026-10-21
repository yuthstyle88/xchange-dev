import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { makeQuoteDto, makeQuoteLineDto, makeQuotePublicationDto, type QuoteDto } from '../../shared/testing/fixtures';
import type { DeskStore, StepIndex, TransactionState } from './desk-store';
import { th } from './i18n';
import { Summary } from './Summary';

function makeQuote(): QuoteDto {
  return makeQuoteDto({
    id: 'q-1',
    type: 'BUY',
    channel: 'DESK',
    expiresAt: '2026-10-21T12:00:00Z',
    publication: makeQuotePublicationDto({ id: 'pub-1', version: 1 }),
    lines: [
      makeQuoteLineDto({
        currency: 'USD',
        shopSide: 'BUY',
        quantity: '100',
        unitSize: '1',
        baseRate: '35.12000000',
        finalRate: '35.12000000',
        thbAmount: '3512.00',
        feeThb: '12.50',
        settleThb: '3499.50',
        publishedRateLineId: 'l-1',
      }),
    ],
    benefits: [],
    stale: false,
  });
}

function stubStore(
  init: {
    step?: StepIndex;
    quote?: QuoteDto | undefined;
    quoting?: boolean;
    customerId?: string;
    transactionState?: TransactionState;
  } = {},
): DeskStore {
  const [step] = createSignal<StepIndex>(init.step ?? 0);
  const [quote] = createSignal<QuoteDto | undefined>('quote' in init ? init.quote : makeQuote());
  const [quoting] = createSignal(init.quoting ?? false);
  const [customerId] = createSignal(init.customerId ?? '');
  const [transactionState] = createSignal<TransactionState | undefined>(init.transactionState);
  const [couponCode, setCouponCode] = createSignal('');
  return {
    api: { view: vi.fn(), command: vi.fn() } as unknown as DeskStore['api'],
    branch: () => 'MAIN',
    step,
    type: () => 'BUY',
    currency: () => 'USD',
    amount: () => '100',
    quote,
    quoting,
    customerId,
    transactionState,
    couponCode,
    setCouponCode: vi.fn((c: string) => setCouponCode(c)),
    quoteError: () => undefined,
    entry: () => undefined,
  } as unknown as DeskStore;
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
  vi.unstubAllGlobals();
});

function mount(store: DeskStore) {
  vi.stubGlobal(
    'fetch',
    vi.fn(() => Promise.reject(new Error('Summary must not call the network'))),
  );
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <Summary store={store} />, host);
}

describe('Summary', () => {
  it('row 1: hides fee and net at step 2 and shows feeThb and settleThb as sent from step 3', () => {
    mount(stubStore({ step: 2 }));
    expect(host.textContent).toContain(th.bookedRate);
    expect(host.textContent).not.toContain(th.fee);
    expect(host.textContent).not.toContain('12.50');
    expect(host.textContent).not.toContain('3499.50 THB');

    dispose?.();
    host.remove();

    mount(stubStore({ step: 3 }));
    expect(host.textContent).toContain(th.fee);
    expect(host.textContent).toContain('12.50');
    // The direction label (shop pays / shop receives) is undecided (#4105), so only the net line is checked.
    expect(host.textContent).toContain(' สุทธิ');
    expect(host.textContent).toContain('3499.50 THB');
  });

  it('row 2: shows no booked rate when there is no quote line', () => {
    mount(stubStore({ quote: undefined }));
    expect(host.textContent).not.toContain(th.bookedRate);
  });

  it('row 3: renders each of the 5 transaction states with its own label', () => {
    const cases: Array<[TransactionState, string]> = [
      ['DRAFT', th.stateDraft],
      ['PENDING_APPROVAL', th.statePending],
      ['CONFIRMED', th.stateConfirmed],
      ['COMPLETED', th.stateCompleted],
      ['CANCELLED', th.stateCancelled],
    ];
    for (const [state, label] of cases) {
      mount(stubStore({ transactionState: state }));
      expect(host.textContent, state).toContain(label);
      for (const [otherState, otherLabel] of cases) {
        if (otherState !== state) {
          expect(host.textContent, `${state} vs ${otherState}`).not.toContain(otherLabel);
        }
      }
      dispose?.();
      host.remove();
    }
  });

  it('row 4: shows recalculating text when quoting', () => {
    mount(stubStore({ quoting: false }));
    expect(host.textContent).not.toContain(th.recalculating);
    dispose?.();
    host.remove();

    mount(stubStore({ quoting: true }));
    expect(host.textContent).toContain(th.recalculating);
  });

  it('row 5: renders no customer row when customerId is empty', () => {
    mount(stubStore({ customerId: '' }));
    expect(host.textContent).not.toContain(th.customer);
    const dts = [...host.querySelectorAll('dt')].map((el) => el.textContent);
    expect(dts).not.toContain(th.customer);
  });
});
