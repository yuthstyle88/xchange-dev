// #5229 CR-d: the summary aside shows the quote line's `roundingThb` when it is not 0.00 (design
// docs/design/blueprint/quote-transaction.md §13.8 CR-d; generated `QuoteLineDto.roundingThb`).
// Written before the implementation by the FE lane's test-writer (claude/8be3a451's order). Do not edit, delete or
// weaken these tests; add your own in another file.
//
// [required] by these tests (behaviour table rows R8-R9 of #5229):
// - From step 3, after the fee, a `dt`/`dd` pair marked `data-row="rounding"`: the new desk key `rounding` (TH
//   "ปัดเศษเงินสด", EN "Cash rounding") and `{roundingThb} THB`.
// - Nothing marked `data-row="rounding"` when `roundingThb` is "0.00".
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { makeQuoteDto, makeQuoteLineDto, makeQuotePublicationDto } from '../../shared/testing/fixtures';
import type { DeskStore, StepIndex } from './desk-store';
import { Summary } from './Summary';

function makeQuote(roundingThb: string) {
  return makeQuoteDto({
    id: 'q-1',
    type: 'BUY',
    channel: 'DESK',
    expiresAt: '2026-10-27T12:00:00Z',
    publication: makeQuotePublicationDto({ id: 'pub-1', version: 1 }),
    cashSettlement: true,
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
        roundingThb,
        settleThb: '3499.35',
        publishedRateLineId: 'l-1',
      }),
    ],
  });
}

function stubStore(roundingThb: string, stepAt: StepIndex = 3): DeskStore {
  const [step] = createSignal<StepIndex>(stepAt);
  const [quote] = createSignal<ReturnType<typeof makeQuote> | undefined>(makeQuote(roundingThb));
  const [couponCode, setCouponCode] = createSignal('');
  return {
    api: { view: vi.fn(), command: vi.fn() } as unknown as DeskStore['api'],
    branch: () => 'MAIN',
    step,
    type: () => 'BUY',
    currency: () => 'USD',
    amount: () => '100',
    method: () => 'Cash',
    quote,
    quoting: () => false,
    customerId: () => '',
    transactionState: () => undefined,
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

/** The text of every element marked `data-row="rounding"` (the dt and the dd, or one wrapper around both). */
const roundingText = () =>
  [...host.querySelectorAll('[data-row="rounding"]')].map((el) => el.textContent ?? '').join(' ');

describe('Summary cash rounding', () => {
  it('R8 at step >= 3 with roundingThb "0.15": a [data-row="rounding"] row reading "ปัดเศษเงินสด" and "0.15 THB"', () => {
    mount(stubStore('0.15', 3));
    expect(host.querySelectorAll('[data-row="rounding"]').length, 'a rounding row').toBeGreaterThan(0);
    expect(roundingText()).toContain('ปัดเศษเงินสด');
    expect(roundingText()).toContain('0.15 THB');
  });

  it('R9 with roundingThb "0.00": no rounding row', () => {
    mount(stubStore('0.00', 3));
    expect(host.querySelector('[data-row="rounding"]')).toBeNull();
    expect(host.textContent).not.toContain('ปัดเศษเงินสด');
  });
});
