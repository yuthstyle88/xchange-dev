// Ticket #603 (RE-6): the kiosk-currencies screen shows the branch's current rate publication and
// its switch saves no rate.
// Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/blueprint/rates.md`
// - §3.3: a rate on the wire is the 8-place string (`"23.50000000"`); the one web formatter,
//   `formatRate` (`web/src/shared/ui/rate.ts`, RE-5 #602), shows it, with `/ <unit> <code>` when the
//   unit is > 1. No screen formats a rate itself.
// - §4.8 and D-3: every reader reads the current publication; a rate changes only through a draft
//   and its publish, so this screen has no rate input and no save-rate control.
// - §4.9 R1/R3: v3 views speak per unit (JPY per 100); the KBank comparison is per 1 and is the
//   server's (the view divides by the unit and rounds, `api/tests/v3_kiosk_currencies_publication_test.rs`).
// - §9 RE-6.
//
// What is pinned:
// 1. Each published side of a card is shown as exactly `formatRate(side, unitSize, code)`, with
//    the view's `unitSize` (never a unit the screen knows from elsewhere): JPY `"23.50000000"` per
//    `"100"` is `23.50 / 100 JPY`; USD `"32.10000000"` per `"1"` is `32.10`; KRW `"23.12345678"`
//    per `"1000"` keeps all 8 places; a CNY line stored per `"10"` shows `/ 10 CNY`; a JPY line
//    stored per `"1"` shows no unit.
// 2. A `null` side is shown as the screen's "no rate" mark, `—` (v2's mark, `th.kbNote`). A
//    currency with no line (both sides `null`, `updated` `""`) shows `—` on both sides, and its
//    switch.
// 3. A card has no rate input and no save button; submitting any form sends nothing.
// 4. Ticking the switch sends `configure` at once, with `{ branchId, code, enabled }` and nothing
//    else (no `buy`, `sell`, `previousUpdated`, `partnerName`, `partnerCost`), then reads the view
//    again and says what the view answers. While it is sent the switch is disabled. A refusal
//    (v2's rates-first sentence) is shown on that card, the switch shows what the view holds
//    (off), and nothing is read again.
// 5. The KBank comparison and each card's KBank notes are the view's strings, unchanged: the shop
//    side is the server's per-1 figure, not the card's per-unit rate and not `formatRate`'s text.
//
// Fields and names assumed:
// - `KioskCurrency` gains `unitSize: string` (generated from the DTO by ts-rs; `v3_types_test.rs`);
//   every other field keeps its name and type.
// - The screen's table `./i18n` keeps `th.buy`, `th.sell` and `th.enabled` (read through the
//   table, so their wording may change). A side is attributed to `th.buy`/`th.sell` by the nearest
//   ancestor of its value whose text holds one of the two labels and not the other.
// - A card is the nearest ancestor of the heading `<code> · <name> / <nameEn>` that holds a
//   checkbox (the switch); the card's `[role="status"]` is its saved line.
//
// Amended by the ticket author in #603's label round: the table loses `save` (pinned in
// `i18n.test.tsx`), so the no-save-button test names the old button's two texts itself.
// - `KioskCurrenciesScreen({ api, branch })` and `configure` on `POST /api/kiosk`, as today.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewAnswer, type ViewParams } from '../../shared/api/client';
import type {
  KbankComparison,
  KioskCurrencies,
  KioskCurrency,
} from '../../shared/api/generated/kiosk_currencies';
import { fill, setLang } from '../../shared/i18n';
import { th } from './i18n';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

const sources = import.meta.glob<string>('./*.{ts,tsx}', {
  query: '?raw',
  import: 'default',
  eager: true,
});

// ── the data the view answers (v3 wire: 8-place rates per unit) ─────────────

