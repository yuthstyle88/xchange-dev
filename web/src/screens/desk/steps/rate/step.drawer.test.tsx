// FE-K #5981: the rate step warns at key-in when the drawer cannot pay, or will be low after the transaction
// (design docs/design/xchange-v3-wave3-desk.md §5.4b K1-K4; server ledger-inventory.md §5.1.3 item 1).
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests:
// - K1: the quote's `drawerShortfall` is not empty ⇒ `[data-testid="drawer-shortfall"]` with `role="alert"`, holding
//   `drawerShortTitle`, then `drawerShortfallMessage` exactly as sent, then `drawerShortNext`.
// - K2: no shortfall, `belowLowCashLevel` not empty ⇒ `[data-testid="drawer-low-after"]` (no alert role) holding
//   `drawerLowAfter` with the currencies joined by ", ".
// - K3: both empty ⇒ neither element.
// - K4: the warning follows the latest quote.
// - The step holds no level or amount of its own: it shows only what the quote sends (user-set settings, PR #5993).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { makeQuoteDto, makeQuoteLineDto } from '../../../../shared/testing/fixtures';
import { OWNER_DESK, STAFF_DESK } from '../../desk-path';
import { createDeskStore, type DeskStore } from '../../desk-store';
import { step } from './step';

const ME = () => ({ id: 'u-1', name: 'Ploy', role: 'TELLER', branch: 'MAIN', permissions: [] });

// The exact words of §5.4b.
const SHORT_TITLE_TH = 'เงินในลิ้นชักไม่พอจ่ายรายการนี้';
// #6013: SHORT_NEXT_TH and LOW_AFTER_TH restated to the OD wording the owner approved ("ok ข้อความ OD ใช้ได้", #564).
const SHORT_NEXT_TH = 'ให้ผู้จัดการสาขาเบิก OD จากตู้เซฟเข้าลิ้นชักก่อนบันทึกรายการ';
const LOW_AFTER_TH = (c: string) => `หลังรายการนี้ เงินในลิ้นชักจะเหลือน้อย ควรเบิก OD: ${c}`;
const SHORT_TITLE_EN = 'The drawer does not hold enough to pay this';
const SHORT_NEXT_EN = 'Ask the branch manager to move cash from the safe into the drawer before saving';
const LOW_AFTER_EN = (c: string) => `After this transaction the drawer will be low on: ${c}`;

// The server's text, passed through as sent (a value the web code never builds itself).
const SERVER_MESSAGE = 'ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: USD ลิ้นชัก ขาด 120.00';

const RATES = [{ code: 'USD', unitSize: '1', buy: '32.10', sell: '32.90', publication: 'pub-1' }];

type QuoteOverrides = NonNullable<Parameters<typeof makeQuoteDto>[0]>;

function quote(o: QuoteOverrides) {
  return makeQuoteDto({
    id: 'q-1',
    type: 'SELL',
    channel: 'DESK',
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    lines: [makeQuoteLineDto({ currency: 'USD', shopSide: 'SELL', quantity: '100', settleThb: '3290.00' })],
    ...o,
  });
}

const SHORT = quote({
  drawerShortfall: [{ currency: 'USD', place: 'DRAWER', shortBy: '120.00' }],
  drawerShortfallMessage: SERVER_MESSAGE,
});
const LOW_AFTER = quote({ belowLowCashLevel: ['USD', 'EUR'] });
const CLEAR = quote({});

function fakeApi(next: () => unknown) {
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input).split('?')[0];
    const json = (status: number, body: unknown) =>
      new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
    if (path === `/api/v3/views/${OWNER_DESK.configView}` || path === `/api/v3/views/${STAFF_DESK.configView}`) {
      return json(200, {
        version: 1,
        changed: true,
        data: { branchId: 'MAIN', branchName: 'XC-B', shiftOpen: true, adminUrl: '/?branch=MAIN', rates: RATES, paymentMethods: [] },
      });
    }
    if (path === '/api/quotes' && String(init?.method).toUpperCase() === 'POST') return json(200, next());
    return json(404, { error: { code: 'notFound', message: 'nf' } });
  });
  return new ApiClient(fetchFn as unknown as typeof fetch);
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.restoreAllMocks();
  setLang('th');
});

