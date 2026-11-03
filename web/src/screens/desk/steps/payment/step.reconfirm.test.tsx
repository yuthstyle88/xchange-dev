// CR-f row CR19: the payment step shows what changed when the save stopped for a new confirm (design
// docs/design/blueprint/quote-transaction.md §13.9.2, §13.9.3). Written before the implementation by the ticket author
// (claude/b091f5e0, on the xchange lane controller claude/608dff's order). Do not edit, delete or weaken these tests;
// add your own in another file.
//
// [required] by these tests:
// - While `store.outcome()` is `'reconfirm'`, the step shows a `role="alert"` notice naming `reconfirm().fromNet` and
//   `reconfirm().toNet`, and both rates when they differ. The confirm box is un-ticked and its label names the new net.
// - With any other outcome there is no such notice.
// The store is a stub with the desk's names, so this file does not depend on `desk-store.ts`. The notice's exact
// wording is [proposed] in the design and goes to the owner with the ticket, so only the figures are pinned here.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import {
  makeDeskPaymentMethod,
  makeQuoteDto,
  makeQuoteLineDto,
  makeQuotePublicationDto,
  type QuoteDto,
} from '../../../../shared/testing/fixtures';
import { step } from './step';

type Reconfirm = { fromNet: string; toNet: string; fromRate: string; toRate: string };

function quote(settleThb: string, finalRate: string, roundingThb: string) {
  return makeQuoteDto({
    id: 'q-2',
    type: 'BUY',
    channel: 'DESK',
    cashSettlement: true,
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    publication: makeQuotePublicationDto({ id: 'p', version: 1 }),
    lines: [
      makeQuoteLineDto({
        currency: 'USD',
        shopSide: 'BUY',
        quantity: '20',
        unitSize: '1',
        baseRate: finalRate,
        finalRate,
        thbAmount: '671.45',
        feeThb: '0.00',
        roundingThb,
        settleThb,
        publishedRateLineId: 'l',
      }),
    ],
    benefits: [],
    stale: false,
  });
}

function stubStore(outcome: string, reconfirm: Reconfirm | undefined, q: QuoteDto) {
  const [config] = createSignal<unknown>({
    shiftOpen: true,
    paymentMethods: [makeDeskPaymentMethod({ id: 'Cash', enabled: true, buy: true, sell: true, account: 'Cash drawer' })],
  });
  const [paymentChecked, setPaymentChecked] = createSignal(false);
  const sig = <T,>(v: T) => createSignal(v)[0];
  return {
    path: { kind: 'owner' },
    step: sig(3),
    config,
    configError: () => undefined,
    type: sig('BUY'),
    quote: sig<QuoteDto | undefined>(q),
    quoteError: () => undefined,
    quoting: () => false,
    passport: () => new File([new Uint8Array([1])], 'p.jpg', { type: 'image/jpeg' }),
    method: sig('Cash'),
    ref: sig(''),
    account: sig('Cash drawer'),
    fee: sig(''),
    bankFee: sig(''),
    notes: sig(''),
    slip: sig<File | undefined>(undefined),
    paymentChecked,
    countConfirmed: sig(false),
    entry: () => undefined,
    outcome: () => outcome,
    reconfirm: () => reconfirm,
    saveError: () => undefined,
    setMethod: vi.fn(),
    setRef: vi.fn(),
    setAccount: vi.fn(),
    setFee: vi.fn(),
    setBankFee: vi.fn(),
    setNotes: vi.fn(),
    setSlip: vi.fn(),
    setPaymentChecked: vi.fn((v: boolean) => setPaymentChecked(v)),
    setCountConfirmed: vi.fn(),
    submit: vi.fn(async () => 'saved'),
    backToEdit: vi.fn(),
    go: vi.fn(),
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('step 3 must not call the network itself'))));
});
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

function mount(store: ReturnType<typeof stubStore>): void {
  dispose = render(() => step.component({ store: store as never }) as JSX.Element, host);
}

const alerts = () => [...host.querySelectorAll<HTMLElement>('[role="alert"]')].map((e) => e.textContent ?? '');
const boxLabel = () => host.querySelector('input[name="paymentChecked"]')?.closest('label')?.textContent ?? '';

describe('CR19 the payment step after a save stopped for a new confirm', () => {
  for (const lang of ['th', 'en'] as const) {
    it(`[${lang}] the net changed, the rate did not: an alert names both nets; the box is un-ticked and names the new net`, () => {
      setLang(lang);
      const store = stubStore(
        'reconfirm',
        { fromNet: '671.45', toNet: '671.50', fromRate: '33.57260000', toRate: '33.57260000' },
        quote('671.50', '33.57260000', '0.05'),
      );
      mount(store);

      const notice = alerts().find((a) => a.includes('671.45') && a.includes('671.50'));
      expect(notice, `an alert names 671.45 and 671.50; alerts: ${JSON.stringify(alerts())}`).toBeDefined();
      expect(host.querySelector<HTMLInputElement>('input[name="paymentChecked"]')?.checked).toBe(false);
      expect(boxLabel(), 'the box label names the new net').toContain('671.50');
      expect(boxLabel()).not.toContain('671.45');
    });

    it(`[${lang}] the rate changed too: the alert names both rates`, () => {
      setLang(lang);
      const store = stubStore(
        'reconfirm',
        { fromNet: '671.45', toNet: '672.00', fromRate: '33.57260000', toRate: '33.60000000' },
        quote('672.00', '33.60000000', '0.00'),
      );
      mount(store);

      const notice = alerts().find((a) => a.includes('671.45') && a.includes('672.00'));
      expect(notice, `an alert names both nets; alerts: ${JSON.stringify(alerts())}`).toBeDefined();
      expect(notice).toContain('33.57260000');
      expect(notice).toContain('33.60000000');
    });
  }

  it('no notice when the outcome is not reconfirm', () => {
    const store = stubStore('idle', undefined, quote('671.50', '33.57260000', '0.05'));
    mount(store);
    expect(alerts().some((a) => a.includes('671.45'))).toBe(false);
  });
});
