import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewAnswer, type ViewParams } from '../../shared/api/client';
import type {
  KbankComparison,
  KioskCurrencies,
  KioskCurrency,
} from '../../shared/api/generated/kiosk_currencies';
import { fill, setLang } from '../../shared/i18n';
import { th as sharedTh } from '../../shared/i18n/th';
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

// Branch B's list (JPY is off in B as well)
const GBP = currency('GBP', 'ปอนด์สเตอร์ลิง', 'Pound Sterling', true);
const LIST_B = [JPY, GBP];

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

function pageStatuses(): string[] {
  return [...host.querySelectorAll('[role="status"]')]
    .filter((el) => !el.closest('form'))
    .map(textOf);
}

async function showing(list: KioskCurrency[]): Promise<void> {
  await vi.waitFor(() => expect(headings()).toEqual(list.map(heading)));
}

describe('KioskCurrenciesScreen toggle re-read order and branch (#2453)', () => {
  it("row 1: toggle in A, switch to B, A's re-read answers late: B's currencies stay", async () => {
    const reReadA = deferred<ViewAnswer<KioskCurrencies>>();
    const f = fake([
      answer(LIST_A, 10),
      reReadA.promise,
      answer(LIST_B, 11),
      answer([{ ...JPY }, { ...GBP, enabled: false }], 11),
    ]);
    const setBranch = mount(f.api, 'A');
    await showing(LIST_A);

    switchOf(cardOf(JPY)).click();
    await vi.waitFor(() => expect(f.views).toHaveLength(2));
    expect(f.views[1]!.q).toEqual({ branch: 'A' });

    setBranch('B');
    await showing(LIST_B);

    reReadA.resolve(answer([AFN, { ...JPY, enabled: true }, USD], 10));
    await new Promise((r) => setTimeout(r, 20));

    expect(headings(), 'no A data on B').toEqual(LIST_B.map(heading));
    expect(switchOf(cardOf(JPY)).checked, "B's JPY stays off").toBe(false);
    expect(cardStatus(cardOf(JPY)), 'no A card status on B').toBe('');
    expect(textOf(host.querySelector('#bank-reference'))).toContain('11');

    // The next toggle on B sends B with a code from B's list.
    switchOf(cardOf(GBP)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(GBP))).toBe(th.savedOff));
    expect(f.posts[1]!.body).toEqual({
      action: 'configure',
      payload: { branchId: 'B', code: 'GBP', enabled: false },
    });
  });

  it("row 2: toggle X then Y; X's re-read lands after Y's: Y shows what Y's re-read reported", async () => {
    const reReadX = deferred<ViewAnswer<KioskCurrencies>>();
    const reReadY = deferred<ViewAnswer<KioskCurrencies>>();
    const f = fake([answer(LIST_A), reReadX.promise, reReadY.promise]);
    mount(f.api, 'A');
    await showing(LIST_A);

    // X = JPY on, Y = AFN on
    switchOf(cardOf(JPY)).click();
    await vi.waitFor(() => expect(f.views).toHaveLength(2));
    switchOf(cardOf(AFN)).click();
    await vi.waitFor(() => expect(f.views).toHaveLength(3));

    reReadY.resolve(
      answer([{ ...AFN, enabled: true }, { ...JPY, enabled: true }, USD]),
    );
    await vi.waitFor(() => expect(cardStatus(cardOf(AFN))).toBe(th.savedOn));
    expect(switchOf(cardOf(AFN)).checked).toBe(true);

    // X's older read was taken before Y's save landed: AFN still off there.
    reReadX.resolve(answer([AFN, { ...JPY, enabled: true }, USD]));
    await vi.waitFor(() => expect(switchOf(cardOf(JPY)).disabled).toBe(false));
    await new Promise((r) => setTimeout(r, 20));

    expect(switchOf(cardOf(AFN)).checked, "Y keeps the state Y's re-read reported").toBe(true);
    expect(cardStatus(cardOf(AFN))).toBe(th.savedOn);
    expect(switchOf(cardOf(JPY)).checked).toBe(true);
  });

  it("row 3: toggle in A, switch to B, A's re-read fails: no error and no patch on B's list", async () => {
    const reReadA = deferred<ViewAnswer<KioskCurrencies>>();
    const f = fake([answer(LIST_A), reReadA.promise, answer(LIST_B, 11)]);
    const setBranch = mount(f.api, 'A');
    await showing(LIST_A);

    switchOf(cardOf(JPY)).click();
    await vi.waitFor(() => expect(f.views).toHaveLength(2));

    setBranch('B');
    await showing(LIST_B);
    await vi.waitFor(() => expect(pageStatuses()).toContain(fill(th.found, { n: 2 })));

    reReadA.reject(new ApiError({ code: 'serverError', message: 'Read failed', status: 500 }));
    await new Promise((r) => setTimeout(r, 20));

    expect(pageStatuses()).not.toContain(sharedTh['error.serverError']);
    expect(pageStatuses()).toContain(fill(th.found, { n: 2 }));
    expect(switchOf(cardOf(JPY)).checked, "B's JPY is not patched on").toBe(false);
    expect(cardStatus(cardOf(JPY)), 'no A card status on B').toBe('');
  });
  it("row 4: A's toggle re-read sent before A -> B -> A must not overwrite the newer first read of A", async () => {
    const reReadA = deferred<ViewAnswer<KioskCurrencies>>();
    const f = fake([
      answer(LIST_A, 10),
      reReadA.promise,
      answer(LIST_B, 11),
      // newer first read of A, taken after JPY's save: JPY on, USD off
      answer([AFN, { ...JPY, enabled: true }, { ...USD, enabled: false }], 12),
    ]);
    const setBranch = mount(f.api, 'A');
    await showing(LIST_A);

    switchOf(cardOf(JPY)).click();
    await vi.waitFor(() => expect(f.views).toHaveLength(2));

    setBranch('B');
    await showing(LIST_B);
    setBranch('A');
    await vi.waitFor(() => expect(f.views).toHaveLength(4));
    await vi.waitFor(() => expect(switchOf(cardOf(USD)).checked).toBe(false));
    expect(textOf(host.querySelector('#bank-reference'))).toContain('12');

    // The older re-read (sent before A -> B -> A) lands last with USD on.
    reReadA.resolve(answer([AFN, { ...JPY, enabled: true }, USD], 10));
    await vi.waitFor(() => expect(switchOf(cardOf(JPY)).disabled).toBe(false));
    await new Promise((r) => setTimeout(r, 20));

    expect(switchOf(cardOf(USD)).checked, 'the newer first read of A keeps USD off').toBe(false);
    expect(textOf(host.querySelector('#bank-reference'))).toContain('12');
    expect(switchOf(cardOf(JPY)).checked).toBe(true);
  });

  it('row 5: configure held over A -> B -> A; re-read lands (JPY on), then the older first read (JPY off): JPY stays on and matches its card', async () => {
    const firstReadA2 = deferred<ViewAnswer<KioskCurrencies>>();
    const reReadA = deferred<ViewAnswer<KioskCurrencies>>();
    const f = fake([answer(LIST_A, 10), answer(LIST_B, 11), firstReadA2.promise, reReadA.promise]);
    const configureHeld = deferred<void>();
    const post = f.api.post.bind(f.api);
    f.api.post = <B, R>(path: string, body: B): Promise<R> =>
      configureHeld.promise.then(() => post<B, R>(path, body));
    const setBranch = mount(f.api, 'A');
    await showing(LIST_A);

    switchOf(cardOf(JPY)).click();
    setBranch('B');
    await showing(LIST_B);
    setBranch('A');
    await vi.waitFor(() => expect(f.views).toHaveLength(3));
    expect(f.views[2]!.q).toEqual({ branch: 'A' });

    // configure answers: the re-read of A is sent after the first read of A.
    configureHeld.resolve();
    await vi.waitFor(() => expect(f.views).toHaveLength(4));
    expect(f.views[3]!.q).toEqual({ branch: 'A' });

    reReadA.resolve(answer([AFN, { ...JPY, enabled: true }, USD], 12));
    await vi.waitFor(() => expect(cardStatus(cardOf(JPY))).toBe(th.savedOn));
    expect(switchOf(cardOf(JPY)).checked).toBe(true);
    await vi.waitFor(() => expect(pageStatuses()).toContain(fill(th.found, { n: 3 })));

    // The older first read (taken before the save landed) answers last.
    firstReadA2.resolve(answer(LIST_A, 10));
    await new Promise((r) => setTimeout(r, 20));

    expect(switchOf(cardOf(JPY)).checked, 'JPY stays on').toBe(true);
    expect(cardStatus(cardOf(JPY)), 'JPY card matches its switch').toBe(th.savedOn);
    expect(textOf(host.querySelector('#bank-reference'))).toContain('12');
    expect(pageStatuses()).toContain(fill(th.found, { n: 3 }));
  });

  it('row 6: toggle in A, switch to B, configure fails: no error on B\'s card', async () => {
    const f = fake([answer(LIST_A), answer(LIST_B, 11)]);
    const configureHeld = deferred<void>();
    f.api.post = <B, R>(_path: string, _body: B): Promise<R> =>
      configureHeld.promise.then(() => {
        throw new ApiError({ code: 'serverError', message: 'Save failed', status: 500 });
      });
    const setBranch = mount(f.api, 'A');
    await showing(LIST_A);

    switchOf(cardOf(JPY)).click();
    setBranch('B');
    await showing(LIST_B);

    configureHeld.resolve();
    await vi.waitFor(() => expect(switchOf(cardOf(JPY)).disabled).toBe(false));
    await new Promise((r) => setTimeout(r, 20));

    expect(cardStatus(cardOf(JPY)), "no A save error on B's JPY").toBe('');
    expect(f.views).toHaveLength(2);
  });
});
