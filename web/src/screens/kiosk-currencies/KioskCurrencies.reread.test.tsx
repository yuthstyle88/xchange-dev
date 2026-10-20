import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewAnswer, type ViewParams } from '../../shared/api/client';
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
import { fill, setLang } from '../../shared/i18n';
import { th as sharedTh } from '../../shared/i18n/th';
import { th } from './i18n';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

const PUBLISHED = '2026-10-02T22:16:01+07:00';
const SOURCE = 'https://www.kasikornbank.com/en/rate/Pages/Foreign-Exchange.aspx';
const RATES_FIRST = 'Set valid counter buy and sell rates first.';

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

interface Fake {
  api: Pick<ApiClient, 'view' | 'post'>;
  views: Array<{ name: string; q?: ViewParams }>;
  posts: Array<{ path: string; body: unknown }>;
}

function fake(
  answers: Array<ViewAnswer<KioskCurrencies> | Error>,
  onPost: (path: string, body: unknown, index: number) => Promise<unknown> = async () => ({ saved: true }),
): Fake {
  const views: Fake['views'] = [];
  const posts: Fake['posts'] = [];
  const api: Pick<ApiClient, 'view' | 'post'> = {
    view<T>(name: string, q?: ViewParams): Promise<ViewAnswer<T>> {
      views.push({ name, q: q === undefined ? undefined : { ...q } });
      const a = answers[Math.min(views.length - 1, answers.length - 1)]!;
      if (a instanceof Error) return Promise.reject(a);
      return Promise.resolve(structuredClone(a) as any);
    },
    post<B, R>(path: string, body: B): Promise<R> {
      const idx = posts.length;
      posts.push({ path, body: JSON.parse(JSON.stringify(body)) as unknown });
      return onPost(path, body, idx) as Promise<R>;
    },
  };
  return { api, views, posts };
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

function mount(api: Fake['api'], branch = 'B9'): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <KioskCurrenciesScreen api={api} branch={branch} />, host);
}

function textOf(el: Node | null | undefined): string {
  return (el?.textContent ?? '').trim();
}

function heading(c: KioskCurrency): string {
  return `${c.code} · ${c.name} / ${c.nameEn}`;
}

function cardOf(c: KioskCurrency): HTMLElement {
  const h = [...host.querySelectorAll<HTMLElement>('h1, h2, h3, h4, h5, h6')].find(
    (x) => textOf(x) === heading(c),
  );
  if (!h) throw new Error(`no heading ${heading(c)}`);
  let el: HTMLElement | null = h.parentElement;
  while (el && el !== host && !el.querySelector('input[type="checkbox"]')) {
    el = el.parentElement;
  }
  if (!el || el === host) throw new Error(`no card with a switch around ${heading(c)}`);
  return el;
}

function switchOf(card: HTMLElement): HTMLInputElement {
  const boxes = card.querySelectorAll<HTMLInputElement>('input[type="checkbox"]');
  expect(boxes, 'one switch per card').toHaveLength(1);
  return boxes[0]!;
}

function cardStatus(card: HTMLElement): string {
  return textOf(card.querySelector('[role="status"]'));
}

function pageStatuses(): string[] {
  return [...host.querySelectorAll('[role="status"]')]
    .filter((el) => !el.closest('form'))
    .map(textOf);
}

async function loaded(n = CURRENCIES.length): Promise<void> {
  await vi.waitFor(() =>
    expect(host.querySelectorAll('input[type="checkbox"]')).toHaveLength(n),
  );
}

