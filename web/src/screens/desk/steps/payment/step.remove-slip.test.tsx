// Issue #2538: Desk payment step — clearing the slip file input value on "Remove slip"
// so choosing the same slip file again fires `change` / `onSlip` and stores the slip.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, Show } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import type { DeskStore } from '../../desk-store';
import {
  makeDeskPaymentMethod,
  makeQuoteDto,
  makeQuoteLineDto,
  makeQuotePublicationDto,
  type QuoteDto,
} from '../../../../shared/testing/fixtures';
import { step } from './step';
import * as texts from './i18n';

const REMOVE_SLIP = texts.th.removeSlip; // 'ลบสลิป'
const INCOMPLETE = texts.th.incomplete;

const PATHS = {
  OWNER_DESK: { kind: 'owner' },
  STAFF_DESK: { kind: 'staff' },
} as const;

const SLIP_PNG = new File([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], 'slip.png', {
  type: 'image/png',
});

const RULES = [
  makeDeskPaymentMethod({
    id: 'Bank Transfer',
    enabled: true,
    buy: true,
    sell: true,
    account: 'SCB 123-4-56789-0',
    instruction: 'โอนเข้าบัญชีร้าน',
  }),
];

function quote(settleThb: string) {
  return makeQuoteDto({
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
        settleThb,
        publishedRateLineId: 'l',
      }),
    ],
    benefits: [],
    stale: false,
  });
}

function stubStore(path: unknown) {
  const [config] = createSignal<any>({ shiftOpen: true, paymentMethods: RULES });
  const [type] = createSignal<'BUY' | 'SELL'>('BUY');
  const [q] = createSignal<QuoteDto | undefined>(quote('3512.00'));
  const [method, setMethod] = createSignal('Bank Transfer');
  const [ref, setRef] = createSignal('REF-123');
  const [account, setAccount] = createSignal('SCB 123-4-56789-0');
  const [fee, setFee] = createSignal('');
  const [bankFee, setBankFee] = createSignal('');
  const [notes, setNotes] = createSignal('');
  const [slip, setSlip] = createSignal<File | undefined>();
  const [paymentChecked, setPaymentChecked] = createSignal(true);
  const [countConfirmed, setCountConfirmed] = createSignal(false);
  const [stepIdx, setStepIdx] = createSignal(3);
  const order: string[] = [];

  return {
    path,
    step: stepIdx,
    config,
    configError: () => undefined,
    type,
    quote: q,
    quoting: () => false,
    passport: () => new File([new Uint8Array([1])], 'p.jpg', { type: 'image/jpeg' }),
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
    setCountConfirmed: vi.fn((v: boolean) => {
      order.push(`countConfirmed=${v}`);
      setCountConfirmed(v);
    }),
    submit: vi.fn(async () => {
      order.push('submit');
      return 'saved';
    }),
    backToEdit: vi.fn(),
    go: vi.fn((i: number) => {
      order.push(`go(${i})`);
      setStepIdx(i);
    }),
    _order: order,
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  vi.stubGlobal('confirm', () => true);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

function mountStep(store: ReturnType<typeof stubStore>) {
  dispose = render(
    () => (
      <Show when={store.step() === 3}>
        {step.component({ store: store as unknown as DeskStore })}
      </Show>
    ),
    host,
  );
}

const q = <T extends Element>(sel: string) => host.querySelector<T>(sel);
const button = (text: string) =>
  [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) =>
    (b.textContent ?? '').includes(text),
  );

/**
 * Sets `input.files` with `Object.defineProperty` (and syncs jsdom's internal `impl.files`
 * so `input.value` reflects `C:\fakepath\<name>` and `input.value = ''` clears the list).
 * Mirrors browser file-picker semantics: `change` fires only when the chosen path differs
 * from the input's current `value`.
 */
function attachSlip(file: File): HTMLInputElement {
  const input = q<HTMLInputElement>('input[type="file"][name="slip"]');
  expect(input, 'the slip file input').not.toBeNull();
  const prevValue = input!.value;
  const list = [file];
  Object.defineProperty(input!, 'files', { value: list, configurable: true });
  const implSym = Object.getOwnPropertySymbols(input!).find((s) => s.description === 'impl');
  if (implSym) {
    Object.defineProperty((input as unknown as Record<symbol, object>)[implSym]!, 'files', {
      value: list,
      configurable: true,
    });
  }
  if (input!.value !== prevValue) {
    input!.dispatchEvent(new Event('change', { bubbles: true }));
  }
  return input!;
}

const submit = () => q<HTMLButtonElement>('button[type="submit"]')!.click();

describe('steps/payment: remove slip clears input value so re-picking the same slip works (#2538)', () => {
  it('row 1: Pick slip X, press Remove -> Input value is empty and no slip is stored', () => {
    for (const path of Object.values(PATHS)) {
      const store = stubStore(path);
      mountStep(store);

      const input = attachSlip(SLIP_PNG);
      expect(input.value).toBe('C:\\fakepath\\slip.png');
      expect(store.slip()).toBe(SLIP_PNG);

      const removeBtn = button(REMOVE_SLIP);
      expect(removeBtn, 'the remove slip button').toBeDefined();
      removeBtn!.click();

      // Crucial assertion: input.value must be reset to ''
      expect(input.value).toBe('');
      expect(store.slip()).toBeUndefined();
      expect(button(REMOVE_SLIP)).toBeUndefined();

      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
    }
  });

  it('row 2: Then pick X again -> onSlip runs and the slip is stored again, and Submit succeeds', async () => {
    for (const path of Object.values(PATHS)) {
      const store = stubStore(path);
      mountStep(store);

      const input = attachSlip(SLIP_PNG);
      expect(store.slip()).toBe(SLIP_PNG);

      button(REMOVE_SLIP)!.click();
      expect(store.slip()).toBeUndefined();

      // Attempting to submit while removed yields incomplete
      submit();
      expect(host.textContent).toContain(INCOMPLETE);
      expect(store.submit).not.toHaveBeenCalled();

      // Pick the same file X again
      store.setSlip.mockClear();
      attachSlip(SLIP_PNG);

      expect(store.setSlip).toHaveBeenCalledTimes(1);
      expect(store.setSlip).toHaveBeenCalledWith(SLIP_PNG);
      expect(store.slip()).toBe(SLIP_PNG);
      expect(input.value).toBe('C:\\fakepath\\slip.png');

      // Now submitting with the re-picked slip succeeds
      submit();
      expect(store.submit).toHaveBeenCalledTimes(1);
      expect(store.go).toHaveBeenCalledWith(4);

      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
    }
  });

  // Kept rows from §8:
  // - "Pick slip X, Submit" is kept and pinned by step.test.tsx.
  // - "Account selection" is kept and pinned by step.account.test.tsx.
});
