// Ticket #5203 (V.9 PV-m): kiosk currencies mark currencies customers cannot quote as "contact staff".
// Behaviour table:
// K1: currency quoteEnabled: true -> no [data-quote="off"]
// K2: quoteEnabled: false, enabled: true -> the note is shown with the TH text
// K3: quoteEnabled: false, enabled: false -> the card is listed with the note
// K4: EN -> the EN text
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { ApiClient, ViewAnswer } from '../../shared/api/client';
import {
  makeKioskCurrencies,
  makeKioskCurrency,
  makeRateFreshnessDto,
  makeViewAnswer,
} from '../../shared/testing/fixtures';
import { setLang } from '../../shared/i18n';
import { en, th } from './i18n';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

function currency(code: string, quoteEnabled: boolean, enabled: boolean) {
  return makeKioskCurrency({
    code,
    name: `${code} name`,
    nameEn: `${code} english`,
    countries: [],
    buy: '30.0000',
    sell: '31.0000',
    unitSize: '1',
    updated: '2026-10-27T00:00:00.000Z',
    partnerName: '',
    partnerCost: null,
    enabled,
    quoteEnabled,
    kbank: [],
  });
}

function fakeApi(currencies: ReturnType<typeof currency>[]) {
  const answer = makeViewAnswer(
    makeKioskCurrencies({
      currencies,
      kbank: null,
      freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
    }),
    { version: 1, changed: true },
  );
  const api: Pick<ApiClient, 'view' | 'post'> = {
    view<T>(): Promise<ViewAnswer<T>> {
      return Promise.resolve(structuredClone(answer) as any);
    },
    post: () => Promise.resolve({ saved: true } as any),
  };
  return api;
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('offline in tests'))));
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

function mount(api: Pick<ApiClient, 'view' | 'post'>) {
  dispose = render(() => <KioskCurrenciesScreen api={api} branch="B1" />, host);
}

describe('KioskCurrenciesScreen quoteEnabled (ticket #5203)', () => {
  it('K1: currency quoteEnabled: true -> no [data-quote="off"]', async () => {
    const usd = currency('USD', true, true);
    mount(fakeApi([usd]));

    await Promise.resolve();

    expect(host.querySelector('h3')?.textContent).toContain('USD');
    expect(host.querySelector('[data-quote="off"]')).toBeNull();
  });

  it('K2: quoteEnabled: false, enabled: true -> the note is shown with the TH text', async () => {
    const jpy = currency('JPY', false, true);
    mount(fakeApi([jpy]));

    await Promise.resolve();

    const note = host.querySelector<HTMLElement>('p.kiosk-quote-off[data-quote="off"]');
    expect(note).not.toBeNull();
    expect(note?.textContent?.trim()).toBe('ติดต่อพนักงาน — ลูกค้าขอราคาสกุลนี้ที่ตู้ไม่ได้');
    expect(note?.textContent?.trim()).toBe(th.contactStaff);
  });

  it('K3: quoteEnabled: false, enabled: false -> the card is listed with the note', async () => {
    const afn = currency('AFN', false, false);
    mount(fakeApi([afn]));

    await Promise.resolve();

    expect(host.querySelector('h3')?.textContent).toContain('AFN');
    const showRadio = host.querySelector<HTMLInputElement>('input[type="radio"][value="SHOW"]');
    expect(showRadio?.checked).toBe(false);
    const hideRadio = host.querySelector<HTMLInputElement>('input[type="radio"][value="HIDE"]');
    expect(hideRadio?.checked).toBe(true);

    const note = host.querySelector<HTMLElement>('p.kiosk-quote-off[data-quote="off"]');
    expect(note).not.toBeNull();
    expect(note?.textContent?.trim()).toBe('ติดต่อพนักงาน — ลูกค้าขอราคาสกุลนี้ที่ตู้ไม่ได้');
    expect(note?.textContent?.trim()).toBe(th.contactStaff);
  });

  it('K4: EN -> the EN text', async () => {
    setLang('en');
    const eur = currency('EUR', false, true);
    mount(fakeApi([eur]));

    await Promise.resolve();

    const note = host.querySelector<HTMLElement>('p.kiosk-quote-off[data-quote="off"]');
    expect(note).not.toBeNull();
    expect(note?.textContent?.trim()).toBe("Contact staff: customers can't get a quote for this currency at the kiosk");
    expect(note?.textContent?.trim()).toBe(en.contactStaff);
  });

  it('only cards with quoteEnabled: false show the note when mixed', async () => {
    const on = currency('USD', true, true);
    const off = currency('EUR', false, true);
    mount(fakeApi([on, off]));

    await Promise.resolve();

    const cards = host.querySelectorAll('form.quote');
    expect(cards.length).toBe(2);

    const onCard = [...cards].find((c) => c.querySelector('h3')?.textContent?.includes('USD'));
    const offCard = [...cards].find((c) => c.querySelector('h3')?.textContent?.includes('EUR'));

    expect(onCard?.querySelector('[data-quote="off"]')).toBeNull();
    expect(offCard?.querySelector('[data-quote="off"]')).not.toBeNull();
  });
});