describe('KioskCurrenciesScreen re-read after configure (#2197)', () => {
  it('row 1: Configure OK, re-read OK shows what the view answers and clears any prior re-read error', async () => {
    const readErr = new ApiError({ code: 'serverError', message: 'Read failed', status: 500 });
    const afterOk = answer([AFN, { ...JPY, enabled: true }, { ...USD, enabled: false }], 9);
    const f = fake([answer(), readErr, afterOk]);
    mount(f.api, 'B9');
    await loaded();

    // First toggle: configure OK, re-read fails -> page message shows read error, card shows savedOn
    switchOf(cardOf(JPY)).click();
    await vi.waitFor(() => expect(pageStatuses()).toContain(sharedTh['error.serverError']));
    expect(cardStatus(cardOf(JPY))).toBe(th.savedOn);

    // Second toggle: configure OK, re-read OK -> card says what view answers, page message clears error
    switchOf(cardOf(USD)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(USD))).toBe(th.savedOff));
    expect(switchOf(cardOf(USD)).checked).toBe(false);
    expect(switchOf(cardOf(JPY)).checked).toBe(true);
    expect(pageStatuses()).toContain(fill(th.found, { n: 3 }));
    expect(pageStatuses()).not.toContain(sharedTh['error.serverError']);
  });

  it("row 2: Configure refused shows server's sentence on the card, switch goes back, nothing re-read, while preserving a prior saved switch", async () => {
    const readErr = new TypeError('Failed to fetch view');
    const f = fake([answer(), readErr], async (_path, _body, idx) => {
      if (idx === 1) {
        throw new ApiError({ code: 'refused', message: RATES_FIRST, status: 400 });
      }
      return { saved: true };
    });
    mount(f.api, 'B9');
    await loaded();

    // First toggle (JPY): configure OK, re-read fails -> switch stays on, card says savedOn
    switchOf(cardOf(JPY)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(JPY))).toBe(th.savedOn));
    expect(switchOf(cardOf(JPY)).checked).toBe(true);
    expect(f.views).toHaveLength(2);

    // Second toggle (AFN): configure refused -> card shows server sentence, switch goes back to off, no new view read
    switchOf(cardOf(AFN)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(AFN))).toBe(RATES_FIRST));
    expect(switchOf(cardOf(AFN)).checked).toBe(false);
    expect(f.views, 'a refused configure reads nothing again').toHaveLength(2);
    expect(switchOf(cardOf(JPY)).checked).toBe(true);
    expect(cardStatus(cardOf(JPY))).toBe(th.savedOn);
  });

  it('row 3: Configure OK, re-read fails shows savedOn/savedOff on card, keeps switch as ticked, and shows read error in page message', async () => {
    const readErr = new ApiError({ code: 'serverError', message: 'Read failed', status: 500 });
    const netErr = new TypeError('Failed to fetch view');
    const f = fake([answer(), readErr, netErr]);
    mount(f.api, 'B9');
    await loaded();

    // Toggle JPY on: configure OK, re-read fails with 500
    switchOf(cardOf(JPY)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(JPY))).toBe(th.savedOn));
    expect(switchOf(cardOf(JPY)).checked).toBe(true);
    expect(pageStatuses()).toContain(sharedTh['error.serverError']);

    // Toggle USD off: configure OK, re-read fails with network error
    switchOf(cardOf(USD)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(USD))).toBe(th.savedOff));
    expect(switchOf(cardOf(USD)).checked).toBe(false);
    expect(pageStatuses()).toContain(sharedTh['error.network']);
  });

  it('row 4: While sending, that switch is disabled and remains at the ticked state even if the follow-up re-read fails', async () => {
    const pending = deferred<unknown>();
    const readErr = new ApiError({ code: 'serverError', message: 'Read failed', status: 500 });
    const f = fake([answer(), readErr], () => pending.promise);
    mount(f.api, 'B9');
    await loaded();

    switchOf(cardOf(JPY)).click();
    await vi.waitFor(() => expect(switchOf(cardOf(JPY)).disabled).toBe(true));
    expect(switchOf(cardOf(USD)).disabled).toBe(false);

    pending.resolve({ saved: true });
    await vi.waitFor(() => expect(switchOf(cardOf(JPY)).disabled).toBe(false));
    expect(cardStatus(cardOf(JPY))).toBe(th.savedOn);
    expect(switchOf(cardOf(JPY)).checked).toBe(true);
    expect(pageStatuses()).toContain(sharedTh['error.serverError']);
  });

  it("edge case: the next successful re-read replaces the sent value with the server's value", async () => {
    const readErr = new ApiError({ code: 'serverError', message: 'Read failed', status: 500 });
    // On the next successful read, server reports JPY is actually false (off) and USD is false (off)
    const serverTruth = answer([AFN, { ...JPY, enabled: false }, { ...USD, enabled: false }], 9);
    const f = fake([answer(), readErr, serverTruth]);
    mount(f.api, 'B9');
    await loaded();

    // Toggle JPY on: configure OK, re-read fails -> JPY stays at sent value (true)
    switchOf(cardOf(JPY)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(JPY))).toBe(th.savedOn));
    expect(switchOf(cardOf(JPY)).checked).toBe(true);
    expect(pageStatuses()).toContain(sharedTh['error.serverError']);

    // Next successful re-read (triggered when toggling USD off) replaces JPY's optimistic true with server's false
    switchOf(cardOf(USD)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(USD))).toBe(th.savedOff));
    expect(switchOf(cardOf(JPY)).checked).toBe(false);
    expect(pageStatuses()).toContain(fill(th.found, { n: 3 }));
  });
});
