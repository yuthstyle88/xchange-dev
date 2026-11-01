// FE-T8 #5856 (b): the payment step folds the floating queue card while it is on screen and gives it back when it
// leaves (design docs/design/fe-triage-1101.md §1 row FE-T8, §3 W6), with the same tool as the staff count form
// (`shared/ui/queue-card-yield.ts` `yieldQueueCard()`, `staff/shift/CountForm.tsx:42-45`, tested by
// `staff/shift/ShiftCard.yield.test.tsx`). Written before the implementation by claude/01TxaRJG (FE sub-controller).
// Do not edit, delete or weaken these tests; add your own elsewhere.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import {
  makeDeskPaymentMethod,
  makeQuoteDto,
  makeQuoteLineDto,
  makeQuotePublicationDto,
} from '../../../../shared/testing/fixtures';
import { queueCardYielding } from '../../../../shared/ui/queue-card-yield';
import { step } from './step';

function stubStore() {
  const [method, setMethod] = createSignal('Cash');
  const [ref, setRef] = createSignal('');
  const [account, setAccount] = createSignal('');
  const [fee, setFee] = createSignal('');
  const [bankFee, setBankFee] = createSignal('');
  const [notes, setNotes] = createSignal('');
  const [slip, setSlip] = createSignal<File | undefined>();
  const [paymentChecked, setPaymentChecked] = createSignal(false);
  const [countConfirmed, setCountConfirmed] = createSignal(false);
  const quote = makeQuoteDto({
    id: 'q-1',
    type: 'BUY',
    channel: 'DESK',
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    publication: makeQuotePublicationDto({ id: 'p', version: 1 }),
    lines: [
      makeQuoteLineDto({
        currency: 'USD',
        shopSide: 'BUY',
        quantity: '100',
        unitSize: '1',
        baseRate: '35.12000000',
        finalRate: '35.12000000',
        thbAmount: '3512.00',
        feeThb: '0.00',
        settleThb: '3512.00',
        publishedRateLineId: 'l',
      }),
    ],
    benefits: [],
    stale: false,
  });
  return {
    path: { kind: 'staff' },
    step: () => 3,
    config: () => ({
      shiftOpen: true,
      paymentMethods: [makeDeskPaymentMethod({ id: 'Cash', enabled: true, buy: true, sell: true, account: 'Cash drawer', instruction: '' })],
    }),
    configError: () => undefined,
    type: () => 'BUY',
    quote: () => quote,
    quoting: () => false,
    passport: () => undefined,
    method,
    ref,
    account,
    fee,
    bankFee,
    notes,
    slip,
    paymentChecked,
    countConfirmed,
    entry: () => undefined,
    outcome: () => 'idle',
    saveError: () => undefined,
    setMethod: vi.fn((v: string) => setMethod(v)),
    setRef: vi.fn((v: string) => setRef(v)),
    setAccount: vi.fn((v: string) => setAccount(v)),
    setFee: vi.fn((v: string) => setFee(v)),
    setBankFee: vi.fn((v: string) => setBankFee(v)),
    setNotes: vi.fn((v: string) => setNotes(v)),
    setSlip: vi.fn((v: File | undefined) => setSlip(v)),
    setPaymentChecked: vi.fn((v: boolean) => setPaymentChecked(v)),
    setCountConfirmed: vi.fn((v: boolean) => setCountConfirmed(v)),
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

function mount() {
  dispose = render(() => step.component({ store: stubStore() as any }), host);
}

describe('FE-T8 W6 the payment step folds the queue card', () => {
  it('T8-7 while step 3 is on screen the queue card yields', async () => {
    expect(queueCardYielding(), 'nothing holds the card before step 3').toBe(false);
    mount();
    expect(host.querySelector('select[name="method"]'), 'the payment form rendered').not.toBeNull();
    await vi.waitFor(() => expect(queueCardYielding()).toBe(true));
  });

  it('T8-8 leaving step 3 gives the card back', async () => {
    mount();
    await vi.waitFor(() => expect(queueCardYielding()).toBe(true));
    dispose!();
    dispose = undefined;
    expect(queueCardYielding()).toBe(false);
  });
});
