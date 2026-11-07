// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm (CE, Bangkok) from the shared formatter.
// The KBank publish time reads `02/10/2026 22:16` in English as in Thai (not the page locale's `toLocaleString`), and
// the comparison's fixed "data as of" date reads `02/10/2026` in both languages. The Thai screen is pinned by the
// restated KioskCurrenciesScreen.test.tsx, KioskCurrenciesScreen.publication.test.tsx and i18n.test.tsx.
// Written before the implementation by the ticket author claude/3b81c9d5. Do not edit, delete or weaken; add tests in a
// file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { ApiClient, ViewAnswer } from '../../shared/api/client';
import {
  makeKbankComparison,
  makeKbankReference,
  makeKioskCountry,
  makeKioskCurrencies,
  makeKioskCurrency,
  makeRateFreshnessDto,
  makeViewAnswer,
} from '../../shared/testing/fixtures';
import { fill, setLang } from '../../shared/i18n';
import { en, th } from './i18n';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

const PUBLISHED = '2026-10-02T22:16:01+07:00';

const JPY = makeKioskCurrency({
  code: 'JPY',
  name: 'เยนญี่ปุ่น',
  nameEn: 'Japanese Yen',
  countries: [makeKioskCountry({ code: 'JP', en: 'Japan', th: 'ญี่ปุ่น' })],
  buy: '0.2105',
  sell: '0.2205',
  unitSize: '1',
  updated: '2026-10-06T05:00:02.123Z',
  partnerName: '',
  partnerCost: null,
  enabled: false,
  kbank: [
    makeKbankComparison({
      label: 'JPY',
      bankBuy: '0.20715',
      shopBuy: '0.21050',
      buyDifference: '0.00335',
      bankSell: '0.21844',
      shopSell: '0.22050',
      sellDifference: '0.00206',
    }),
  ],
});

const ANSWER = makeViewAnswer(
  makeKioskCurrencies({
    currencies: [JPY],
    kbank: makeKbankReference({ source: 'https://example.invalid/kbank', publishedAt: PUBLISHED, round: 10 }),
    freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
  }),
  { version: 7, changed: true },
);

const api: Pick<ApiClient, 'view' | 'post'> = {
  view<T>(): Promise<ViewAnswer<T>> {
    return Promise.resolve(structuredClone(ANSWER) as any);
  },
  post<B, R>(_path: string, _body: B): Promise<R> {
    return Promise.resolve({ saved: true } as R);
  },
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function flat(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

beforeEach(() => {
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('offline in tests'))));
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

describe('F55 kiosk currencies: one date format', () => {
  it('F55-KC1 en: the KBank stamp and the comparison line read "02/10/2026 22:16"', async () => {
    setLang('en');
    dispose = render(() => <KioskCurrenciesScreen api={api} branch="B9" />, host);
    const stamp = fill(en.kbStamp, { time: '02/10/2026 22:16', round: 10 });
    const published = fill(en.comparePublished, { time: '02/10/2026 22:16', round: 10 });
    await vi.waitFor(() => expect(flat(host.textContent)).toContain(published));
    expect(flat(host.textContent)).toContain(stamp);
  });

  it('F55-KC2 the comparison warning\'s fixed date reads "02/10/2026" in Thai and English (no "2 ต.ค. 2026", no "2 Oct 2026")', () => {
    expect(th.compareWarning).toContain('02/10/2026');
    expect(th.compareWarning).not.toContain('ต.ค.');
    expect(en.compareWarning).toContain('02/10/2026');
    expect(en.compareWarning).not.toContain('Oct');
  });
});