const PUBLISHED = '2026-10-02T22:16:01+07:00';
const SOURCE = 'https://www.kasikornbank.com/en/rate/Pages/Foreign-Exchange.aspx';
const DASH = '—';
const RATES_FIRST = 'Set valid counter buy and sell rates first.';
/** The old save button's texts (v2's `save` key, which #603 removes from the table). */
const V2_SAVE_TH = 'บันทึกเรทและการเปิดใช้งาน';
const V2_SAVE_EN = 'Save rates and activation';

function cmp(
  label: string,
  bankBuy: string | null,
  shopBuy: string | null,
  buyDifference: string | null,
  bankSell: string | null,
  shopSell: string | null,
  sellDifference: string | null,
): KbankComparison {
  return { label, bankBuy, shopBuy, buyDifference, bankSell, shopSell, sellDifference };
}

/** A currency with no line: both sides null, no time, no partner. */
const AFN: KioskCurrency = {
  code: 'AFN',
  name: 'อัฟกานิอัฟกานิสถาน',
  nameEn: 'Afghan Afghani',
  countries: [{ code: 'AF', en: 'Afghanistan', th: 'อัฟกานิสถาน' }],
  buy: null,
  sell: null,
  unitSize: '1',
  updated: '',
  partnerName: '',
  partnerCost: null,
  enabled: false,
  kbank: [cmp('AFN', null, null, null, null, null, null)],
};

/** Published per 100; the comparison is the server's per-1 figure (23.5 ÷ 100 = 0.235). */
const JPY: KioskCurrency = {
  code: 'JPY',
  name: 'เยนญี่ปุ่น',
  nameEn: 'Japanese Yen',
  countries: [{ code: 'JP', en: 'Japan', th: 'ญี่ปุ่น' }],
  buy: '23.50000000',
  sell: '24.10000000',
  unitSize: '100',
  updated: '2026-10-09T05:00:01.123Z',
  partnerName: '',
  partnerCost: null,
  enabled: false,
  kbank: [cmp('JPY', '0.20715', '0.23500', '0.02785', '0.21844', '0.24100', '0.02256')],
};

/** Published per 1000, with all 8 places significant. */
const KRW: KioskCurrency = {
  code: 'KRW',
  name: 'วอนเกาหลีใต้',
  nameEn: 'South Korean Won',
  countries: [{ code: 'KR', en: 'South Korea', th: 'เกาหลีใต้' }],
  buy: '23.12345678',
  sell: '25.87654321',
  unitSize: '1000',
  updated: '2026-10-09T05:00:01.123Z',
  partnerName: '',
  partnerCost: null,
  enabled: true,
  kbank: [],
};

/** A line stored per 10 (the register says 1 for CNY): the view's unit is what is shown. */
const CNY: KioskCurrency = {
  code: 'CNY',
  name: 'หยวนจีน',
  nameEn: 'Chinese Yuan',
  countries: [{ code: 'CN', en: 'China', th: 'จีน' }],
  buy: '45.67800000',
  sell: '47.00000000',
  unitSize: '10',
  updated: '2026-10-09T05:00:01.123Z',
  partnerName: '',
  partnerCost: null,
  enabled: false,
  kbank: [],
};

/** Buy published, sell unavailable (a NULL side, design §4.1). */
const NOK: KioskCurrency = {
  code: 'NOK',
  name: 'โครนนอร์เวย์',
  nameEn: 'Norwegian Krone',
  countries: [{ code: 'NO', en: 'Norway', th: 'นอร์เวย์' }],
  buy: '3.45000000',
  sell: null,
  unitSize: '1',
  updated: '2026-10-09T05:00:01.123Z',
  partnerName: '',
  partnerCost: null,
  enabled: false,
  kbank: [cmp('NOK', null, null, null, '3.56995', null, null)],
};

const USD: KioskCurrency = {
  code: 'USD',
  name: 'ดอลลาร์สหรัฐ',
  nameEn: 'US Dollar',
  countries: [{ code: 'EC', en: 'Ecuador', th: 'เอกวาดอร์' }],
  buy: '32.10000000',
  sell: '33.00000000',
  unitSize: '1',
  updated: '2026-10-09T05:00:01.123Z',
  partnerName: 'SuperRich',
  partnerCost: '31.90000000',
  enabled: true,
  kbank: [cmp('USD 1', '32.37919', '32.10000', '-0.27919', '33.81656', '33.00000', '-0.81656')],
};

