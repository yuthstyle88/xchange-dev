import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { makeQuoteDto, makeQuoteLineDto, makeQuotePublicationDto } from '../../shared/testing/fixtures';
import type { DeskStore, StepIndex, TransactionState } from './desk-store';
import { en, th } from './i18n';
import { Summary } from './Summary';

function makeQuote(type: 'BUY' | 'SELL' = 'BUY') {
  return makeQuoteDto({
    id: 'q-1',
    type,
    channel: 'DESK',
    expiresAt: '2026-10-21T12:00:00Z',
    publication: makeQuotePublicationDto({ id: 'pub-1', version: 1 }),
    lines: [
      makeQuoteLineDto({
        currency: 'USD',
        shopSide: type,
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
    type?: 'BUY' | 'SELL';
    currency?: string;
    amount?: string;
    step?: StepIndex;
    quote?: ReturnType<typeof makeQuote> | undefined;
    quoting?: boolean;
    customerId?: string;
    transactionState?: TransactionState;
  } = {},
): DeskStore {
  const [step] = createSignal<StepIndex>(init.step ?? 0);
  const type = init.type ?? 'BUY';
  const [quote] = createSignal<ReturnType<typeof makeQuote> | undefined>('quote' in init ? init.quote : makeQuote(type));
  const [quoting] = createSignal(init.quoting ?? false);
  const [customerId] = createSignal(init.customerId ?? '');
  const [transactionState] = createSignal<TransactionState | undefined>(init.transactionState);
  const [couponCode, setCouponCode] = createSignal('');
  return {
    api: { view: vi.fn(), command: vi.fn() } as unknown as DeskStore['api'],
    branch: () => 'MAIN',
    step,
    type: () => type,
    currency: () => init.currency ?? 'USD',
    amount: () => init.amount ?? '100',
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

  it('ticket #4127: SELL USD 100 first row names USD without THB, net row carries direction', () => {
    mount(stubStore({ type: 'SELL', currency: 'USD', amount: '100', step: 3 }));
    const dts = [...host.querySelectorAll('dt')].map((el) => el.textContent);
    const firstDt = dts[0] ?? '';
    // SELL: Customer buys USD / ลูกค้าซื้อ USD
    expect(firstDt).toContain('USD');
    expect(firstDt).toContain('ลูกค้าซื้อ USD');
    expect(firstDt).not.toContain('THB');

    // Net row's label carries the direction (shopReceives: ร้านรับเงิน THB จากลูกค้า)
    const netDt = dts.find((t) => t?.includes('สุทธิ'));
    expect(netDt).toBeDefined();
    expect(netDt).toContain(th.shopReceives);
  });

  it('ticket #4127: BUY USD 100 first row names USD without THB, net row carries direction', () => {
    mount(stubStore({ type: 'BUY', currency: 'USD', amount: '100', step: 3 }));
    const dts = [...host.querySelectorAll('dt')].map((el) => el.textContent);
    const firstDt = dts[0] ?? '';
    // BUY: Customer sells USD / ลูกค้าขาย USD
    expect(firstDt).toContain('USD');
    expect(firstDt).toContain('ลูกค้าขาย USD');
    expect(firstDt).not.toContain('THB');

    // Net row's label carries the direction (shopPays: ร้านจ่ายเงิน THB ให้ลูกค้า)
    const netDt = dts.find((t) => t?.includes('สุทธิ'));
    expect(netDt).toBeDefined();
    expect(netDt).toContain(th.shopPays);
  });

  it('ticket #4127 en: SELL and BUY in english', () => {
    setLang('en');
    mount(stubStore({ type: 'SELL', currency: 'EUR', amount: '200', step: 3 }));
    const dtsSell = [...host.querySelectorAll('dt')].map((el) => el.textContent);
    expect(dtsSell[0]).toBe('Customer buys EUR');
    expect(dtsSell[0]).not.toContain('THB');
    const netDtSell = dtsSell.find((t) => t?.includes('net'));
    expect(netDtSell).toContain(en.shopReceives);

    dispose?.();
    host.remove();

    mount(stubStore({ type: 'BUY', currency: 'EUR', amount: '200', step: 3 }));
    const dtsBuy = [...host.querySelectorAll('dt')].map((el) => el.textContent);
    expect(dtsBuy[0]).toBe('Customer sells EUR');
    expect(dtsBuy[0]).not.toContain('THB');
    const netDtBuy = dtsBuy.find((t) => t?.includes('net'));
    expect(netDtBuy).toContain(en.shopPays);
  });
});
