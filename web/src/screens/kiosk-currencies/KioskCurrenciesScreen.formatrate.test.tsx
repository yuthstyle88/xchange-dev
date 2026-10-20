// Ticket #603 (RE-6): the kiosk-currencies screen hands each published side to the one rate
// formatter with the view's own unit.
// Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/blueprint/rates.md` §3.3 (one web formatter, `formatRate` in
// `web/src/shared/ui/rate.ts`, RE-5 #602; no screen formats a rate itself), §4.8 (the view reads
// the current publication), §4.9 R1 (v3 views speak per unit), §9 RE-6; ♻️ (a second formatter
// would be a per-variant copy).
//
// `formatRate` is replaced here by a stand-in that prints its three arguments, so what the card
// shows proves what it was given: the side's 8-place string as the view sent it, the view's
// `unitSize` string (`"10"` for a CNY line stored per 10, which no register gives), and the code.
// A null side is never handed to it, and neither is a KBank figure (those are the server's strings).
// `KioskCurrenciesScreen.publication.test.tsx` pins the real formatter's output on the same screen.
//
// Fields and names assumed: `formatRate(rate: string, unitSize: number | string, code: string)`, a
// named export of `../../shared/ui/rate`, imported by the screen; `KioskCurrency.unitSize: string`;
// the screen's table keeps `th.buy` / `th.sell` (read through the table).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { ApiClient } from '../../shared/api/client';
import {
  makeKbankComparison,
  makeKbankReference,
  makeKioskCountry,
  makeKioskCurrencies,
  makeKioskCurrency,
  makeRateFreshnessDto,
  makeViewAnswer,
  type ViewAnswer,
} from '../../shared/testing/fixtures';
import { setLang } from '../../shared/i18n';
import { formatRate } from '../../shared/ui/rate';
import { th } from './i18n';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

vi.mock('../../shared/ui/rate', () => ({
  formatRate: vi.fn((rate: string, unitSize: number | string, code: string) => `fmt[${rate}|${unitSize}|${code}]`),
}));

const CNY = makeKioskCurrency({
  code: 'CNY',
  name: 'หยวนจีน',
  nameEn: 'Chinese Yuan',
  countries: [makeKioskCountry({ code: 'CN', en: 'China', th: 'จีน' })],
  buy: '45.67800000',
  sell: null,
  unitSize: '10',
  updated: '2026-10-09T05:00:01.123Z',
  partnerName: '',
  partnerCost: null,
  enabled: true,
  kbank: [
    makeKbankComparison({ label: 'CNY', bankBuy: '4.41000', shopBuy: '4.56780', buyDifference: '0.15780', bankSell: null, shopSell: null, sellDifference: null }),
  ],
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
  kbank: [],
});

function answer() {
  return makeViewAnswer(
    makeKioskCurrencies({
      currencies: [CNY, JPY],
      kbank: makeKbankReference({ source: 'https://www.kasikornbank.com/', publishedAt: '2026-10-02T22:16:01+07:00', round: 10 }),
      // RF-3 (rates.md §4.10): the view's answer gains `freshness`; re-expected by the RF-3 ticket author.
      freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
    }),
    {
      version: 1,
      changed: true,
    },
  );
}

const api: Pick<ApiClient, 'view' | 'post'> = {
  view<T>(): Promise<ViewAnswer<T>> {
    return Promise.resolve(structuredClone(answer()) as any);
  },
  post<B, R>(_path: string, _body: B): Promise<R> {
    return Promise.resolve({ saved: true } as unknown as R);
  },
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  vi.mocked(formatRate).mockClear();
  vi.stubGlobal(
    'fetch',
    vi.fn(() => Promise.reject(new Error('offline in tests'))),
  );
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <KioskCurrenciesScreen api={api} branch="B9" />, host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
});

function textOf(el: Node | null | undefined): string {
  return (el?.textContent ?? '').trim();
}

/** The nearest ancestor of the element showing `text` that names one side label only. */
function sideOf(text: string): 'buy' | 'sell' | 'none' | 'absent' {
  const walker = document.createTreeWalker(host, NodeFilter.SHOW_TEXT);
  let at: HTMLElement | null = null;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if ((n.nodeValue ?? '').includes(text)) {
      at = n.parentElement;
      break;
    }
  }
  if (!at) return 'absent';
  for (let a: HTMLElement | null = at; a && a !== host; a = a.parentElement) {
    const t = textOf(a);
    const buy = t.includes(th.buy);
    const sell = t.includes(th.sell);
    if (buy && !sell) return 'buy';
    if (sell && !buy) return 'sell';
    if (buy && sell) return 'none';
  }
  return 'none';
}

describe('#603 RE-6: the screen shows what formatRate makes of the view (§3.3, ♻️)', () => {
  it("each published side is formatRate(side, the view's unitSize, code), shown on its side", async () => {
    await vi.waitFor(() => expect(sideOf('fmt[45.67800000|10|CNY]')).toBe('buy'));
    expect(sideOf('fmt[23.50000000|100|JPY]')).toBe('buy');
    expect(sideOf('fmt[24.10000000|100|JPY]')).toBe('sell');
    expect(vi.mocked(formatRate).mock.calls).toContainEqual(['45.67800000', '10', 'CNY']);
    expect(vi.mocked(formatRate).mock.calls).toContainEqual(['23.50000000', '100', 'JPY']);
    expect(vi.mocked(formatRate).mock.calls).toContainEqual(['24.10000000', '100', 'JPY']);
  });

  it('a null side and the KBank figures never reach formatRate', async () => {
    await vi.waitFor(() => expect(sideOf('fmt[45.67800000|10|CNY]')).toBe('buy'));
    const given = vi.mocked(formatRate).mock.calls.map(([rate]) => rate);
    expect(given.every((r) => typeof r === 'string' && r !== '')).toBe(true);
    expect(given.filter((r) => !['45.67800000', '23.50000000', '24.10000000'].includes(r))).toEqual([]);
  });
});
