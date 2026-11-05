// Ticket #6327: desk payment methods are named in the screen's language.
// In Thai every option's text is the Thai name and its value the wire value;
// in English the English name. Red when reverted.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import { makeQuoteDto, makeQuoteLineDto } from '../../../../shared/testing/fixtures';

type StepModule = { step: { index: number; label: () => string; component: (p: { store: any }) => JSX.Element } };
const stepMods = import.meta.glob<StepModule>('./step.tsx');

async function load<T>(mods: Record<string, () => Promise<T>>, key: string): Promise<T> {
  const l = mods[key];
  expect(l, `web/src/screens/desk/steps/payment/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

function stubStore() {
  const [config] = createSignal({
    shiftOpen: true,
    paymentMethods: [],
  });
  const [quote] = createSignal(
    makeQuoteDto({
      id: 'q-1',
      type: 'BUY',
      channel: 'DESK',
      expiresAt: new Date(Date.now() + 300_000).toISOString(),
      lines: [
        makeQuoteLineDto({
          currency: 'USD',
          shopSide: 'BUY',
          quantity: '100',
          thbAmount: '3512.00',
          feeThb: '0.00',
          settleThb: '3512.00',
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

describe('payment method names in the screen language (#6327)', () => {
  it('in Thai every option text is the Thai name and its value is the wire value', async () => {
    setLang('th');
    const { step } = await load(stepMods, './step.tsx');
    dispose = render(() => step.component({ store: stubStore() }), host);
    await Promise.resolve();

    const options = [...host.querySelectorAll<HTMLOptionElement>('select[name="method"] option')];
    expect(options.length).toBe(6);

    expect(options.map((o) => ({ value: o.value, text: o.textContent?.trim() }))).toEqual([
      { value: 'Credit Card', text: 'บัตรเครดิต' },
      { value: 'Alipay', text: 'Alipay' },
      { value: 'PromptPay', text: 'พร้อมเพย์' },
      { value: 'Cash', text: 'เงินสด' },
      { value: 'Bank Transfer', text: 'โอนธนาคาร' },
      { value: 'SCB Transfer', text: 'โอน SCB' },
    ]);
  });

  it('in English every option text is the English name and its value is the wire value', async () => {
    setLang('en');
    const { step } = await load(stepMods, './step.tsx');
    dispose = render(() => step.component({ store: stubStore() }), host);
    await Promise.resolve();

    const options = [...host.querySelectorAll<HTMLOptionElement>('select[name="method"] option')];
    expect(options.length).toBe(6);

    expect(options.map((o) => ({ value: o.value, text: o.textContent?.trim() }))).toEqual([
      { value: 'Credit Card', text: 'Credit Card' },
      { value: 'Alipay', text: 'Alipay' },
      { value: 'PromptPay', text: 'PromptPay' },
      { value: 'Cash', text: 'Cash' },
      { value: 'Bank Transfer', text: 'Bank Transfer' },
      { value: 'SCB Transfer', text: 'SCB Transfer' },
    ]);
  });
});
