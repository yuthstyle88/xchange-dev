import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewAnswer, type ViewParams } from '../../shared/api/client';
import type {
  KbankComparison,
  KioskCurrencies,
  KioskCurrency,
} from '../../shared/api/generated/kiosk_currencies';
import { setLang } from '../../shared/i18n';
import { wordedError } from '../../shared/i18n/worded-error';
import { th } from './i18n';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

const PUBLISHED = '2026-10-02T22:16:01+07:00';
const SOURCE = 'https://www.kasikornbank.com/en/rate/Pages/Foreign-Exchange.aspx';

function cmp(label: string): KbankComparison {
  return {
    label,
    bankBuy: null,
    shopBuy: null,
    buyDifference: null,
    bankSell: null,
    shopSell: null,
    sellDifference: null,
  };
}

function currency(code: string, name: string, nameEn: string, enabled: boolean): KioskCurrency {
  return {
    code,
    name,
    nameEn,
    countries: [],
    buy: null,
    sell: null,
    unitSize: '1',
    updated: '',
    partnerName: '',
    partnerCost: null,
    enabled,
    kbank: [cmp(code)],
  };
}

// Branch A's list
const AFN = currency('AFN', 'อัฟกานิอัฟกานิสถาน', 'Afghan Afghani', false);
const JPY = currency('JPY', 'เยนญี่ปุ่น', 'Japanese Yen', false);
const USD = currency('USD', 'ดอลลาร์สหรัฐ', 'US Dollar', true);
const LIST_A = [AFN, JPY, USD];

// Branch B's list (both list USD)
const USD_B = currency('USD', 'ดอลลาร์สหรัฐ', 'US Dollar', false);
const GBP = currency('GBP', 'ปอนด์สเตอร์ลิง', 'Pound Sterling', true);
const LIST_B = [USD_B, GBP];

function answer(currencies: KioskCurrency[], round = 10): ViewAnswer<KioskCurrencies> {
  return {
    version: 1,
    changed: true,
    data: {
      currencies,
      kbank: { source: SOURCE, publishedAt: PUBLISHED, round },
      freshness: { stale: false, sourceDate: null, publishedAt: null },
    },
  };
}

type Reply = ViewAnswer<KioskCurrencies> | Error | Promise<ViewAnswer<KioskCurrencies>>;

interface Fake {
  api: Pick<ApiClient, 'view' | 'post'>;
  views: Array<{ name: string; q?: ViewParams }>;
  posts: Array<{ path: string; body: unknown }>;
}

