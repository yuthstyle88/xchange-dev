import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import type { DeskPaymentMethod } from '../../../../shared/api/generated/desk_config';
import type { QuoteDto } from '../../../../shared/api/generated/quote';

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

const rule = (id: string, over: Partial<DeskPaymentMethod> = {}): DeskPaymentMethod => ({
  id,
  enabled: true,
  buy: true,
  sell: true,
  account: '',
  instruction: '',
  ...over,
});

const RULES: DeskPaymentMethod[] = [
  rule('Cash', { account: 'Cash drawer', instruction: 'นับเงินต่อหน้าลูกค้า' }),
  rule('Bank Transfer', { account: 'SCB 123-4-56789-0', instruction: 'โอนเข้าบัญชีร้าน' }),
  rule('PromptPay', { buy: false }),
  rule('Credit Card', { enabled: false }),
  rule('Alipay', { buy: false }),
];

function quote(settleThb: string): QuoteDto {
  return {
    id: 'q-1',
    type: 'BUY',
    channel: 'DESK',
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    publication: { id: 'p', version: 1 },
    lines: [
      {
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
      },
    ],
    benefits: [],
    stale: false,
  };
}

type Init = {
  rules?: DeskPaymentMethod[] | null;
  type?: 'BUY' | 'SELL';
  method?: string;
  ref?: string;
  account?: string;
  slip?: File;
  checked?: boolean;
};

function stubStore(init: Init = {}) {
  const [config] = createSignal<any>(
    init.rules === null ? undefined : { shiftOpen: true, paymentMethods: init.rules ?? RULES },
  );
  const [type] = createSignal(init.type ?? 'BUY');
  const [q] = createSignal<QuoteDto | undefined>(quote('3512.00'));
  const [method, setMethod] = createSignal(init.method ?? 'Cash');
  const [ref, setRef] = createSignal(init.ref ?? '');
  const [account, setAccount] = createSignal(init.account ?? '');
  const [fee, setFee] = createSignal('');
  const [bankFee, setBankFee] = createSignal('');
  const [notes, setNotes] = createSignal('');
  const [slip, setSlip] = createSignal<File | undefined>(init.slip);
  const [paymentChecked, setPaymentChecked] = createSignal(init.checked ?? false);
  const [countConfirmed, setCountConfirmed] = createSignal(false);
  const [step] = createSignal(3);
  return {
    path: { kind: 'staff' },
    step,
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
  return step;
}

const control = <T extends Element = HTMLInputElement>(name: string) =>
  host.querySelector<T & HTMLElement>(`[name="${name}"]`);
const select = () => control<HTMLSelectElement>('method')!;
const submit = () => host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
const settle = async () => {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};

function choose(v: string) {
  select().value = v;
  select().dispatchEvent(new Event('change', { bubbles: true }));
}

function type(name: string, value: string) {
  const el = control(name)!;
  (el as HTMLInputElement).value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('steps/payment: account & reference switching (#2356)', () => {
  it('row 1: Account "", Cash chosen (fills "Cash drawer"), then Bank Transfer sets account to Bank Transfer row account', async () => {
    const store = stubStore({ method: '', account: '' });
    await mount(store);
    await settle();
    expect(store.method()).toBe('Cash');
    expect(store.account()).toBe('Cash drawer');

    choose('Bank Transfer');
    await settle();
    expect(store.method()).toBe('Bank Transfer');
    expect(store.account()).toBe('SCB 123-4-56789-0');
    expect(control('account')!.value).toBe('SCB 123-4-56789-0');
  });

  it('row 2: Auto-filled "Cash drawer", then a method whose row account is "" clears account to "" and submit refuses as incomplete', async () => {
    const store = stubStore({
      rules: [
        rule('Cash', { account: 'Cash drawer' }),
        rule('Bank Transfer', { account: '' }),
      ],
      method: '',
      account: '',
      ref: 'BT-001',
      slip: new File([new Uint8Array([1])], 's.png', { type: 'image/png' }),
      checked: true,
    });
    await mount(store);
    await settle();
    expect(store.method()).toBe('Cash');
    expect(store.account()).toBe('Cash drawer');

    choose('Bank Transfer');
    await settle();
    expect(store.method()).toBe('Bank Transfer');
    expect(store.account()).toBe('');

    submit().click();
    await settle();
    expect(host.textContent).toContain(INCOMPLETE);
    expect(store.submit).not.toHaveBeenCalled();
  });

  it('row 3: Ref "—" from a Cash submit, then switching to Bank Transfer clears ref to ""', async () => {
    const store = stubStore({
      method: 'Cash',
      account: 'Cash drawer',
      slip: new File([new Uint8Array([1])], 's.png', { type: 'image/png' }),
      checked: true,
    });
    await mount(store);

    submit().click();
    await settle();
    expect(store.ref()).toBe('—');
    expect(store.submit).toHaveBeenCalledTimes(1);

    choose('Bank Transfer');
    await settle();
    expect(store.method()).toBe('Bank Transfer');
    expect(store.ref()).toBe('');
    expect(store.account()).toBe('SCB 123-4-56789-0');

    submit().click();
    await settle();
    expect(host.textContent).toContain(INCOMPLETE);
    expect(store.submit).toHaveBeenCalledTimes(1);
  });

  it('row 4: A typed account ("my own account") is not overwritten when switching methods, whereas a rule prefilled account is overwritten', async () => {
    const store = stubStore({ method: 'Cash', account: 'my own account' });
    await mount(store);

    choose('Bank Transfer');
    await settle();
    expect(store.account()).toBe('my own account');

    store.setAccount('Cash drawer');
    choose('Bank Transfer');
    await settle();
    expect(store.account()).toBe('SCB 123-4-56789-0');
  });

  it('row 5: Empty account is filled from the chosen row with its instruction shown, and switching to another method updates both instruction and auto-filled account', async () => {
    const store = stubStore({ method: 'Cash', account: '' });
    await mount(store);

    choose('Cash');
    await settle();
    expect(store.account()).toBe('Cash drawer');
    expect(host.textContent).toContain('นับเงินต่อหน้าลูกค้า');

    choose('Bank Transfer');
    await settle();
    expect(store.account()).toBe('SCB 123-4-56789-0');
    expect(host.textContent).toContain('โอนเข้าบัญชีร้าน');
  });

  it('edge case 1: A typed value that equals a row account ("Cash drawer") is treated as auto-filled and updated when switching to Bank Transfer', async () => {
    const store = stubStore({ method: 'Cash', account: '' });
    await mount(store);

    type('account', 'Cash drawer');
    expect(store.account()).toBe('Cash drawer');

    choose('Bank Transfer');
    await settle();
    expect(store.account()).toBe('SCB 123-4-56789-0');
  });

  it('edge case 2: Switching from Cash to Bank Transfer and back to Cash refills "Cash drawer"', async () => {
    const store = stubStore({ method: '', account: '' });
    await mount(store);
    await settle();
    expect(store.account()).toBe('Cash drawer');

    choose('Bank Transfer');
    await settle();
    expect(store.account()).toBe('SCB 123-4-56789-0');

    choose('Cash');
    await settle();
    expect(store.account()).toBe('Cash drawer');
  });
});
