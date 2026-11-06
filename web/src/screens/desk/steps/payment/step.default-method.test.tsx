// #564 F38, owner ruling R5 (A) (2026-11-06 07:56 ICT, #564 issuecomment-6673699296): "The desk reads the kiosk's
// owner-set defaultPaymentMethod (one setting, default Cash). Cash needs no account name."
//
// Written before the implementation by the ticket author (claude/33691c06). Do not edit, delete or weaken these
// tests; add your own elsewhere.
//
// [required] by these tests:
// - On mount with no method chosen (`store.method() === ''`), step 3 selects `store.config().defaultPaymentMethod`
//   (generated `DeskConfig`, answered by the desk config view from the branch's `kioskScreen` setting) when that
//   method is open for `store.type()`; otherwise the first open method in `METHODS` order, as today. A method
//   already chosen is kept.
// - Submit with method Cash and an empty (or blank) account is not refused; every other refusal stays (passport,
//   net, the count confirm). A non-cash method with an empty account is still refused with v2's `incomplete`.
// The store is a stub with §5.3's names, the same seam as `step.test.tsx` / `step.account.test.tsx`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import {
  makeDeskPaymentMethod,
  makeQuoteDto,
  makeQuoteLineDto,
  makeQuotePublicationDto,
  type DeskPaymentMethod,
  type QuoteDto,
} from '../../../../shared/testing/fixtures';

type StepModule = {
  step: { index: number; label: () => string; component: (p: { store: any }) => JSX.Element };
};
const stepMods = import.meta.glob<StepModule>('./step.tsx');