/** Each view call i gets replies[i]; a Promise reply lets the test decide when it answers. */
function fake(replies: Reply[]): Fake {
  const views: Fake['views'] = [];
  const posts: Fake['posts'] = [];
  const api: Pick<ApiClient, 'view' | 'post'> = {
    view<T>(name: string, q?: ViewParams): Promise<ViewAnswer<T>> {
      views.push({ name, q: q === undefined ? undefined : { ...q } });
      const r = replies[views.length - 1];
      if (r === undefined) return Promise.reject(new Error(`unexpected view call #${views.length}`));
      if (r instanceof Error) return Promise.reject(r);
      return Promise.resolve(r).then((a) => structuredClone(a) as unknown as ViewAnswer<T>);
    },
    post<B, R>(path: string, body: B): Promise<R> {
      posts.push({ path, body: JSON.parse(JSON.stringify(body)) as unknown });
      return Promise.resolve({ saved: true } as R);
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

function mount(api: Fake['api'], first: string): (b: string) => void {
  const [branch, setBranch] = createSignal(first);
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <KioskCurrenciesScreen api={api} branch={branch()} />, host);
  return (b: string) => setBranch(b);
}

function textOf(el: Node | null | undefined): string {
  return (el?.textContent ?? '').trim();
}

function heading(c: KioskCurrency): string {
  return `${c.code} · ${c.name} / ${c.nameEn}`;
}

function headings(): string[] {
  return [...host.querySelectorAll<HTMLElement>('form h3')].map(textOf);
}

function cardOf(c: KioskCurrency): HTMLElement {
  const h = [...host.querySelectorAll<HTMLElement>('form h3')].find(
    (x) => textOf(x) === heading(c),
  );
  if (!h) throw new Error(`no heading ${heading(c)}`);
  return h.closest('form')!;
}

function switchOf(card: HTMLElement): HTMLInputElement {
  return card.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
}

function cardStatus(card: HTMLElement): string {
  return textOf(card.querySelector('[role="status"]'));
}

async function showing(list: KioskCurrency[]): Promise<void> {
  await vi.waitFor(() => expect(headings()).toEqual(list.map(heading)));
}

describe('KioskCurrenciesScreen branch status reset (#2662)', () => {
  it('row 1: Toggle USD on A (success, or worded error) -> USD card on A shows Saved ON/OFF or the error (unchanged)', async () => {
    // 1a: Success toggle
    const fSuccess = fake([
      answer(LIST_A),
      answer([AFN, JPY, { ...USD, enabled: false }]),
    ]);
    mount(fSuccess.api, 'A');
    await showing(LIST_A);

    switchOf(cardOf(USD)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(USD))).toBe(th.savedOff));
    expect(switchOf(cardOf(USD)).disabled).toBe(false);

    // 1b: Error toggle
    dispose?.();
    host.innerHTML = '';
    const serverErr = new ApiError({ code: 'serverError', message: 'Save failed', status: 500 });
    const fErr = fake([answer(LIST_A)]);
    fErr.api.post = () => Promise.reject(serverErr);
    mount(fErr.api, 'A');
    await showing(LIST_A);

    switchOf(cardOf(USD)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(USD))).toBe(wordedError(serverErr)));
    expect(switchOf(cardOf(USD)).disabled).toBe(false);
  });

  it('row 2: Then switch to B, which lists USD -> USD card on B shows no status text and is not busy', async () => {
    // 2a: Switch to B after successful toggle on A
    const f = fake([
      answer(LIST_A),
      answer([AFN, JPY, { ...USD, enabled: false }]),
      answer(LIST_B),
    ]);
    const setBranch = mount(f.api, 'A');
    await showing(LIST_A);

    switchOf(cardOf(USD)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(USD))).toBe(th.savedOff));

    setBranch('B');
    await showing(LIST_B);

    expect(cardStatus(cardOf(USD_B)), 'USD on B has no status text from A').toBe('');
    expect(switchOf(cardOf(USD_B)).disabled, 'USD on B is not busy').toBe(false);

    // 2b: Switch to B after error on A
    dispose?.();
    host.innerHTML = '';
    const serverErr = new ApiError({ code: 'serverError', message: 'Save failed', status: 500 });
    const fErr = fake([answer(LIST_A), answer(LIST_B)]);
    fErr.api.post = () => Promise.reject(serverErr);
    const setBranchErr = mount(fErr.api, 'A');
    await showing(LIST_A);

    switchOf(cardOf(USD)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(USD))).toBe(wordedError(serverErr)));

    setBranchErr('B');
    await showing(LIST_B);

    expect(cardStatus(cardOf(USD_B)), 'USD on B has no error from A').toBe('');
    expect(switchOf(cardOf(USD_B)).disabled, 'USD on B is not busy').toBe(false);
  });

  it('row 3: Toggle USD on A, switch to B before it answers, A answers -> No status/error appears on B\'s USD card', async () => {
    const f = fake([answer(LIST_A), answer(LIST_B)]);
    const configureHeld = deferred<void>();
    f.api.post = <B, R>(path: string, body: B): Promise<R> => {
      f.posts.push({ path, body: JSON.parse(JSON.stringify(body)) as unknown });
      return configureHeld.promise.then(() => Promise.resolve({ saved: true } as R));
    };

    const setBranch = mount(f.api, 'A');
    await showing(LIST_A);

    switchOf(cardOf(USD)).click();
    await vi.waitFor(() => expect(f.posts).toHaveLength(1));

    setBranch('B');
    await showing(LIST_B);

    // While A is still saving, USD on B must not be marked busy
    expect(switchOf(cardOf(USD_B)).disabled, 'USD on B is not busy while A save is in flight').toBe(false);

    // Now A answers
    configureHeld.resolve();
    await new Promise((r) => setTimeout(r, 20));

    expect(cardStatus(cardOf(USD_B)), 'no status appears on B USD card').toBe('');
    expect(switchOf(cardOf(USD_B)).disabled, 'USD on B remains not busy').toBe(false);
  });

  it('row 4: Switch back to A -> Clean cards', async () => {
    const f = fake([
      answer(LIST_A),
      answer([AFN, JPY, { ...USD, enabled: false }]),
      answer(LIST_B),
      answer(LIST_A),
    ]);
    const setBranch = mount(f.api, 'A');
    await showing(LIST_A);

    switchOf(cardOf(USD)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(USD))).toBe(th.savedOff));

    setBranch('B');
    await showing(LIST_B);
    expect(cardStatus(cardOf(USD_B))).toBe('');

    setBranch('A');
    await showing(LIST_A);

    expect(cardStatus(cardOf(USD)), 'USD on A is clean after switching back').toBe('');
    expect(switchOf(cardOf(USD)).disabled, 'USD on A is not busy').toBe(false);
  });
});
