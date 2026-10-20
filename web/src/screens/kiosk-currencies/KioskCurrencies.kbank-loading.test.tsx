import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
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
  type KbankReference,
  type KioskCurrencies,
  type KioskCurrency,
} from '../../shared/testing/fixtures';
import { errorText, setLang } from '../../shared/i18n';
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

const KBANK_REF = makeKbankReference({
  source: SOURCE,
  publishedAt: PUBLISHED,
  round: 10,
});

function answer(
  currencies?: KioskCurrency[],
  kbank?: KbankReference | null,
  version = 7,
) {
  return makeViewAnswer(
    makeKioskCurrencies({
      currencies: currencies ?? CURRENCIES,
      kbank: kbank === undefined ? KBANK_REF : kbank,
      freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
    }),
    {
      version,
      changed: true,
    },
  );
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (v: T) => void;
  reject: (e: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function fakeQueue() {
  const calls: Array<{ name: string; q?: ViewParams; deferred: Deferred<ViewAnswer<KioskCurrencies>> }> = [];
  const api: Pick<ApiClient, 'view' | 'post'> = {
    view<T>(name: string, q?: ViewParams): Promise<ViewAnswer<T>> {
      const d = deferred<ViewAnswer<KioskCurrencies>>();
      calls.push({ name, q: q === undefined ? undefined : { ...q }, deferred: d });
      return d.promise as unknown as Promise<ViewAnswer<T>>;
    },
    post<B, R>(_path: string, _body: B): Promise<R> {
      return Promise.resolve({ saved: true } as unknown as R);
    },
  };
  return { api, calls };
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

function mount(api: Pick<ApiClient, 'view' | 'post'>, initialBranch = 'B9') {
  const [branch, setBranch] = createSignal(initialBranch);
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <KioskCurrenciesScreen api={api} branch={branch()} />, host);
  return { branch, setBranch };
}

function textOf(el: Node | null | undefined): string {
  return (el?.textContent ?? '').trim();
}

function bankSection(): HTMLElement | null {
  return host.querySelector<HTMLElement>('#bank-reference');
}

function pageStatuses(): string[] {
  return [...host.querySelectorAll('[role="status"]')]
    .filter((el) => !el.closest('form'))
    .map(textOf);
}

describe('KioskCurrenciesScreen bank-reference loading state (#2279)', () => {
  it('row 1: Before the first read answers -> No kbankFailed text', () => {
    setLang('en');
    const q = fakeQueue();
    mount(q.api, 'B9');

    expect(q.calls).toHaveLength(1);
    expect(textOf(bankSection())).not.toContain(en.kbankFailed);
    expect(textOf(bankSection())).toBe('');

    setLang('th');
    expect(textOf(bankSection())).not.toContain(th.kbankFailed);
    expect(textOf(bankSection())).toBe('');
  });

  it('row 2: Read answers with a KBank reference -> The reference is shown (kept)', async () => {
    const q = fakeQueue();
    mount(q.api, 'B9');

    expect(textOf(bankSection())).not.toContain(th.kbankFailed);
    expect(textOf(bankSection())).toBe('');

    q.calls[0]!.deferred.resolve(answer(CURRENCIES, KBANK_REF));
    await vi.waitFor(() => {
      expect(textOf(bankSection())).toContain(th.compareHeading);
    });
    expect(textOf(bankSection())).toContain('USD 1');
    expect(textOf(bankSection())).not.toContain(th.kbankFailed);
  });

  it('row 3: Read fails -> kbankFailed (or the load error) is shown (kept)', async () => {
    const q = fakeQueue();
    mount(q.api, 'B9');

    expect(textOf(bankSection())).not.toContain(th.kbankFailed);
    expect(textOf(bankSection())).toBe('');

    q.calls[0]!.deferred.reject(
      new ApiError({ code: 'kioskOwnerOrRates', message: OWNER_OR_RATES, status: 400 }),
    );
    const expectedErr = errorText('kioskOwnerOrRates', OWNER_OR_RATES);
    await vi.waitFor(() => {
      expect(textOf(bankSection())).toBe(th.kbankFailed);
      expect(pageStatuses()).toContain(expectedErr);
    });
  });

  it('row 4: Switching branch while a list is shown -> No kbankFailed flash while the new read is pending', async () => {
    const q = fakeQueue();
    const { setBranch } = mount(q.api, 'B9');

    q.calls[0]!.deferred.resolve(answer(CURRENCIES, KBANK_REF));
    await vi.waitFor(() => {
      expect(textOf(bankSection())).toContain(th.compareHeading);
    });

    setBranch('B2');
    expect(q.calls).toHaveLength(2);
    expect(textOf(bankSection())).not.toContain(th.kbankFailed);
    expect(textOf(bankSection())).toBe('');

    q.calls[1]!.deferred.resolve(answer(CURRENCIES, KBANK_REF));
    await vi.waitFor(() => {
      expect(textOf(bankSection())).toContain(th.compareHeading);
    });
  });

  it('edge case: Answer with data but no KBank reference -> shows kbankFailed once the read has answered', async () => {
    const q = fakeQueue();
    mount(q.api, 'B9');

    expect(textOf(bankSection())).not.toContain(th.kbankFailed);
    expect(textOf(bankSection())).toBe('');

    q.calls[0]!.deferred.resolve(answer(CURRENCIES, null));
    await vi.waitFor(() => {
      expect(textOf(bankSection())).toBe(th.kbankFailed);
    });
    expect(host.querySelectorAll('form.quote')).toHaveLength(3);
  });
});
