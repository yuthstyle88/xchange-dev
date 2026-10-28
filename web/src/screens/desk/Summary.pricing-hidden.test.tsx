import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { makeQuoteDto, makeQuoteLineDto, makeQuotePublicationDto } from '../../shared/testing/fixtures';
import type { DeskStore, StepIndex } from './desk-store';
import { Summary } from './Summary';

function makeQuote(pricingHidden: boolean, pricingHiddenReason: string | null = null) {
  return makeQuoteDto({
    id: 'q-1',
    type: 'BUY',
    channel: 'DESK',
    expiresAt: '2026-10-27T12:00:00Z',
    publication: makeQuotePublicationDto({ id: 'pub-1', version: 1 }),
    cashSettlement: true,
    lines: [
      makeQuoteLineDto({
        currency: 'USD',
        shopSide: 'BUY',
        quantity: '100',
        unitSize: '1',
        baseRate: '35.12000000',
        finalRate: '35.12000000',
        thbAmount: '3512.00',
        feeThb: '12.50',
        roundingThb: '0.00',
        settleThb: '3499.50',
        publishedRateLineId: 'l-1',
        pricingHidden,
        pricingHiddenReason,
      }),
    ],
  });
}

function stubStore(initialQuote?: ReturnType<typeof makeQuote>) {
  const [step] = createSignal<StepIndex>(3);
  const [quote, setQuote] = createSignal(initialQuote);
  const [couponCode, setCouponCode] = createSignal('');
  const store = {
    api: { view: vi.fn(), command: vi.fn() } as unknown as DeskStore['api'],
    branch: () => 'MAIN',
    step,
    type: () => 'BUY',
    currency: () => 'USD',
    amount: () => '100',
    method: () => 'Cash',
    quote,
    quoting: () => false,
    customerId: () => '',
    transactionState: () => undefined,
    couponCode,
    setCouponCode: vi.fn((c: string) => setCouponCode(c)),
    quoteError: () => undefined,
    entry: () => undefined,
  } as unknown as DeskStore;

  return { store, setQuote };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  setLang('th');
  vi.unstubAllGlobals();
});

function mount(store: DeskStore) {
  vi.stubGlobal(
    'fetch',
    vi.fn(() => Promise.reject(new Error('Summary must not call the network'))),
  );
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <Summary store={store} />, host);
}

describe('Summary pricingHidden (ticket #5319)', () => {
  // §8 Row 1: pricingHidden:false => no pricing-hidden row
  it('row 1: pricingHidden:false shows no pricing-hidden row', () => {
    const { store } = stubStore(makeQuote(false));
    mount(store);
    expect(host.querySelector('[data-row="pricing-hidden"]')).toBeNull();
  });

  // §8 Row 2: pricingHidden:true, reason STALE => warning row with the STALE reason text
  it('row 2: pricingHidden:true, reason STALE shows warning row with STALE reason', () => {
    const { store } = stubStore(makeQuote(true, 'STALE'));
    mount(store);
    const row = host.querySelector('[data-row="pricing-hidden"]');
    expect(row).not.toBeNull();
    expect(row?.getAttribute('role')).toBe('status');
    expect(row?.textContent).toContain('ราคานี้ไม่แสดงบนกระดานเรท — ตรวจราคากับผู้จัดการก่อนยืนยัน');
    expect(row?.textContent).toContain('ราคาอ้างอิงเก่าเกินกำหนด');
  });

  // §8 Row 3: pricingHidden:true, reason BLOCKED => warning row with the BLOCKED reason text
  it('row 3: pricingHidden:true, reason BLOCKED shows warning row with BLOCKED reason', () => {
    const { store } = stubStore(makeQuote(true, 'BLOCKED'));
    mount(store);
    const row = host.querySelector('[data-row="pricing-hidden"]');
    expect(row).not.toBeNull();
    expect(row?.getAttribute('role')).toBe('status');
    expect(row?.textContent).toContain('ราคานี้ไม่แสดงบนกระดานเรท — ตรวจราคากับผู้จัดการก่อนยืนยัน');
    expect(row?.textContent).toContain('ชุดราคาสาขาหลักถูกระงับ');
  });

  // §8 Row 4: pricingHidden:true, reason null => warning row, no reason text
  it('row 4: pricingHidden:true, reason null shows warning row with no reason text', () => {
    const { store } = stubStore(makeQuote(true, null));
    mount(store);
    const row = host.querySelector('[data-row="pricing-hidden"]');
    expect(row).not.toBeNull();
    expect(row?.getAttribute('role')).toBe('status');
    expect(row?.textContent).toContain('ราคานี้ไม่แสดงบนกระดานเรท — ตรวจราคากับผู้จัดการก่อนยืนยัน');
    expect(row?.textContent).not.toContain('ราคาอ้างอิงเก่าเกินกำหนด');
    expect(row?.textContent).not.toContain('ชุดราคาสาขาหลักถูกระงับ');
    expect(row?.textContent).not.toContain('ชุดราคาไม่ตรงกับสาขาหลัก');
  });

  // §8 Row 5: a re-quote turns true -> false => the row disappears
  it('row 5: re-quote from true to false makes the row disappear', () => {
    const { store, setQuote } = stubStore(makeQuote(true, 'STALE'));
    mount(store);
    expect(host.querySelector('[data-row="pricing-hidden"]')).not.toBeNull();

    setQuote(makeQuote(false));
    expect(host.querySelector('[data-row="pricing-hidden"]')).toBeNull();
  });

  // §7 Reason texts: MISMATCH
  it('pricingHidden:true, reason MISMATCH shows warning row with MISMATCH reason', () => {
    const { store } = stubStore(makeQuote(true, 'MISMATCH'));
    mount(store);
    const row = host.querySelector('[data-row="pricing-hidden"]');
    expect(row).not.toBeNull();
    expect(row?.textContent).toContain('ราคานี้ไม่แสดงบนกระดานเรท — ตรวจราคากับผู้จัดการก่อนยืนยัน');
    expect(row?.textContent).toContain('ชุดราคาไม่ตรงกับสาขาหลัก');
  });

  // §9 Edge case: No quote yet (quote() undefined): no row, no error
  it('edge case §9: no quote yet (quote() undefined) shows no row and no error', () => {
    const { store } = stubStore(undefined);
    mount(store);
    expect(host.querySelector('[data-row="pricing-hidden"]')).toBeNull();
  });

  // English locale test
  it('en: pricingHidden warning and reason rendered in English when locale is en', () => {
    setLang('en');
    const { store } = stubStore(makeQuote(true, 'STALE'));
    mount(store);
    const row = host.querySelector('[data-row="pricing-hidden"]');
    expect(row).not.toBeNull();
    expect(row?.textContent).toContain('This price is hidden on the rate board — check it with a manager before confirming');
    expect(row?.textContent).toContain('the reference price is stale');
  });
});
