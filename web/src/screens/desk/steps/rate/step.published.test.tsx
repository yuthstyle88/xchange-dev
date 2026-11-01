// FE-T2 #5849: the desk offers only currencies with a published rate for the chosen side
// (design docs/design/fe-triage-1101.md §1 row FE-T2, §2 PublishedRates/RateStep/DeskStore).
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests:
// - `web/src/screens/desk/published-rates.ts` exports `publishedCodes(rates, type)`. BUY reads `DeskRate.buy`, SELL reads
//   `DeskRate.sell` (the quote sends `shopSide: type()`); a code is published for a side when that field is not null.
// - The rate step's currency select lists only the published codes for the current type. A currency that is not
//   published stays in the list only while a desk entry (kiosk/booking) holds it (today's "keep current" rule).
// - The store's default currency is the first published code for the type. Switching type without an entry moves an
//   unpublished currency to the first published code of the new side, or to '' when there is none.
// - No published code for the side: no options, and a `.desk-notice[data-desk-notice="no-published-rate"]` with
//   `noPublishedRate`. A quote refused with 409 `rateUnavailable` is worded with `rateNotPublished` (code + side),
//   never the shared "try later" text.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { OWNER_DESK, STAFF_DESK, type DeskPath } from '../../desk-path';
import { createDeskStore, type DeskStore } from '../../desk-store';
import { step } from './step';

type PublishedModule = { publishedCodes: (rates: unknown[], type: 'BUY' | 'SELL') => string[] };
const publishedMods = import.meta.glob<PublishedModule>('../../published-rates.ts');

const ME = () => ({ id: 'u-1', name: 'Ploy', role: 'TELLER', branch: 'MAIN', permissions: [] });

// The exact texts this ticket adds to steps/rate/i18n.ts.
const NO_PUBLISHED_TH = (side: string) => `สาขานี้ยังไม่มีเรทประกาศสำหรับรายการ${side}`;
const NO_PUBLISHED_EN = (side: string) => `No rate is published at this branch for ${side}.`;
const NOT_PUBLISHED_TH = (code: string, side: string) => `สาขานี้ไม่มีเรทประกาศของ ${code} สำหรับรายการ${side}`;
const BUY_TH = 'ร้านรับซื้อ (BUY)';
const SELL_TH = 'ร้านขาย (SELL)';
const SELL_EN = 'Shop sells (SELL)';
const TRY_LATER = /ภายหลัง|later/i;

const rate = (code: string, buy: string | null, sell: string | null) => ({
  code,
  unitSize: '1',
  buy,
  sell,
  publication: buy === null && sell === null ? null : 'pub-1',
});

// EUR: neither side · GBP: SELL only · JPY: BUY only · USD: both sides.
const MIXED = [rate('EUR', null, null), rate('GBP', null, '45.10'), rate('JPY', '0.21', null), rate('USD', '32.10', '32.90')];

function config(rates: unknown[]) {
  return {
    branchId: 'MAIN',
    branchName: 'XC-B',
    shiftOpen: true,
    adminUrl: '/?branch=MAIN',
    rates,
    paymentMethods: [],
  };
}

type QuoteReply = { status: number; body: unknown };

function fakeApi(rates: unknown[], quoteReply?: () => QuoteReply) {
  const quoteBodies: unknown[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input).split('?')[0];
    const json = (status: number, body: unknown) =>
      new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
    if (path === `/api/v3/views/${OWNER_DESK.configView}` || path === `/api/v3/views/${STAFF_DESK.configView}`) {
      return json(200, { version: 1, changed: true, data: config(rates) });
    }
    if (path === '/api/quotes' && String(init?.method).toUpperCase() === 'POST') {
      quoteBodies.push(JSON.parse(String(init?.body)));
      const a = quoteReply?.() ?? { status: 500, body: { error: { code: 'serverError', message: 'x' } } };
      return json(a.status, a.body);
    }
    return json(404, { error: { code: 'notFound', message: 'nf' } });
  });
  return { api: new ApiClient(fetchFn as unknown as typeof fetch), quoteBodies };
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

async function mountRateStep(path: DeskPath, api: ApiClient): Promise<DeskStore> {
  let store!: DeskStore;
  dispose = render(() => {
    store = createDeskStore({ api, path, me: ME as any });
    return step.component({ store });
  }, host);
  await vi.waitFor(() => expect(store.config()).toBeDefined());
  return store;
}

const currencySelect = () => host.querySelector<HTMLSelectElement>('select[name="currency"]')!;
const options = () => [...currencySelect().options].map((o) => o.value);
const noPublishedNotice = () => host.querySelector<HTMLElement>('.desk-notice[data-desk-notice="no-published-rate"]');

