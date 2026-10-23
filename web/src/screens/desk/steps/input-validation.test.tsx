// #4905 (audit T4): the desk exchange steps check what is typed before it is sent, and show each refusal at its field
// (design docs/design/web-input-validation.md; ticket §3 rows D1–D6).
// Written before the implementation by the sub-controller claude/studio-claude-4 (XC FE lane controller
// claude/e8020026's order). Do not edit, delete or weaken these tests; add your own elsewhere.
//
// Every row runs the real desk store over a fake server, so "no request" and "no save" are what reached the network.
// - The server refuses a bad amount on field `quantity` (api/src/presentation/http/v3/commands/quotes.rs:185-192);
//   the rate step shows it at the amount, worded with the shared `input.invalid` and the step's amount label.
// - Amount, fee and bank fee are checked with the shared bounded mirror (`web/src/shared/ui/bounded.ts`) before
//   anything is sent; the line sits inside the field's label, the way the rate step already shows amount errors.
// - The value date is required: a cleared date is refused on step 2 with a worded line, and step 3 is not reached.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import {
  makeDeskConfig,
  makeDeskPaymentMethod,
  makeDeskRate,
  makeQuoteDto,
  makeQuoteLineDto,
  makeQuotePublicationDto,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import { OWNER_DESK } from '../desk-path';
import { createDeskStore, type DeskStore } from '../desk-store';
import { step as rateStep } from './rate/step';
import { en as rateEn, th as rateTh } from './rate/i18n';
import { step as bookingStep } from './booking/step';
import { step as paymentStep } from './payment/step';

const THAI_SCRIPT = /[฀-๿]/;
const QUOTE_DEBOUNCE_WAIT_MS = 450;

const ME = () => ({
  userId: 'u-1',
  username: 'ploy',
  name: 'Ploy',
  roles: [],
  branch: 'MAIN',
  branchId: 'MAIN',
  branchName: 'XC-B',
  branches: [],
});

const PASSPORT = new File([new Uint8Array([1, 2, 3])], 'passport.jpg', { type: 'image/jpeg' });

type Answer = { status: number; body?: unknown };
type Route = (body: unknown) => Answer;
type Call = { method: string; path: string; body: unknown };

function fakeServer() {
  const routes = new Map<string, Route>();
  const calls: Call[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0]!;
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : init?.body;
    calls.push({ method, path, body });
    const route = routes.get(`${method} ${path}`);
    const a: Answer = route ? route(body) : { status: 404, body: { error: { code: 'notFound', message: 'not found' } } };
    return new Response(JSON.stringify(a.body ?? {}), {
      status: a.status,
      headers: { 'content-type': 'application/json' },
    });
  });
  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    calls,
    sent: (method: string, path: string) => calls.filter((c) => c.method === method && c.path === path),
    on(method: string, path: string, route: Route) {
      routes.set(`${method.toUpperCase()} ${path}`, route);
    },
  };
}

function quoteDto(quantity: string) {
  return makeQuoteDto({
    id: 'q-1',
    type: 'BUY',
    channel: 'DESK',
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    publication: makeQuotePublicationDto({ id: 'pub-1', version: 3 }),
    lines: [
      makeQuoteLineDto({
        currency: 'USD',
        shopSide: 'BUY',
        quantity,
        unitSize: '1',
        baseRate: '32.10',
        finalRate: '32.10',
        thbAmount: '3210.00',
        feeThb: '0.00',
        settleThb: '3210.00',
        publishedRateLineId: 'line-1',
      }),
    ],
    benefits: [],
    stale: false,
  });
}

const QUOTES = '/api/quotes';
const TRANSACTIONS = '/api/transactions';

let disposers: (() => void)[] = [];
let host: HTMLDivElement;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  vi.stubGlobal('confirm', vi.fn(() => true));
});