const CURRENCIES = [AFN, JPY, KRW, CNY, NOK, USD];

function answer(
  currencies: KioskCurrency[] = CURRENCIES,
  version = 7,
): ViewAnswer<KioskCurrencies> {
  return {
    version,
    changed: true,
    data: { currencies, kbank: { source: SOURCE, publishedAt: PUBLISHED, round: 10 }, freshness: { stale: false, sourceDate: null, publishedAt: null } },
  };
}

// ── the fake ApiClient ───────────────────────────────────────────────────────

interface Fake {
  api: Pick<ApiClient, 'view' | 'post'>;
  views: Array<{ name: string; q?: ViewParams }>;
  posts: Array<{ path: string; body: unknown }>;
}

/** `view` answers `answers` in turn (the last one again after that); `post` answers `onPost`. */
function fake(
  answers: Array<ViewAnswer<KioskCurrencies>>,
  onPost: (path: string, body: unknown) => Promise<unknown> = async () => ({ saved: true }),
): Fake {
  const views: Fake['views'] = [];
  const posts: Fake['posts'] = [];
  const api: Pick<ApiClient, 'view' | 'post'> = {
    view<T>(name: string, q?: ViewParams): Promise<ViewAnswer<T>> {
      views.push({ name, q: q === undefined ? undefined : { ...q } });
      const a = answers[Math.min(views.length - 1, answers.length - 1)]!;
      return Promise.resolve(structuredClone(a) as unknown as ViewAnswer<T>);
    },
    post<B, R>(path: string, body: B): Promise<R> {
      // what goes on the wire: JSON drops keys whose value is undefined
      posts.push({ path, body: JSON.parse(JSON.stringify(body)) as unknown });
      return onPost(path, body) as Promise<R>;
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

// ── the page ─────────────────────────────────────────────────────────────────

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

/** The card of `c`: the nearest ancestor of its heading that holds the switch. */
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

async function loaded(n = CURRENCIES.length): Promise<void> {
  await vi.waitFor(() =>
    expect(host.querySelectorAll('input[type="checkbox"]')).toHaveLength(n),
  );
}

/** The elements of `card` showing exactly `text`: as their whole text, or as one text node. */
function showing(card: HTMLElement, text: string): HTMLElement[] {
  const found = new Set<HTMLElement>();
  for (const el of card.querySelectorAll<HTMLElement>('*')) {
    if (textOf(el) === text && ![...el.children].some((ch) => textOf(ch) === text)) {
      found.add(el);
    }
  }
  const walker = document.createTreeWalker(card, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if ((n.nodeValue ?? '').trim() === text && n.parentElement) found.add(n.parentElement);
  }
  return [...found];
}

/** The side each place showing `text` belongs to: the nearest ancestor naming one label only. */
function sidesShowing(card: HTMLElement, text: string): Array<'buy' | 'sell' | 'none'> {
  return showing(card, text).map((el) => {
    let a: HTMLElement | null = el;
    while (a) {
      const t = textOf(a);
      const buy = t.includes(th.buy);
      const sell = t.includes(th.sell);
      if (buy && !sell) return 'buy';
      if (sell && !buy) return 'sell';
      if (buy && sell) return 'none';
      if (a === card) return 'none';
      a = a.parentElement;
    }
    return 'none';
  });
}

/** The sides `text` is shown on, leaving out places that belong to neither (the KBank notes). */
function sidesOf(card: HTMLElement, text: string): string[] {
  return sidesShowing(card, text)
    .filter((s) => s !== 'none')
    .sort();
}

function bankSection(): HTMLElement {
  const s = [...host.querySelectorAll<HTMLElement>('[aria-label]')].find(
    (el) => el.getAttribute('aria-label') === th.bankReference,
  );
  if (!s) throw new Error('no KBank section');
  return s;
}

function bodyRows(table: Element): string[][] {
  return [...table.querySelectorAll('tr')]
    .filter((tr) => tr.querySelector('td'))
    .map((tr) => [...tr.querySelectorAll('td')].map((td) => textOf(td)));
}

// ── 1. the published rate, through formatRate ───────────────────────────────

describe('#603 RE-6: each published side is shown through formatRate, per the view unit (§3.3, §4.8)', () => {
  it('JPY 23.50000000 per 100 shows 23.50 / 100 JPY on the buy side and 24.10 / 100 JPY on the sell side', async () => {
    mount(fake([answer()]).api);
    await loaded();
    const jpy = cardOf(JPY);
    expect(sidesOf(jpy, '23.50 / 100 JPY')).toEqual(['buy']);
    expect(sidesOf(jpy, '24.10 / 100 JPY')).toEqual(['sell']);
    expect(textOf(jpy), 'the stored text is not shown raw').not.toContain('23.50000000');
  });

  it('USD 32.10000000 per 1 shows 32.10, and 33.00000000 shows 33.00, with no unit', async () => {
    mount(fake([answer()]).api);
    await loaded();
    const usd = cardOf(USD);
    expect(sidesOf(usd, '32.10')).toEqual(['buy']);
    expect(sidesOf(usd, '33.00')).toEqual(['sell']);
    expect(textOf(usd)).not.toContain('/ 1 USD');
  });

  it('KRW per 1000 keeps all eight significant places: 23.12345678 / 1000 KRW and 25.87654321 / 1000 KRW', async () => {
    mount(fake([answer()]).api);
    await loaded();
    const krw = cardOf(KRW);
    expect(sidesOf(krw, '23.12345678 / 1000 KRW')).toEqual(['buy']);
    expect(sidesOf(krw, '25.87654321 / 1000 KRW')).toEqual(['sell']);
  });

  it("the unit is the view's unitSize: a CNY line stored per 10 shows / 10 CNY", async () => {
    mount(fake([answer()]).api);
    await loaded();
    const cny = cardOf(CNY);
    expect(sidesOf(cny, '45.678 / 10 CNY')).toEqual(['buy']);
    expect(sidesOf(cny, '47.00 / 10 CNY')).toEqual(['sell']);
  });

  it("the unit is the view's unitSize: a JPY line stored per 1 shows no unit", async () => {
    const perOne: KioskCurrency = { ...JPY, buy: '0.23500000', sell: '0.24100000', unitSize: '1' };
    mount(fake([answer([perOne])]).api);
    await loaded(1);
    const jpy = cardOf(perOne);
    expect(sidesOf(jpy, '0.235')).toEqual(['buy']);
    expect(sidesOf(jpy, '0.241')).toEqual(['sell']);
    expect(textOf(jpy)).not.toContain('/ 100 JPY');
  });

  it('a null side shows the no-rate mark — on that side only', async () => {
    mount(fake([answer()]).api);
    await loaded();
    const nok = cardOf(NOK);
    expect(sidesOf(nok, '3.45')).toEqual(['buy']);
    expect(sidesOf(nok, DASH)).toEqual(['sell']);
  });

  it('a currency with no line shows — on both sides, no rate, and its switch (off)', async () => {
    mount(fake([answer()]).api);
    await loaded();
    const afn = cardOf(AFN);
    expect(sidesOf(afn, DASH)).toEqual(['buy', 'sell']);
    expect(switchOf(afn).checked).toBe(false);
    expect(textOf(afn)).not.toMatch(/\d\.\d/);
    expect(textOf(afn)).not.toMatch(/null|undefined|NaN/);
  });

  it('the switch shows what the view holds', async () => {
    mount(fake([answer()]).api);
    await loaded();
    expect(switchOf(cardOf(USD)).checked).toBe(true);
    expect(switchOf(cardOf(KRW)).checked).toBe(true);
    expect(switchOf(cardOf(JPY)).checked).toBe(false);
    expect(
      [...cardOf(USD).querySelectorAll('label')].some((l) => textOf(l) === th.enabled),
      'the switch keeps its label',
    ).toBe(true);
  });
});

// ── 2. no rate is entered here ──────────────────────────────────────────────

describe('#603 RE-6: the screen has no rate input and no save-rate control (D-3, §4.8)', () => {
  it('no card has an input other than its switch, and no save button', async () => {
    mount(fake([answer()]).api);
    await loaded();
    for (const c of CURRENCIES) {
      const card = cardOf(c);
      expect(
        [...card.querySelectorAll('input')].filter((i) => i.type !== 'checkbox'),
        c.code,
      ).toEqual([]);
      expect(card.querySelectorAll('textarea, select, [contenteditable="true"]'), c.code).toHaveLength(0);
      expect(
        [...card.querySelectorAll('button')].filter((b) => [V2_SAVE_TH, V2_SAVE_EN].includes(textOf(b))),
        c.code,
      ).toEqual([]);
    }
    expect(host.querySelectorAll('input[type="number"]')).toHaveLength(0);
  });

  it('submitting any form of the page sends nothing', async () => {
    const f = fake([answer()]);
    mount(f.api);
    await loaded();
    for (const form of host.querySelectorAll('form')) {
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    }
    await new Promise((r) => setTimeout(r, 20));
    expect(f.posts).toEqual([]);
    expect(f.views).toHaveLength(1);
  });
});

// ── 3. the switch ────────────────────────────────────────────────────────────

describe('#603 RE-6: ticking the switch sends configure with no rate (§4.8, D-3)', () => {
  it('switching JPY on sends exactly { branchId, code, enabled: true }, reads the view again and says it is on', async () => {
    const after = answer([AFN, { ...JPY, enabled: true }, KRW, CNY, NOK, USD], 8);
    const f = fake([answer(), after]);
    mount(f.api, 'B9');
    await loaded();
    switchOf(cardOf(JPY)).click();

    await vi.waitFor(() => expect(f.views).toHaveLength(2));
    expect(f.posts).toStrictEqual([
      {
        path: '/api/kiosk',
        body: { action: 'configure', payload: { branchId: 'B9', code: 'JPY', enabled: true } },
      },
    ]);
    expect(f.views[1]!.name).toBe('kiosk_currencies');
    expect(f.views[1]!.q?.branch).toBe('B9');
    await vi.waitFor(() => expect(cardStatus(cardOf(JPY))).toBe(th.savedOn));
    expect(switchOf(cardOf(JPY)).checked).toBe(true);
    expect(cardStatus(cardOf(USD))).toBe('');
  });

  it('switching USD off sends exactly { branchId: MAIN, code, enabled: false } — no partner, no time — and says it is off', async () => {
    const after = answer([AFN, JPY, KRW, CNY, NOK, { ...USD, enabled: false }], 8);
    const f = fake([answer(), after]);
    mount(f.api, 'MAIN');
    await loaded();
    switchOf(cardOf(USD)).click();

    await vi.waitFor(() => expect(cardStatus(cardOf(USD))).toBe(th.savedOff));
    expect(f.posts).toStrictEqual([
      {
        path: '/api/kiosk',
        body: { action: 'configure', payload: { branchId: 'MAIN', code: 'USD', enabled: false } },
      },
    ]);
    expect(switchOf(cardOf(USD)).checked).toBe(false);
  });

  it('the saved line says what the view answers after, not what was ticked', async () => {
    // the view still answers JPY off after the write (e.g. switched off again elsewhere)
    const f = fake([answer(), answer(CURRENCIES, 8)]);
    mount(f.api);
    await loaded();
    switchOf(cardOf(JPY)).click();
    await vi.waitFor(() => expect(cardStatus(cardOf(JPY))).toBe(th.savedOff));
    expect(switchOf(cardOf(JPY)).checked).toBe(false);
  });

  it('while it is sent the switch is disabled; the other cards stay usable', async () => {
    const pending = deferred<unknown>();
    const f = fake([answer(), answer([AFN, { ...JPY, enabled: true }, KRW, CNY, NOK, USD], 8)], () => pending.promise);
    mount(f.api);
    await loaded();
    switchOf(cardOf(JPY)).click();
    await vi.waitFor(() => expect(switchOf(cardOf(JPY)).disabled).toBe(true));
    expect(switchOf(cardOf(USD)).disabled).toBe(false);
    expect(f.posts).toHaveLength(1);

    pending.resolve({ saved: true });
    await vi.waitFor(() => expect(cardStatus(cardOf(JPY))).toBe(th.savedOn));
    expect(switchOf(cardOf(JPY)).disabled).toBe(false);
  });

  it("a refusal shows the server's sentence on that card, the switch goes back to off, and nothing is read again", async () => {
    const f = fake([answer()], () =>
      Promise.reject(new ApiError({ code: 'refused', message: RATES_FIRST, status: 400 })),
    );
    mount(f.api);
    await loaded();
    switchOf(cardOf(AFN)).click();

    await vi.waitFor(() => expect(cardStatus(cardOf(AFN))).toBe(RATES_FIRST));
    expect(f.posts).toStrictEqual([
      {
        path: '/api/kiosk',
        body: { action: 'configure', payload: { branchId: 'B9', code: 'AFN', enabled: true } },
      },
    ]);
    await vi.waitFor(() => expect(switchOf(cardOf(AFN)).checked).toBe(false));
    expect(switchOf(cardOf(AFN)).disabled).toBe(false);
    expect(cardStatus(cardOf(JPY))).toBe('');
    expect(f.views, 'a refused switch reads nothing again').toHaveLength(1);
  });
});

// ── 4. the KBank figures are the server's ───────────────────────────────────

describe("#603 RE-6: the KBank comparison is the view's per-1 strings, unchanged (§4.9 R1, §9 RE-6)", () => {
  it("the comparison shows the server's per-1 shop figure and difference, not the card's per-unit rate", async () => {
    mount(fake([answer()]).api);
    await loaded();
    const rows = bodyRows(bankSection().querySelector('table')!);
    expect(rows).toContainEqual(['JPY', '0.20715', '0.23500', '0.02785', '0.21844', '0.24100', '0.02256']);
    expect(rows).toContainEqual(['USD 1', '32.37919', '32.10000', '-0.27919', '33.81656', '33.00000', '-0.81656']);
    expect(rows).toContainEqual(['NOK', DASH, DASH, DASH, '3.56995', DASH, DASH]);
  });

  it("each card's KBank notes are the view's strings", async () => {
    mount(fake([answer()]).api);
    await loaded();
    const time = new Date(PUBLISHED).toLocaleString('th-TH', { timeZone: 'Asia/Bangkok' });
    const stamp = fill(th.kbStamp, { time, round: 10 });
    expect(bodyRows(cardOf(JPY).querySelector('table')!)).toEqual([['JPY', '0.20715', '0.21844', stamp]]);
  });
});

// ── 5. the source ───────────────────────────────────────────────────────────

describe('#603 RE-6: the screen formats rates only through formatRate (§3.3, ♻️)', () => {
  const own = Object.entries(sources).filter(([path]) => !path.includes('.test.'));

  it('imports formatRate from shared/ui/rate', () => {
    const all = own.map(([, t]) => t).join('\n');
    expect(all).toMatch(/import\s*\{[^}]*\bformatRate\b[^}]*\}\s*from\s*['"]\.\.\/\.\.\/shared\/ui\/rate['"]/);
  });

  it('formats no number itself: no toFixed, parseFloat or Intl.NumberFormat', () => {
    for (const [path, t] of own) {
      expect(t, path).not.toMatch(/\.toFixed\(|parseFloat\(|NumberFormat/);
    }
  });
});
