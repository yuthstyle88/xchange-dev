// F1 FE: the rate step says the drawer advice is for a cash payout when the quote says so (#564 finding F1;
// server side PR 6039 / #6036: `QuoteDto.drawerCheckIfCash`, design ledger-inventory.md §5.1.3 item 1a).
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests:
// - C1: `drawerCheckIfCash` true with a shortfall ⇒ the alert's first line is the lead `หากจ่ายเป็นเงินสด: ` followed
//   by `drawerShortTitle`; the server message and `drawerShortNext` follow unchanged, without the lead.
// - C2: `drawerCheckIfCash` true, no shortfall, a low currency ⇒ the low note reads the lead followed by `drawerLowAfter`.
// - C3: `drawerCheckIfCash` false ⇒ both warnings read exactly as today, with no lead anywhere.
// - C4: `drawerCheckIfCash` true with no advice ⇒ nothing is shown (the lead never stands alone).
// - C5: the lead follows the latest quote.
// - C6: in English the lead is `If paid in cash: `.
// The Thai lead is the owner-approved text (#564 issuecomment-6596511858 and issuecomment-6596705394).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { makeQuoteDto, makeQuoteLineDto } from '../../../../shared/testing/fixtures';
import { OWNER_DESK, STAFF_DESK } from '../../desk-path';
import { createDeskStore, type DeskStore } from '../../desk-store';
import { step } from './step';

const ME = () => ({ id: 'u-1', name: 'Ploy', role: 'TELLER', branch: 'MAIN', permissions: [] });

const LEAD_TH = 'หากจ่ายเป็นเงินสด: ';
const LEAD_EN = 'If paid in cash: ';
const SHORT_TITLE_TH = 'เงินในลิ้นชักไม่พอจ่ายรายการนี้';
const SHORT_NEXT_TH = 'ให้ผู้จัดการสาขาเบิก OD จากตู้เซฟเข้าลิ้นชักก่อนบันทึกรายการ';
const LOW_AFTER_TH = (c: string) => `หลังรายการนี้ เงินในลิ้นชักจะเหลือน้อย ควรเบิก OD: ${c}`;
const SHORT_TITLE_EN = 'The drawer does not hold enough to pay this';
const LOW_AFTER_EN = (c: string) => `After this transaction the drawer will be low on: ${c}`;

// The server's text, passed through as sent.
const SERVER_MESSAGE = 'ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: THB ลิ้นชัก ขาด 24.25';

const RATES = [{ code: 'USD', unitSize: '1', buy: '32.10', sell: '32.90', publication: 'pub-1' }];

type QuoteOverrides = NonNullable<Parameters<typeof makeQuoteDto>[0]>;

function quote(o: QuoteOverrides) {
  return makeQuoteDto({
    id: 'q-1',
    type: 'BUY',
    channel: 'DESK',
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    lines: [makeQuoteLineDto({ currency: 'USD', shopSide: 'BUY', quantity: '2', settleThb: '64.20' })],
    ...o,
  });
}

const SHORT_IF_CASH = quote({
  drawerCheckIfCash: true,
  drawerShortfall: [{ currency: 'THB', place: 'DRAWER', shortBy: '24.25' }],
  drawerShortfallMessage: SERVER_MESSAGE,
});
const LOW_IF_CASH = quote({ drawerCheckIfCash: true, belowLowCashLevel: ['THB'] });
const CLEAR_IF_CASH = quote({ drawerCheckIfCash: true });
const SHORT_CHOSEN = quote({
  drawerCheckIfCash: false,
  drawerShortfall: [{ currency: 'THB', place: 'DRAWER', shortBy: '24.25' }],
  drawerShortfallMessage: SERVER_MESSAGE,
});
const LOW_CHOSEN = quote({ drawerCheckIfCash: false, belowLowCashLevel: ['THB'] });

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
  store.setType('BUY');
  await vi.waitFor(() => expect(store.currency()).toBe('USD'));
  store.setAmount('2');
  await store.requote();
  await vi.waitFor(() => expect(store.quote()).toBeTruthy());
  return store;
}

