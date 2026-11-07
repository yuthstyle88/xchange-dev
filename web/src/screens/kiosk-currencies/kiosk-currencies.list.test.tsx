// OL-6 wave B kiosk-currencies-list: the page lists 152 full cards (155,942 px at 430 wide). Cards that are HIDE collapse
// behind one button and sort after the rest. The view's DOM order stays (KioskCurrenciesScreen.test.tsx C6 pins it).
// Written before the implementation by the test-writer claude/94389a6b on the sub-controller claude/1cc51615's order.
// Do not edit, delete or weaken.
import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ApiClient, ViewAnswer } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import {
  makeKioskCurrencies,
  makeKioskCurrency,
  makeRateFreshnessDto,
  makeViewAnswer,
} from '../../shared/testing/fixtures';
import { en, th } from './i18n';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Display = 'SHOW' | 'ASK' | 'HIDE';
type Currency = ReturnType<typeof makeKioskCurrency>;

function currency(code: string, display: Display): Currency {
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
    enabled: display !== 'HIDE',
    display,
    quoteEnabled: display === 'SHOW',
    kbank: [],
  });
}

let list: Currency[];

const api: Pick<ApiClient, 'view' | 'post'> = {
  view<T>(): Promise<ViewAnswer<T>> {
    const answer = makeViewAnswer(
      makeKioskCurrencies({
        currencies: list,
        kbank: null,
        freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
      }),
      { version: 1, changed: true },
    );
    return Promise.resolve(structuredClone(answer) as never);
  },
  post: (_path: unknown, body: unknown) => {
    const { code, display } = (body as { payload: { code: string; display: Display } }).payload;
    list = list.map((c) => (c.code === code ? currency(code, display) : c));
    return Promise.resolve({ saved: true } as never);
  },
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

async function mount(): Promise<void> {
  dispose = render(() => <KioskCurrenciesScreen api={api} branch="B1" />, host);
  await vi.waitFor(() => expect(host.querySelector('form.quote')).not.toBeNull());
}

const cards = () => [...host.querySelectorAll<HTMLFormElement>('form.quote')];
const cardOf = (code: string) => cards().find((c) => c.querySelector('h3')?.textContent?.startsWith(code))!;
const visible = () => cards().filter((c) => !c.hidden).map((c) => c.querySelector('h3')?.textContent?.slice(0, 3));
const toggle = () => host.querySelector<HTMLButtonElement>('button.kc-off-toggle');

function search(value: string): void {
  const input = host.querySelector<HTMLInputElement>('#kiosk-search')!;
  input.value = value;
  input.dispatchEvent(new InputEvent('input', { bubbles: true }));
}

describe('kiosk-currencies list (OL-6 wave B)', () => {
  let css: string;

  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    css = fs.readFileSync(here + 'kiosk-currencies.css', 'utf8').replace(/\s+/g, ' ');
  });

  beforeEach(() => {
    setLang('th');
    vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('offline in tests'))));
    list = [currency('AFN', 'HIDE'), currency('JPY', 'ASK'), currency('NOK', 'HIDE'), currency('USD', 'SHOW')];
    host = document.createElement('div');
    document.body.append(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    setLang('th');
  });

  it('LS-1: on load only SHOW and ASK cards show; HIDE cards stay in the DOM, hidden', async () => {
    await mount();
    expect(cards()).toHaveLength(4);
    expect(visible()).toEqual(['JPY', 'USD']);
  });

  it('LS-2: every card says its display, in the view order', async () => {
    await mount();
    expect(cards().map((c) => c.dataset.display)).toEqual(['HIDE', 'ASK', 'HIDE', 'SHOW']);
  });

  it('LS-3: the button counts the cards it hides and starts collapsed', async () => {
    await mount();
    expect(toggle()?.textContent).toBe('แสดงสกุลที่ปิดอยู่ (2)');
    expect(toggle()?.getAttribute('aria-expanded')).toBe('false');
    expect(toggle()?.type).toBe('button');
  });

  it('LS-4: English follows the language', async () => {
    setLang('en');
    await mount();
    expect(toggle()?.textContent).toBe('Show disabled currencies (2)');
    toggle()!.click();
    expect(toggle()?.textContent).toBe(en.offHide);
  });

  it('LS-5: pressing it shows all cards; pressing again collapses', async () => {
    await mount();
    toggle()!.click();
    expect(visible()).toEqual(['AFN', 'JPY', 'NOK', 'USD']);
    expect(toggle()?.getAttribute('aria-expanded')).toBe('true');
    expect(toggle()?.textContent).toBe(th.offHide);
    expect(th.offHide).toBe('ซ่อนสกุลที่ปิดอยู่');
    toggle()!.click();
    expect(visible()).toEqual(['JPY', 'USD']);
  });

  it('LS-6: a search reaches hidden cards, and the button goes away while searching', async () => {
    await mount();
    search('NOK');
    expect(visible()).toEqual(['NOK']);
    expect(toggle()).toBeNull();
    search('   ');
    expect(visible()).toEqual(['JPY', 'USD']);
    expect(toggle()).not.toBeNull();
  });

  it('LS-7: the page count stays the count of all matches', async () => {
    await mount();
    expect(host.querySelector('div[role="status"]')?.textContent).toBe('พบ 4 สกุลเงิน');
  });

  it('LS-8: no HIDE card: no button', async () => {
    list = [currency('JPY', 'ASK'), currency('USD', 'SHOW')];
    await mount();
    expect(toggle()).toBeNull();
    expect(visible()).toEqual(['JPY', 'USD']);
  });

  it('LS-9: every card HIDE: all hidden, the button reachable', async () => {
    list = [currency('AFN', 'HIDE'), currency('NOK', 'HIDE')];
    await mount();
    expect(visible()).toEqual([]);
    expect(toggle()?.textContent).toBe('แสดงสกุลที่ปิดอยู่ (2)');
  });

  it('LS-10: a card just switched to HIDE stays visible with its saved message', async () => {
    await mount();
    cardOf('USD').querySelector<HTMLInputElement>('input[value="HIDE"]')!.click();
    await vi.waitFor(() => expect(cardOf('USD').querySelector('span[role="status"]')?.textContent).toBe(th.savedOff));
    expect(cardOf('USD').hidden).toBe(false);
    expect(toggle()?.textContent).toBe('แสดงสกุลที่ปิดอยู่ (3)');
  });

  it('LS-11: a hidden card switched on leaves the hidden set', async () => {
    await mount();
    toggle()!.click();
    cardOf('AFN').querySelector<HTMLInputElement>('input[value="ASK"]')!.click();
    await vi.waitFor(() => expect(cardOf('AFN').dataset.display).toBe('ASK'));
    toggle()!.click();
    expect(visible()).toEqual(['AFN', 'JPY', 'USD']);
  });

  it('LS-12: another branch starts collapsed again', async () => {
    const { createSignal } = await import('solid-js');
    const [branch, setBranch] = createSignal('B1');
    dispose = render(() => <KioskCurrenciesScreen api={api} branch={branch()} />, host);
    await vi.waitFor(() => expect(toggle()).not.toBeNull());
    toggle()!.click();
    expect(toggle()?.getAttribute('aria-expanded')).toBe('true');
    setBranch('B2');
    await vi.waitFor(() => expect(toggle()?.getAttribute('aria-expanded')).toBe('false'));
  });

  it('LS-13: HIDE cards sort after the rest by CSS order; [hidden] cannot be overridden', () => {
    expect(css).toContain('.xc-kiosk .cards .quote[data-display="HIDE"] { order: 1; }');
    expect(css).toContain('.xc-kiosk .xc-shell-main .quote[hidden] { display: none; }');
  });
});
