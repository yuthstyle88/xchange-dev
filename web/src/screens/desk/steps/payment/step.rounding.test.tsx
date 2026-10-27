// #5229 CR-d: the payment step shows the quote line's `roundingThb` next to the net when it is not 0.00 (design
// docs/design/blueprint/quote-transaction.md §13.8 CR-d; generated `QuoteLineDto.roundingThb`).
// Written before the implementation by the FE lane's test-writer (claude/8be3a451's order). Do not edit, delete or
// weaken these tests; add your own in another file.
//
// [required] by this test (behaviour table row R10 of #5229):
// - Next to the net, an element marked `data-row="rounding"` with the summary's text ("ปัดเศษเงินสด", `{roundingThb}
//   THB`) when `roundingThb` is "0.15".
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import { makeDeskPaymentMethod, makeQuoteDto, makeQuoteLineDto } from '../../../../shared/testing/fixtures';

type StepModule = { step: { index: number; label: () => string; component: (p: { store: any }) => JSX.Element } };
const stepMods = import.meta.glob<StepModule>('./step.tsx');

async function load<T>(mods: Record<string, () => Promise<T>>, key: string): Promise<T> {
  const l = mods[key];
  expect(l, `web/src/screens/desk/steps/payment/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

function stubStore(roundingThb: string) {
  const [config] = createSignal({
    shiftOpen: true,
    paymentMethods: [makeDeskPaymentMethod({ id: 'Cash', enabled: true, buy: true, sell: true, account: 'Cash drawer' })],
  });
  const [quote] = createSignal(
    makeQuoteDto({
      id: 'q-1',
      type: 'BUY',
      channel: 'DESK',
      expiresAt: new Date(Date.now() + 300_000).toISOString(),
      cashSettlement: true,
      lines: [
        makeQuoteLineDto({
          currency: 'USD',
          shopSide: 'BUY',
          quantity: '100',
          thbAmount: '3512.00',
          feeThb: '0.00',
          roundingThb,
          settleThb: '3511.85',
        }),
      ],
    }),
  );
  const [method, setMethod] = createSignal('Cash');
  const [ref, setRef] = createSignal('');
  const [account, setAccount] = createSignal('Cash drawer');
  const [fee, setFee] = createSignal('');
  const [bankFee, setBankFee] = createSignal('');
  const [notes, setNotes] = createSignal('');
  const [slip, setSlip] = createSignal<File | undefined>();
  const [paymentChecked, setPaymentChecked] = createSignal(false);
  const [countConfirmed, setCountConfirmed] = createSignal(false);
  return {
    path: { kind: 'owner' },
    step: () => 3,
    config,
    configError: () => undefined,
    type: () => 'BUY',
    quote,
    quoting: () => false,
    quoteError: () => undefined,
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

describe('payment step cash rounding', () => {
  it('R10 with roundingThb "0.15": a [data-row="rounding"] with "0.15"', async () => {
    const { step } = await load(stepMods, './step.tsx');
    dispose = render(() => step.component({ store: stubStore('0.15') }), host);
    await Promise.resolve();
    const rows = [...host.querySelectorAll('[data-row="rounding"]')];
    expect(rows.length, 'a rounding row on the payment step').toBeGreaterThan(0);
    expect(rows.map((el) => el.textContent ?? '').join(' ')).toContain('0.15');
  });
});