const shortfall = () => host.querySelector<HTMLElement>('[data-testid="drawer-shortfall"]');
const lowAfter = () => host.querySelector<HTMLElement>('[data-testid="drawer-low-after"]');
const lines = (el: HTMLElement) => [...el.querySelectorAll('p')].map((p) => p.textContent ?? '');

describe('F1 drawer advice for a cash payout at key-in', () => {
  it('C1 a shortfall if paid in cash: the lead opens the title; the message and the next step follow unchanged', async () => {
    await mountQuoted(() => SHORT_IF_CASH);
    await vi.waitFor(() => expect(shortfall()).not.toBeNull());
    const el = shortfall()!;
    expect(el.getAttribute('role')).toBe('alert');
    expect(lines(el)).toEqual([LEAD_TH + SHORT_TITLE_TH, SERVER_MESSAGE, SHORT_NEXT_TH]);
    expect(lowAfter()).toBeNull();
  });

  it('C2 low after if paid in cash: the lead opens the low note', async () => {
    await mountQuoted(() => LOW_IF_CASH);
    await vi.waitFor(() => expect(lowAfter()).not.toBeNull());
    expect(lowAfter()!.textContent?.trim()).toBe(LEAD_TH + LOW_AFTER_TH('THB'));
    expect(shortfall()).toBeNull();
  });

  it('C3 a quote whose method is chosen keeps both warnings exactly as today, with no lead', async () => {
    await mountQuoted(() => SHORT_CHOSEN);
    await vi.waitFor(() => expect(shortfall()).not.toBeNull());
    expect(lines(shortfall()!)).toEqual([SHORT_TITLE_TH, SERVER_MESSAGE, SHORT_NEXT_TH]);
    expect(host.textContent).not.toContain(LEAD_TH.trim());
    dispose?.();
    dispose = undefined;
    host.innerHTML = '';

    await mountQuoted(() => LOW_CHOSEN);
    await vi.waitFor(() => expect(lowAfter()).not.toBeNull());
    expect(lowAfter()!.textContent?.trim()).toBe(LOW_AFTER_TH('THB'));
    expect(host.textContent).not.toContain(LEAD_TH.trim());
  });

  it('C4 if paid in cash with nothing to advise: no warning and no lead on its own', async () => {
    await mountQuoted(() => CLEAR_IF_CASH);
    expect(shortfall()).toBeNull();
    expect(lowAfter()).toBeNull();
    expect(host.textContent).not.toContain(LEAD_TH.trim());
  });

  it('C5 the lead follows the latest quote', async () => {
    let next: unknown = SHORT_IF_CASH;
    const store = await mountQuoted(() => next);
    await vi.waitFor(() => expect(shortfall()).not.toBeNull());
    expect(lines(shortfall()!)[0]).toBe(LEAD_TH + SHORT_TITLE_TH);

    next = { ...SHORT_CHOSEN, id: 'q-2' };
    await store.requote();
    await vi.waitFor(() => expect(store.quote()?.id).toBe('q-2'));
    await vi.waitFor(() => expect(lines(shortfall()!)[0]).toBe(SHORT_TITLE_TH));
  });

  it('C6 in English the lead reads "If paid in cash: "', async () => {
    setLang('en');
    await mountQuoted(() => SHORT_IF_CASH);
    await vi.waitFor(() => expect(shortfall()).not.toBeNull());
    expect(lines(shortfall()!)[0]).toBe(LEAD_EN + SHORT_TITLE_EN);
    dispose?.();
    dispose = undefined;
    host.innerHTML = '';

    await mountQuoted(() => LOW_IF_CASH);
    await vi.waitFor(() => expect(lowAfter()).not.toBeNull());
    expect(lowAfter()!.textContent?.trim()).toBe(LEAD_EN + LOW_AFTER_EN('THB'));
  });
});