describe('FE-T2 publishedCodes (published-rates.ts)', () => {
  async function publishedCodes() {
    const l = publishedMods['../../published-rates.ts'];
    expect(l, 'web/src/screens/desk/published-rates.ts exists').toBeDefined();
    return (await l!()).publishedCodes;
  }

  it('P1 BUY with USD{buy,sell} and EUR{null,null} gives [USD]', async () => {
    const f = await publishedCodes();
    expect(f([rate('USD', '32.10', '32.90'), rate('EUR', null, null)], 'BUY')).toEqual(['USD']);
  });

  it('P2 SELL does not offer a code published on the buy side only; order follows the config', async () => {
    const f = await publishedCodes();
    expect(f([rate('X', '1.00', null)], 'SELL')).toEqual([]);
    expect(f(MIXED, 'SELL')).toEqual(['GBP', 'USD']);
    expect(f(MIXED, 'BUY')).toEqual(['JPY', 'USD']);
  });
});

for (const [which, path] of [
  ['OWNER_DESK', OWNER_DESK],
  ['STAFF_DESK', STAFF_DESK],
] as const) {
  describe(`FE-T2 desk currency list on ${which}`, () => {
    it('P3 the BUY list holds only codes with a published buy rate', async () => {
      const { api } = fakeApi(MIXED);
      await mountRateStep(path, api);
      await vi.waitFor(() => expect(options()).toEqual(['JPY', 'USD']));
      expect(noPublishedNotice()).toBeNull();
    });

    it('P4 the default currency is the first published code for the type, not rates[0]', async () => {
      const { api } = fakeApi(MIXED);
      const store = await mountRateStep(path, api);
      await vi.waitFor(() => expect(store.currency()).toBe('JPY'));
      expect(currencySelect().value).toBe('JPY');
    });

    it('P5 switching to SELL re-filters the list and moves a BUY-only currency to the first SELL code', async () => {
      const { api } = fakeApi(MIXED);
      const store = await mountRateStep(path, api);
      await vi.waitFor(() => expect(store.currency()).toBe('JPY'));
      store.setType('SELL');
      await vi.waitFor(() => expect(options()).toEqual(['GBP', 'USD']));
      expect(store.currency()).toBe('GBP');
      expect(currencySelect().value).toBe('GBP');
    });

    it('P6 switching type keeps a currency that is published on both sides', async () => {
      const { api } = fakeApi(MIXED);
      const store = await mountRateStep(path, api);
      store.setCurrency('USD');
      store.setType('SELL');
      await vi.waitFor(() => expect(options()).toEqual(['GBP', 'USD']));
      expect(store.currency()).toBe('USD');
    });

    it('P7 a currency brought in by a desk entry stays listed and selected even when not published', async () => {
      const { api } = fakeApi(MIXED);
      const store = await mountRateStep(path, api);
      store.setEntry({ kind: 'kiosk', id: 'b-1', locked: true } as any);
      store.setCurrency('CHF');
      store.setType('SELL');
      await vi.waitFor(() => expect(options()).toEqual(['CHF', 'GBP', 'USD']));
      expect(store.currency()).toBe('CHF');
    });

    it('P8 no published code for the side: no options, no currency, and the noPublishedRate notice', async () => {
      const { api, quoteBodies } = fakeApi([rate('EUR', null, null), rate('JPY', '0.21', null)]);
      const store = await mountRateStep(path, api);
      store.setType('SELL');
      await vi.waitFor(() => expect(noPublishedNotice()?.textContent).toBe(NO_PUBLISHED_TH(SELL_TH)));
      expect(options()).toEqual([]);
      expect(store.currency()).toBe('');
      expect(noPublishedNotice()!.textContent).not.toMatch(TRY_LATER);
      store.setAmount('100');
      await store.requote();
      expect(quoteBodies).toHaveLength(0);
    });

    it('P9 the noPublishedRate notice follows the language', async () => {
      const { api } = fakeApi([rate('EUR', null, null)]);
      const store = await mountRateStep(path, api);
      store.setType('SELL');
      setLang('en');
      await vi.waitFor(() => expect(noPublishedNotice()?.textContent).toBe(NO_PUBLISHED_EN(SELL_EN)));
    });

    it('P10 a quote refused with 409 rateUnavailable names the code and side, without "try later"', async () => {
      const { api, quoteBodies } = fakeApi([rate('USD', '32.10', '32.90')], () => ({
        status: 409,
        body: { error: { code: 'rateUnavailable', message: 'rateUnavailable: no published line' } },
      }));
      const store = await mountRateStep(path, api);
      await vi.waitFor(() => expect(store.currency()).toBe('USD'));
      store.setAmount('100');
      await store.requote();
      expect(quoteBodies).toHaveLength(1);
      const expected = NOT_PUBLISHED_TH('USD', BUY_TH);
      expect(store.quoteError()?.message).toBe(expected);
      await vi.waitFor(() => expect(host.textContent).toContain(expected));
      expect(host.textContent).not.toMatch(TRY_LATER);
    });

    it('P11 New after a SELL sale returns to BUY and to the first published BUY code', async () => {
      const { api } = fakeApi(MIXED);
      const store = await mountRateStep(path, api);
      store.setType('SELL');
      await vi.waitFor(() => expect(store.currency()).toBe('GBP'));
      expect(store.startNew()).toBe(true);
      await vi.waitFor(() => expect(options()).toEqual(['JPY', 'USD']));
      expect(store.type()).toBe('BUY');
      expect(store.currency()).toBe('JPY');
    });
  });
}