async function mountQuoted(next: () => unknown): Promise<DeskStore> {
  const api = fakeApi(next);
  let store!: DeskStore;
  dispose = render(() => {
    store = createDeskStore({ api, path: OWNER_DESK, me: ME as any });
    return step.component({ store });
  }, host);
  await vi.waitFor(() => expect(store.config()).toBeDefined());
  store.setType('SELL');
  await vi.waitFor(() => expect(store.currency()).toBe('USD'));
  store.setAmount('100');
  await store.requote();
  await vi.waitFor(() => expect(store.quote()).toBeTruthy());
  return store;
}

const shortfall = () => host.querySelector<HTMLElement>('[data-testid="drawer-shortfall"]');
const lowAfter = () => host.querySelector<HTMLElement>('[data-testid="drawer-low-after"]');

describe('FE-K drawer warning at key-in (§5.4b)', () => {
  it('K1 a shortfall shows the alert: title, the server message as sent, then the next step', async () => {
    await mountQuoted(() => SHORT);
    await vi.waitFor(() => expect(shortfall()).not.toBeNull());
    const el = shortfall()!;
    expect(el.getAttribute('role')).toBe('alert');
    const text = el.textContent ?? '';
    const iTitle = text.indexOf(SHORT_TITLE_TH);
    const iMsg = text.indexOf(SERVER_MESSAGE);
    const iNext = text.indexOf(SHORT_NEXT_TH);
    expect(iTitle, 'title present').toBeGreaterThanOrEqual(0);
    expect(iMsg, 'server message present as sent').toBeGreaterThan(iTitle);
    expect(iNext, 'next step after the message').toBeGreaterThan(iMsg);
    expect(lowAfter(), 'K1 and K2 together: only K1').toBeNull();
  });

  it('K1 is a warning, not a block: the quote stays and no quote error is raised', async () => {
    const store = await mountQuoted(() => SHORT);
    await vi.waitFor(() => expect(shortfall()).not.toBeNull());
    expect(store.quote()?.id).toBe('q-1');
    expect(store.quoteError()).toBeFalsy();
  });

  it('K1 with a low currency as well still shows only the alert', async () => {
    await mountQuoted(() => quote({ ...SHORT, belowLowCashLevel: ['EUR'] }));
    await vi.waitFor(() => expect(shortfall()).not.toBeNull());
    expect(lowAfter()).toBeNull();
  });

  it('K2 no shortfall, low after: the note with the currencies joined by ", ", not an alert', async () => {
    await mountQuoted(() => LOW_AFTER);
    await vi.waitFor(() => expect(lowAfter()).not.toBeNull());
    const el = lowAfter()!;
    expect(el.textContent?.trim()).toBe(LOW_AFTER_TH('USD, EUR'));
    expect(el.getAttribute('role')).not.toBe('alert');
    expect(shortfall()).toBeNull();
  });

  it('K3 both empty: nothing', async () => {
    await mountQuoted(() => CLEAR);
    expect(shortfall()).toBeNull();
    expect(lowAfter()).toBeNull();
  });

  it('K3 fields absent (an older server): nothing, and no crash', async () => {
    const { drawerShortfall: _a, drawerShortfallMessage: _b, belowLowCashLevel: _c, ...older } = CLEAR;
    await mountQuoted(() => older);
    expect(shortfall()).toBeNull();
    expect(lowAfter()).toBeNull();
  });

  it('K4 the warning follows the latest quote', async () => {
    const answers = [SHORT, LOW_AFTER, CLEAR];
    let i = 0;
    const store = await mountQuoted(() => answers[Math.min(i++, answers.length - 1)]);
    await vi.waitFor(() => expect(shortfall()).not.toBeNull());

    store.setAmount('50');
    await store.requote();
    await vi.waitFor(() => expect(lowAfter()).not.toBeNull());
    expect(shortfall()).toBeNull();

    store.setAmount('10');
    await store.requote();
    await vi.waitFor(() => expect(lowAfter()).toBeNull());
    expect(shortfall()).toBeNull();
  });

  it('EN words', async () => {
    await mountQuoted(() => SHORT);
    setLang('en');
    await vi.waitFor(() => expect(shortfall()?.textContent).toContain(SHORT_TITLE_EN));
    expect(shortfall()!.textContent).toContain(SHORT_NEXT_EN);
    expect(shortfall()!.textContent).toContain(SERVER_MESSAGE);
  });

  it('EN low-after words', async () => {
    await mountQuoted(() => LOW_AFTER);
    setLang('en');
    await vi.waitFor(() => expect(lowAfter()?.textContent?.trim()).toBe(LOW_AFTER_EN('USD, EUR')));
  });
});