afterEach(() => {
  for (const d of disposers) d();
  disposers = [];
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

function makeStore(s: ReturnType<typeof fakeServer>): DeskStore {
  s.on('GET', '/api/v3/views/desk_config', () => ({
    status: 200,
    body: {
      version: 1,
      changed: true,
      data: makeDeskConfig({
        shiftOpen: true,
        rates: [makeDeskRate({ code: 'USD', unitSize: '1', buy: '32.10', sell: '32.50' })],
        paymentMethods: [makeDeskPaymentMethod({ id: 'Cash', enabled: true, buy: true, sell: true, account: 'Cash drawer 1' })],
      }),
    },
  }));
  s.on('POST', QUOTES, (body) => {
    const quantity = (body as { lines: { quantity: string }[] }).lines[0]!.quantity;
    return { status: 200, body: quoteDto(quantity) };
  });
  s.on('POST', '/api/upload', () => ({ status: 200, body: { uploadKey: 'key-1' } }));
  s.on('POST', TRANSACTIONS, () => ({ status: 500, body: { error: { code: 'serverError', message: 'must not be sent' } } }));
  let store!: DeskStore;
  createRoot((dispose) => {
    disposers.push(dispose);
    store = createDeskStore({ api: s.api, path: OWNER_DESK, me: ME as never });
  });
  return store;
}

/** A store with a good quote and every field step 3 needs, as a teller who reached the payment step leaves it. */
async function readyForPayment(s: ReturnType<typeof fakeServer>): Promise<DeskStore> {
  const store = makeStore(s);
  await vi.waitFor(() => expect(store.config()).toBeDefined());
  store.setType('BUY');
  store.setCurrency('USD');
  store.setAmount('100');
  await store.requote();
  expect(store.quote()).toBeDefined();
  store.setPassport(PASSPORT);
  store.setField('name', 'JOHN DOE', 'user');
  store.setField('passportNo', 'AA1234567', 'user');
  store.setField('nationality', 'USA', 'user');
  store.setField('birthDate', '1990-01-01', 'user');
  store.setField('passportExpiry', '2030-01-01', 'user');
  store.setIdentityChecked(true);
  store.setCustomerId('CX-00000001');
  store.setValueDate('2026-10-11');
  store.setMethod('Cash');
  store.setAccount('Cash drawer 1');
  store.setBankFee('0');
  store.setNotes('');
  store.go(3);
  return store;
}

function mount(step: { component: (p: { store: DeskStore }) => unknown }, store: DeskStore): void {
  disposers.push(render(() => step.component({ store }) as never, host));
}

const input = (name: string) => host.querySelector<HTMLInputElement>(`input[name="${name}"]`)!;

function type(name: string, value: string): void {
  const el = input(name);
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

/** The line shown inside the field's own label, or '' when there is none. */
function lineAt(name: string): string {
  const label = input(name).closest('label');
  expect(label, `input[name=${name}] sits in a label`).not.toBeNull();
  return (label!.querySelector('.desk-error')?.textContent ?? '').trim();
}

function submitForm(): void {
  host.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
}

function checkPayment(): void {
  const box = input('paymentChecked');
  box.checked = true;
  box.dispatchEvent(new Event('change', { bubbles: true }));
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe('desk exchange steps: input validation (#4905)', () => {
  it('D1 rate/amount: a 400 validationFailed on field "quantity" shows at the amount, worded with the amount label', async () => {
    setLang('en');
    const s = fakeServer();
    const store = makeStore(s);
    await vi.waitFor(() => expect(store.config()).toBeDefined());
    s.on('POST', QUOTES, () => ({
      status: 400,
      body: { error: { code: 'validationFailed', field: 'quantity', message: 'ข้อมูลไม่ถูกต้อง' } },
    }));
    mount(rateStep, store);

    store.setCurrency('USD');
    store.setAmount('100');
    await store.requote();

    const expected = sharedEn['input.invalid']!.replace('{field}', rateEn.amount);
    await vi.waitFor(() => expect(lineAt('amount')).toBe(expected));
    expect(host.textContent).not.toContain('ข้อมูลไม่ถูกต้อง');

    setLang('th');
    await store.requote();
    await vi.waitFor(() => expect(lineAt('amount')).toBe(sharedTh['input.invalid']!.replace('{field}', rateTh.amount)));
  });

  it('D2 rate/amount: "1,000" shows a worded line at the amount and sends no quote request', async () => {
    const s = fakeServer();
    const store = makeStore(s);
    await vi.waitFor(() => expect(store.config()).toBeDefined());
    mount(rateStep, store);
    store.setCurrency('USD');
    await wait(QUOTE_DEBOUNCE_WAIT_MS);
    const before = s.sent('POST', QUOTES).length;

    type('amount', '1,000');
    await wait(QUOTE_DEBOUNCE_WAIT_MS);
    await store.requote();
    submitForm();

    expect(s.sent('POST', QUOTES).filter((c) => JSON.stringify(c.body).includes('1,000'))).toEqual([]);
    expect(s.sent('POST', QUOTES).length).toBe(before);
    await vi.waitFor(() => expect(lineAt('amount')).not.toBe(''));
    expect(lineAt('amount')).toMatch(THAI_SCRIPT);
    expect(store.step()).toBe(0);
  });

  it('D3 payment/fee: "1.234" shows a worded line at the fee and Next is blocked', async () => {
    const s = fakeServer();
    const store = await readyForPayment(s);
    mount(paymentStep, store);

    type('fee', '1.234');
    await wait(QUOTE_DEBOUNCE_WAIT_MS);
    checkPayment();
    submitForm();

    await vi.waitFor(() => expect(lineAt('fee')).not.toBe(''));
    expect(lineAt('fee')).toMatch(THAI_SCRIPT);
    expect(store.step()).toBe(3);
    expect(s.sent('POST', TRANSACTIONS)).toEqual([]);
  });

  it('D4 payment/bank fee: "abc" shows a worded line at the bank fee and nothing is saved', async () => {
    const s = fakeServer();
    const store = await readyForPayment(s);
    mount(paymentStep, store);

    type('bankFee', 'abc');
    checkPayment();
    submitForm();
    await wait(50);

    await vi.waitFor(() => expect(lineAt('bankFee')).not.toBe(''));
    expect(lineAt('bankFee')).toMatch(THAI_SCRIPT);
    expect(store.step()).toBe(3);
    expect(store.outcome()).toBe('idle');
    expect(s.sent('POST', TRANSACTIONS)).toEqual([]);
  });

  it('D5 booking/valueDate: a cleared date is required and refused with a worded line; nothing is saved', async () => {
    const s = fakeServer();
    const store = await readyForPayment(s);
    store.go(2);
    mount(bookingStep, store);

    expect(input('valueDate').required).toBe(true);
    type('valueDate', '');
    expect(store.valueDate()).toBe('');
    submitForm();

    const alert = host.querySelector('[role="alert"]');
    expect(alert?.textContent?.trim() ?? '').not.toBe('');
    expect(alert!.textContent).toMatch(THAI_SCRIPT);
    expect(store.step()).toBe(2);
    expect(s.sent('POST', TRANSACTIONS)).toEqual([]);
  });

  it('D6 rate/amount: "100" is quoted as today, the text unchanged, and no line shows at the amount', async () => {
    const s = fakeServer();
    const store = makeStore(s);
    await vi.waitFor(() => expect(store.config()).toBeDefined());
    mount(rateStep, store);
    store.setCurrency('USD');

    type('amount', '100');
    await vi.waitFor(() =>
      expect(s.sent('POST', QUOTES).some((c) => (c.body as { lines: { quantity: string }[] }).lines[0]!.quantity === '100')).toBe(
        true,
      ),
    );
    await vi.waitFor(() => expect(store.quote()).toBeDefined());
    expect(lineAt('amount')).toBe('');
  });
});