async function load<T>(mods: Record<string, () => Promise<T>>, key: string): Promise<T> {
  const l = mods[key];
  expect(l, `web/src/screens/desk/steps/payment/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

const INCOMPLETE = 'ตรวจชื่อบัญชี เลขอ้างอิง สลิป พาสปอร์ต และยอดสุทธิให้ครบ';
const PATHS = { OWNER_DESK: { kind: 'owner' }, STAFF_DESK: { kind: 'staff' } } as const;

const rule = (id: string, over: Partial<DeskPaymentMethod> = {}) =>
  makeDeskPaymentMethod({ id, enabled: true, buy: true, sell: true, account: '', instruction: '', ...over });

/** Every method open both ways: Credit Card is first in `METHODS`, so today's mount picks it (the F38 symptom). */
const ALL_OPEN = [
  rule('Credit Card', { account: 'Card terminal' }),
  rule('Alipay'),
  rule('PromptPay'),
  rule('Cash'),
  rule('Bank Transfer', { account: 'SCB 123-4-56789-0' }),
];

/** Cash, Bank Transfer and Alipay open for BUY; PromptPay SELL only; Credit Card off. */
const MIXED = [
  rule('Cash'),
  rule('Bank Transfer', { account: 'SCB 123-4-56789-0' }),
  rule('PromptPay', { buy: false }),
  rule('Credit Card', { enabled: false }),
  rule('Alipay'),
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

type Init = {
  rules?: DeskPaymentMethod[];
  /** `undefined` = the config has no such field (an older server). */
  defaultPaymentMethod?: string;
  type?: 'BUY' | 'SELL';
  method?: string;
  ref?: string;
  account?: string;
  slip?: File;
  passport?: boolean;
  checked?: boolean;
};

function stubStore(path: unknown, init: Init = {}) {
  const cfg: Record<string, unknown> = { shiftOpen: true, paymentMethods: init.rules ?? MIXED };
  if (init.defaultPaymentMethod !== undefined) cfg.defaultPaymentMethod = init.defaultPaymentMethod;
  const [config] = createSignal<any>(cfg);
  const [type] = createSignal(init.type ?? 'BUY');
  const [q] = createSignal<QuoteDto | undefined>(quote('3512.00'));
  const [method, setMethod] = createSignal(init.method ?? '');
  const [ref, setRef] = createSignal(init.ref ?? '');
  const [account, setAccount] = createSignal(init.account ?? '');
  const [fee, setFee] = createSignal('');
  const [bankFee, setBankFee] = createSignal('');
  const [notes, setNotes] = createSignal('');
  const [slip, setSlip] = createSignal<File | undefined>(init.slip);
  const [paymentChecked, setPaymentChecked] = createSignal(init.checked ?? false);
  const [countConfirmed, setCountConfirmed] = createSignal(false);
  const [step] = createSignal(3);
  const passport = init.passport === false ? undefined : new File([new Uint8Array([1])], 'p.jpg', { type: 'image/jpeg' });
  return {
    path,
    step,
    config,
    configError: () => undefined,
    type,
    quote: q,
    quoting: () => false,
    passport: () => passport,
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
let confirmSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  vi.stubGlobal(
    'fetch',
    vi.fn(() => Promise.reject(new Error('step 3 must not call the network itself'))),
  );
  confirmSpy = vi.fn(() => true);
  vi.stubGlobal('confirm', confirmSpy);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

async function mount(store: ReturnType<typeof stubStore>) {
  const { step } = await load(stepMods, './step.tsx');
  dispose = render(() => step.component({ store }), host);
  await Promise.resolve();
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
}

function unmount() {
  dispose?.();
  dispose = undefined;
  host.innerHTML = '';
}

const select = () => host.querySelector<HTMLSelectElement>('select[name="method"]')!;
const submit = () => host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
const settle = async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};
const SLIP = () => new File([new Uint8Array([1])], 's.png', { type: 'image/png' });

for (const which of ['OWNER_DESK', 'STAFF_DESK'] as const) {
  describe(`steps/payment: the owner-set default method (F38, R5) on ${which}`, () => {
    it('D1 configured Cash with every method open: Cash is selected, not the first method (Credit Card)', async () => {
      const store = stubStore(PATHS[which], { rules: ALL_OPEN, defaultPaymentMethod: 'Cash' });
      await mount(store);
      expect(store.method()).toBe('Cash');
      expect(select().value).toBe('Cash');
    });

    it('D2 configured Bank Transfer, open: selected, and its row account filled as a chosen method is', async () => {
      const store = stubStore(PATHS[which], { rules: MIXED, defaultPaymentMethod: 'Bank Transfer' });
      await mount(store);
      expect(store.method()).toBe('Bank Transfer');
      expect(select().value).toBe('Bank Transfer');
      expect(store.account()).toBe('SCB 123-4-56789-0');
    });

    it('D3 the configured method is read for the direction: PromptPay (SELL only) is selected on a SELL', async () => {
      const store = stubStore(PATHS[which], { rules: MIXED, type: 'SELL', defaultPaymentMethod: 'PromptPay' });
      await mount(store);
      expect(store.method()).toBe('PromptPay');
    });

    it("D4 configured method not open (off, wrong direction, unknown): the first open method in METHODS order", async () => {
      for (const [defaultPaymentMethod, type, expected] of [
        ['Credit Card', 'BUY', 'Alipay'],
        ['PromptPay', 'BUY', 'Alipay'],
        ['Bitcoin', 'BUY', 'Alipay'],
        ['', 'BUY', 'Alipay'],
      ] as const) {
        const store = stubStore(PATHS[which], { rules: MIXED, type, defaultPaymentMethod });
        await mount(store);
        expect(store.method(), `${defaultPaymentMethod} ${type}`).toBe(expected);
        unmount();
      }
    });

    it('D5 a config without the field: the first open method, as today', async () => {
      const store = stubStore(PATHS[which], { rules: MIXED });
      await mount(store);
      expect(store.method()).toBe('Alipay');
    });

    it('D6 a method already chosen (back from a later step) is kept over the configured default', async () => {
      const store = stubStore(PATHS[which], {
        rules: ALL_OPEN,
        defaultPaymentMethod: 'Cash',
        method: 'Bank Transfer',
        account: 'SCB 123-4-56789-0',
      });
      await mount(store);
      expect(store.method()).toBe('Bank Transfer');
      expect(store.setMethod).not.toHaveBeenCalledWith('Cash');
    });
  });

  describe(`steps/payment: Cash needs no account name (F38, R5) on ${which}`, () => {
    it('C1 Cash with an empty or blank account: no refusal, the count confirm, submit once, go(4)', async () => {
      for (const account of ['', '   ']) {
        const store = stubStore(PATHS[which], { rules: MIXED, method: 'Cash', account, checked: true });
        await mount(store);
        submit().click();
        await settle();
        expect(host.textContent, JSON.stringify(account)).not.toContain(INCOMPLETE);
        expect(confirmSpy, JSON.stringify(account)).toHaveBeenCalledTimes(1);
        expect(store.setRef).toHaveBeenCalledWith('—');
        expect(store.submit, JSON.stringify(account)).toHaveBeenCalledTimes(1);
        expect(store.go).toHaveBeenCalledWith(4);
        confirmSpy.mockClear();
        unmount();
      }
    });

    it('C2 the configured Cash default, mounted with no account row, submits without an account', async () => {
      const store = stubStore(PATHS[which], { rules: ALL_OPEN, defaultPaymentMethod: 'Cash', checked: true });
      await mount(store);
      expect(store.method()).toBe('Cash');
      expect(store.account()).toBe('');
      submit().click();
      await settle();
      expect(host.textContent).not.toContain(INCOMPLETE);
      expect(store.submit).toHaveBeenCalledTimes(1);
    });

    it('C3 Cash with no account but no passport: still refused, nothing sent', async () => {
      const store = stubStore(PATHS[which], { rules: MIXED, method: 'Cash', account: '', passport: false, checked: true });
      await mount(store);
      submit().click();
      await settle();
      expect(host.textContent).toContain(INCOMPLETE);
      expect(store.submit).not.toHaveBeenCalled();
    });

    it('C4 non-cash with reference and slip but no account: still refused with v2 incomplete, nothing sent', async () => {
      for (const method of ['Credit Card', 'Bank Transfer', 'SCB Transfer', 'Alipay', 'PromptPay']) {
        const store = stubStore(PATHS[which], {
          rules: ALL_OPEN,
          method,
          ref: 'R1',
          account: '',
          slip: SLIP(),
          checked: true,
        });
        await mount(store);
        submit().click();
        await settle();
        expect(host.textContent, method).toContain(INCOMPLETE);
        expect(store.submit, method).not.toHaveBeenCalled();
        unmount();
      }
    });
  });
}
