import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  ApiError,
  type ApiClient,
  type ViewAnswer,
  type ViewParams,
} from '../../shared/api/client';
import {
  makeKbankComparison,
  makeKbankReference,
  makeKioskCountry,
  makeKioskCurrencies,
  makeKioskCurrency,
  makeRateFreshnessDto,
  makeViewAnswer,
  type KioskCurrencies,
  type KioskCurrency,
} from '../../shared/testing/fixtures';
import { errorText, fill, setLang } from '../../shared/i18n';
import { en, th } from './i18n';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

const PUBLISHED = '2026-10-02T22:16:01+07:00';
const SOURCE = 'https://www.kasikornbank.com/en/rate/Pages/Foreign-Exchange.aspx';
const OWNER_OR_RATES = 'Owner or Rates access required.';

function cmp(
  label: string,
  bankBuy: string | null,
  shopBuy: string | null,
  buyDifference: string | null,
  bankSell: string | null,
  shopSell: string | null,
  sellDifference: string | null,
) {
  return makeKbankComparison({ label, bankBuy, shopBuy, buyDifference, bankSell, shopSell, sellDifference });
}

const AFN = makeKioskCurrency({
  code: 'AFN',
  name: 'อัฟกานิอัฟกานิสถาน',
  nameEn: 'Afghan Afghani',
  countries: [makeKioskCountry({ code: 'AF', en: 'Afghanistan', th: 'อัฟกานิสถาน' })],
  buy: null,
  sell: null,
  unitSize: '1',
  updated: '',
  partnerName: '',
  partnerCost: null,
  enabled: false,
  kbank: [cmp('AFN', null, null, null, null, null, null)],
});

const JPY = makeKioskCurrency({
  code: 'JPY',
  name: 'เยนญี่ปุ่น',
  nameEn: 'Japanese Yen',
  countries: [makeKioskCountry({ code: 'JP', en: 'Japan', th: 'ญี่ปุ่น' })],
  buy: '23.50000000',
  sell: '24.10000000',
  unitSize: '100',
  updated: '2026-10-09T05:00:01.123Z',
  partnerName: '',
  partnerCost: null,
  enabled: false,
  kbank: [cmp('JPY', '0.20715', '0.23500', '0.02785', '0.21844', '0.24100', '0.02256')],
});

const USD = makeKioskCurrency({
  code: 'USD',
  name: 'ดอลลาร์สหรัฐ',
  nameEn: 'US Dollar',
  countries: [makeKioskCountry({ code: 'EC', en: 'Ecuador', th: 'เอกวาดอร์' })],
  buy: '32.10000000',
  sell: '33.00000000',
  unitSize: '1',
  updated: '2026-10-09T05:00:01.123Z',
  partnerName: 'SuperRich',
  partnerCost: '31.90000000',
  enabled: true,
  kbank: [cmp('USD 1', '32.37919', '32.10000', '-0.27919', '33.81656', '33.00000', '-0.81656')],
});

const CURRENCIES = [AFN, JPY, USD];

function answer(
  currencies?: KioskCurrency[],
  version = 7,
) {
  return makeViewAnswer(
    makeKioskCurrencies({
      currencies: currencies ?? CURRENCIES,
      kbank: makeKbankReference({ source: SOURCE, publishedAt: PUBLISHED, round: 10 }),
      freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
    }),
    {
      version,
      changed: true,
    },
  );
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function fakeDeferred() {
  const d = deferred<ViewAnswer<KioskCurrencies>>();
  const views: Array<{ name: string; q?: ViewParams }> = [];
  const api: Pick<ApiClient, 'view' | 'post'> = {
    view<T>(name: string, q?: ViewParams): Promise<ViewAnswer<T>> {
      views.push({ name, q: q === undefined ? undefined : { ...q } });
      return d.promise as unknown as Promise<ViewAnswer<T>>;
    },
    post<B, R>(_path: string, _body: B): Promise<R> {
      return Promise.resolve({ saved: true } as unknown as R);
    },
  };
  return { api, views, deferred: d };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn(() => Promise.reject(new Error('offline in tests'))),
  );
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

function mount(api: Pick<ApiClient, 'view' | 'post'>, branch = 'B9'): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <KioskCurrenciesScreen api={api} branch={branch} />, host);
}

function textOf(el: Node | null | undefined): string {
  return (el?.textContent ?? '').trim();
}

function pageStatuses(): string[] {
  return [...host.querySelectorAll('[role="status"]')]
    .filter((el) => !el.closest('form'))
    .map(textOf);
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('KioskCurrenciesScreen loading state (#2252)', () => {
  it('row 1: before the first read answers, shows no "Found 0" count line while loading', () => {
    setLang('en');
    const f = fakeDeferred();
    mount(f.api, 'B9');

    expect(f.views).toHaveLength(1);
    expect(pageStatuses()).not.toContain(fill(en.found, { n: 0 }));
    expect(pageStatuses()).toEqual(['']);

    setLang('th');
    expect(pageStatuses()).not.toContain(fill(th.found, { n: 0 }));
    expect(pageStatuses()).toEqual(['']);
  });

  it('row 2: after waiting with no "Found 0", read answering with 3 currencies shows "Found 3"', async () => {
    const f = fakeDeferred();
    mount(f.api, 'B9');

    expect(pageStatuses()).not.toContain(fill(th.found, { n: 0 }));
    expect(pageStatuses()).toEqual(['']);

    f.deferred.resolve(answer(CURRENCIES));
    await vi.waitFor(() => {
      expect(pageStatuses()).toContain(fill(th.found, { n: 3 }));
    });
  });

  it('row 3: after waiting with no "Found 0", a failed read shows the load error', async () => {
    const f = fakeDeferred();
    mount(f.api, 'B9');

    expect(pageStatuses()).not.toContain(fill(th.found, { n: 0 }));
    expect(pageStatuses()).toEqual(['']);

    f.deferred.reject(
      new ApiError({ code: 'kioskOwnerOrRates', message: OWNER_OR_RATES, status: 400 }),
    );
    const expected = errorText('kioskOwnerOrRates', OWNER_OR_RATES);
    await vi.waitFor(() => {
      expect(pageStatuses()).toContain(expected);
    });
  });

  it('row 4: after waiting with no "Found 0" and resolving, search filtering to 1 currency shows "Found 1"', async () => {
    const f = fakeDeferred();
    mount(f.api, 'B9');

    expect(pageStatuses()).not.toContain(fill(th.found, { n: 0 }));
    expect(pageStatuses()).toEqual(['']);

    f.deferred.resolve(answer(CURRENCIES));
    await vi.waitFor(() => {
      expect(pageStatuses()).toContain(fill(th.found, { n: 3 }));
    });

    const search = host.querySelector<HTMLInputElement>('input[type="search"]')!;
    type(search, 'japan');
    await vi.waitFor(() => {
      expect(pageStatuses()).toContain(fill(th.found, { n: 1 }));
    });
  });
});
