// V1-kiosk-currencies (#399): the currency and counter-rate screen at /kiosk-currencies.html
// (ticket §7.4, §9, §10). Written before the implementation by the lane controller. Do not edit,
// delete or weaken; add tests in a file of your own.
//
// The screen runs against a fake `ApiClient`: `view` answers what the test gives it, `post`
// records what it is sent. Its own text is read through its table (`th.<key>`, W10); what v2
// showed is pinned by `i18n.test.tsx`. The rates and the KBank figures are the view's strings,
// shown as they come (W7): one KBank difference below is deliberately not shop − bank, so a
// screen that computes shows the wrong figure.
//
// Amended by the ticket author of #603 (RE-6, claude/e5ab6c; `docs/design/blueprint/rates.md`
// §4.8, §4.9 R1, §9 RE-6): each currency of the view gains `unitSize`, the unit its rates are per.
// The four fixtures here (AFN, JPY, NOK, USD) say `'1'`, which keeps their rates and KBank figures
// as they were.
// RE-6 removes the rate fields and the save button (D-3: a rate changes only through a draft and
// its publish). So `cards()` finds a card by its switch, and "a card: …" no longer pins the rate
// fields (C8), and the six saving tests are retired (see the note at the end of this file).
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
  type KbankReference,
  type KioskCurrencies,
  type KioskCurrency,
} from '../../shared/testing/fixtures';
import { errorText, fill, setLang } from '../../shared/i18n';
import page from '../../../pages/kiosk-currencies.html?raw';
import { en, th } from './i18n';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

// ── the data the view answers ────────────────────────────────────────────────

const PUBLISHED = '2026-10-02T22:16:01+07:00';
const SOURCE = 'https://www.kasikornbank.com/en/rate/Pages/Foreign-Exchange.aspx';
const DASH = '—';
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
  buy: '0.2105',
  sell: '0.2205',
  unitSize: '1',
  updated: '2026-10-06T05:00:02.123Z',
  partnerName: '',
  partnerCost: null,
  enabled: false,
  kbank: [cmp('JPY', '0.20715', '0.21050', '0.00335', '0.21844', '0.22050', '0.00206')],
});

const NOK = makeKioskCurrency({
  code: 'NOK',
  name: 'โครนนอร์เวย์',
  nameEn: 'Norwegian Krone',
  countries: [
    makeKioskCountry({ code: 'NO', en: 'Norway', th: 'นอร์เวย์' }),
    makeKioskCountry({ code: 'SJ', en: 'Svalbard & Jan Mayen', th: 'สฟาลบาร์และยานไมเอน' }),
  ],
  buy: null,
  sell: null,
  unitSize: '1',
  updated: '',
  partnerName: '',
  partnerCost: null,
  enabled: false,
  kbank: [cmp('NOK', null, null, null, '3.56995', null, null)],
});

const USD = makeKioskCurrency({
  code: 'USD',
  name: 'ดอลลาร์สหรัฐ',
  nameEn: 'US Dollar',
  countries: [
    makeKioskCountry({ code: 'AS', en: 'American Samoa', th: 'อเมริกันซามัว' }),
    makeKioskCountry({ code: 'EC', en: 'Ecuador', th: 'เอกวาดอร์' }),
  ],
  buy: '33.2500',
  sell: '33.5000',
  unitSize: '1',
  updated: '2026-10-06T05:00:01.123Z',
  partnerName: 'SuperRich',
  partnerCost: '32.9000',
  enabled: true,
  kbank: [
    cmp('USD 1', '32.37919', '33.25000', '0.87081', '33.81656', '33.50000', '-0.31656'),
    cmp('USD 5-20', '32.58661', '33.25000', '0.66339', '33.81656', '33.50000', '-0.31656'),
    // not shop − bank on purpose: the screen shows the view's figure and computes nothing
    cmp('USD 50-100', '33.10516', '33.25000', '9.99999', '33.85113', '33.50000', '-0.35113'),
  ],
});

const CURRENCIES = [AFN, JPY, NOK, USD];

const KBANK_DEFAULT = makeKbankReference({ source: SOURCE, publishedAt: PUBLISHED, round: 10 });

function answer(
  currencies?: KioskCurrency[],
  kbank?: KbankReference | null,
  version = 7,
) {
  return makeViewAnswer(
    makeKioskCurrencies({
      currencies: currencies ?? CURRENCIES,
      kbank: kbank === undefined ? KBANK_DEFAULT : kbank,
      freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
    }),
    { version, changed: true },
  );
}

// ── the fake ApiClient ───────────────────────────────────────────────────────

interface Fake {
  api: Pick<ApiClient, 'view' | 'post'>;
  views: Array<{ name: string; q?: ViewParams }>;
  posts: Array<{ path: string; body: unknown }>;
}

/** `view` answers `answers` in turn (the last one again after that); `post` answers `onPost`. */
function fake(
  answers: Array<ViewAnswer<KioskCurrencies> | ApiError>,
  onPost: (path: string, body: unknown) => Promise<unknown> = async () => ({ saved: true }),
): Fake {
  const views: Fake['views'] = [];
  const posts: Fake['posts'] = [];
  const api: Pick<ApiClient, 'view' | 'post'> = {
    view<T>(name: string, q?: ViewParams): Promise<ViewAnswer<T>> {
      views.push({ name, q: q === undefined ? undefined : { ...q } });
      const a = answers[Math.min(views.length - 1, answers.length - 1)]!;
      if (a instanceof ApiError) return Promise.reject(a);
      return Promise.resolve(structuredClone(a) as any);
    },
    post<B, R>(path: string, body: B): Promise<R> {
      posts.push({ path, body: structuredClone(body) });
      return onPost(path, body) as Promise<R>;
    },
  };
  return { api, views, posts };
}

// ── the page ─────────────────────────────────────────────────────────────────

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let fetchSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  setLang('th');
  // AppShell may read /health/ready; nothing on this screen fetches /api/ by itself.
  fetchSpy = vi.fn(() => Promise.reject(new Error('offline in tests')));
  vi.stubGlobal('fetch', fetchSpy);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.useRealTimers();
  const own = fetchSpy.mock.calls
    .map((c) => String(c[0]))
    .filter((url) => url.includes('/api/'));
  vi.unstubAllGlobals();
  setLang('th');
  expect(own, 'no fetch of its own: every call goes through the ApiClient').toEqual([]);
});

function mount(api: Fake['api'], branch = 'B9'): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <KioskCurrenciesScreen api={api} branch={branch} />, host);
}

function textOf(el: Element | null | undefined): string {
  return (el?.textContent ?? '').trim();
}

/** The innermost elements under `root` whose text is `text`. */
function byText(root: ParentNode, text: string): HTMLElement[] {
  const all = [...root.querySelectorAll<HTMLElement>('*')].filter((el) => textOf(el) === text);
  return all.filter((el) => ![...el.children].some((child) => textOf(child) === text));
}

function heading(c: KioskCurrency): string {
  return `${c.code} · ${c.name} / ${c.nameEn}`;
}

/**
 * The currency cards: around each switch labelled `enabled` (its text in the language shown), the
 * nearest ancestor that holds a heading. Amended for #603 (was: forms with the save button, which
 * RE-6 removes, design D-3/§4.8).
 */
function cards(enabled: string = th.enabled): HTMLElement[] {
  const found: HTMLElement[] = [];
  for (const l of host.querySelectorAll('label')) {
    if (textOf(l) !== enabled) continue;
    let el: HTMLElement | null = l.parentElement;
    while (el && el !== host && !el.querySelector('h1, h2, h3, h4, h5, h6')) el = el.parentElement;
    if (el && el !== host && !found.includes(el)) found.push(el);
  }
  return found;
}

function cardOf(c: KioskCurrency): HTMLElement {
  const card = cards().find((f) =>
    [...f.querySelectorAll('h1, h2, h3, h4, h5, h6')].some((h) => textOf(h) === heading(c)),
  );
  if (!card) throw new Error(`no card headed ${heading(c)}`);
  return card;
}

function shownCodes(): string[] {
  return cards().map((f) => {
    const h = [...f.querySelectorAll('h1, h2, h3, h4, h5, h6')]
      .map(textOf)
      .find((t) => t.includes(' · '));
    return (h ?? '').split(' · ')[0]!;
  });
}

/** The texts of the status lines outside the cards. */
function pageStatuses(): string[] {
  return [...host.querySelectorAll('[role="status"]')].filter((el) => !el.closest('form')).map(textOf);
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

/** The KBank section, by its aria-label (`label` = its text in the language shown). */
function bankSection(label: string = th.bankReference): HTMLElement {
  const s = [...host.querySelectorAll<HTMLElement>('[aria-label]')].find(
    (el) => el.getAttribute('aria-label') === label,
  );
  if (!s) throw new Error(`no KBank section labelled ${label}`);
  return s;
}

function headerCells(table: Element): string[] {
  return [...table.querySelectorAll('th')].map(textOf);
}

function bodyRows(table: Element): string[][] {
  return [...table.querySelectorAll('tr')]
    .filter((tr) => tr.querySelector('td'))
    .map((tr) => [...tr.querySelectorAll('td')].map(textOf));
}

function shown(v: string | null): string {
  return v ?? DASH;
}

function stamp(): string {
  const time = new Date(PUBLISHED).toLocaleString('th-TH', { timeZone: 'Asia/Bangkok' });
  return fill(th.kbStamp, { time, round: 10 });
}

async function loaded(n = CURRENCIES.length): Promise<void> {
  await vi.waitFor(() => expect(cards()).toHaveLength(n));
}

// ── the page entry ───────────────────────────────────────────────────────────

describe('web/pages/kiosk-currencies.html (#399 §7.4)', () => {
  it('is Thai, titled as v2, at /kiosk-currencies.html behind the signed-in gate, running the screen', () => {
    const doc = new DOMParser().parseFromString(page, 'text/html');
    expect(doc.documentElement.getAttribute('lang')).toBe('th');
    expect(doc.title).toBe('สกุลเงินหน้าลูกค้า');
    const meta = (name: string) =>
      [...doc.querySelectorAll('meta')].find((m) => m.getAttribute('name') === name)?.getAttribute('content');
    expect(meta('xc-path')).toBe('/kiosk-currencies.html');
    expect(meta('xc-gate')).toBe('signed-in');
    const scripts = [...doc.querySelectorAll('script')].map((s) => [s.getAttribute('type'), s.getAttribute('src')]);
    expect(scripts).toContainEqual(['module', '/src/screens/kiosk-currencies/main.tsx']);
  });
});

// ── reading ──────────────────────────────────────────────────────────────────

describe('KioskCurrenciesScreen reading (#399)', () => {
  it('reads the kiosk_currencies view for the branch it is given, once, and never polls (C15, C16)', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const f = fake([answer()]);
    mount(f.api, 'B9');
    await loaded();
    await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    expect(f.views).toHaveLength(1);
    expect(f.views[0]!.name).toBe('kiosk_currencies');
    expect(f.views[0]!.q?.branch).toBe('B9');
    expect(f.views[0]!.q?.since).toBeUndefined();
    expect(f.posts).toEqual([]);
  });

  it("shows v2's page texts through its table (C2, C4)", async () => {
    mount(fake([answer()]).api);
    await loaded();
    const back = [...host.querySelectorAll('a')].find((a) => textOf(a) === th.back);
    expect(back?.getAttribute('href')).toBe('/');
    expect(byText(host, th.heading).length).toBeGreaterThan(0);
    expect(byText(host, th.intro)).toHaveLength(1);
    const search = host.querySelector<HTMLInputElement>('input[type="search"]');
    expect(search).not.toBeNull();
    expect(search!.placeholder).toBe(th.searchPlaceholder);
    const label = [...host.querySelectorAll('label')].find((l) => textOf(l) === th.searchLabel);
    expect(label, 'the search has its label').toBeDefined();
    expect(label!.htmlFor ? label!.htmlFor === search!.id : label!.contains(search)).toBe(true);
    expect(bankSection()).toBeDefined();
  });

  it("one card per currency in the view's order, and the count in the page message (C6, C7)", async () => {
    mount(fake([answer()]).api);
    await loaded();
    expect(shownCodes()).toEqual(['AFN', 'JPY', 'NOK', 'USD']);
    expect(pageStatuses()).toContain(fill(th.found, { n: 4 }));
  });

  // Amended for #603 (RE-6, design D-3/§4.8): the rate fields and their browser checks (C8) are
  // gone; how a card shows its published rate is pinned in KioskCurrenciesScreen.publication.test.tsx.
  it("a card: heading, the countries' Thai names and the switch (C7)", async () => {
    mount(fake([answer()]).api);
    await loaded();
    const usd = cardOf(USD);
    expect(byText(usd, 'อเมริกันซามัว, เอกวาดอร์')).toHaveLength(1);
    const enabled = usd.querySelector<HTMLInputElement>('input[type="checkbox"]');
    expect(enabled?.checked).toBe(true);
    expect([...usd.querySelectorAll('label')].some((l) => textOf(l) === th.enabled)).toBe(true);
    expect(cardOf(JPY).querySelector<HTMLInputElement>('input[type="checkbox"]')?.checked).toBe(false);
  });

  it('the search filters the cards and the count on every keystroke (C4, C5, C6)', async () => {
    mount(fake([answer()]).api);
    await loaded();
    const search = host.querySelector<HTMLInputElement>('input[type="search"]')!;
    for (const [query, codes] of [
      ['japan', ['JPY']],
      ['ญี่ปุ่น', ['JPY']],
      ['JP', ['JPY']],
      ['  usd ', ['USD']],
      ['svalbard', ['NOK']],
      ['zzz', []],
      ['', ['AFN', 'JPY', 'NOK', 'USD']],
    ] as const) {
      type(search, query);
      await vi.waitFor(() => expect(shownCodes(), query).toEqual(codes));
      expect(pageStatuses(), query).toContain(fill(th.found, { n: codes.length }));
    }
  });

  it('a load refusal shows the refusal in the page message, and no card', async () => {
    const refusal = new ApiError({ code: 'kioskOwnerOrRates', message: OWNER_OR_RATES, status: 400 });
    mount(fake([refusal]).api);
    const expected = errorText('kioskOwnerOrRates', OWNER_OR_RATES);
    await vi.waitFor(() => expect(pageStatuses()).toContain(expected));
    expect(cards()).toEqual([]);
  });

  // Amended for #603: the cards are found by the switch's label, not by the save button RE-6 removes.
  it('every text follows the language: English when chosen, Thai again after the switch', async () => {
    setLang('en');
    mount(fake([answer()]).api);
    await vi.waitFor(() => expect(cards(en.enabled)).toHaveLength(4));
    expect(cards(th.enabled)).toEqual([]);
    expect(byText(host, en.heading).length).toBeGreaterThan(0);
    expect(host.querySelector<HTMLInputElement>('input[type="search"]')!.placeholder).toBe(en.searchPlaceholder);
    expect(pageStatuses()).toContain(fill(en.found, { n: 4 }));
    expect(byText(bankSection(en.bankReference), en.compareHeading)).toHaveLength(1);

    setLang('th');
    await loaded();
    expect(byText(host, th.heading).length).toBeGreaterThan(0);
    expect(byText(host, en.heading)).toEqual([]);
    expect(pageStatuses()).toContain(fill(th.found, { n: 4 }));
    expect(byText(bankSection(th.bankReference), th.compareHeading)).toHaveLength(1);
  });
});

// ── KBank (C11–C14, V6) ──────────────────────────────────────────────────────

describe('KioskCurrenciesScreen KBank reference (#399)', () => {
  it("each card shows its KBank notes: the view's strings, — for null, and the stamp (C12)", async () => {
    mount(fake([answer()]).api);
    await loaded();
    const usd = cardOf(USD);
    expect(byText(usd, th.kbCaption)).toHaveLength(1);
    expect(byText(usd, th.kbNote)).toHaveLength(1);
    const table = usd.querySelector('table')!;
    expect(headerCells(table)).toEqual([th.colLabel, th.kbColBuy, th.kbColSell, th.kbColTime]);
    expect(bodyRows(table)).toEqual(
      USD.kbank.map((r) => [r.label, shown(r.bankBuy), shown(r.bankSell), stamp()]),
    );
    expect(bodyRows(cardOf(AFN).querySelector('table')!)).toEqual([['AFN', DASH, DASH, stamp()]]);
    expect(bodyRows(cardOf(NOK).querySelector('table')!)).toEqual([['NOK', DASH, '3.56995', stamp()]]);
  });

  it("the comparison: v2's texts, the source link, and every figure as the view sent it (C13, C14)", async () => {
    mount(fake([answer()]).api);
    await loaded();
    const section = bankSection();
    const time = new Date(PUBLISHED).toLocaleString('th-TH', { timeZone: 'Asia/Bangkok' });
    expect(byText(section, th.compareHeading)).toHaveLength(1);
    expect(byText(section, fill(th.comparePublished, { time, round: 10 }))).toHaveLength(1);
    expect(byText(section, th.compareWarning)).toHaveLength(1);
    expect(byText(section, th.compareNote)).toHaveLength(1);
    const link = [...section.querySelectorAll('a')].find((a) => textOf(a) === th.compareLink);
    expect(link?.getAttribute('href')).toBe(SOURCE);
    expect(link?.getAttribute('target')).toBe('_blank');
    expect(link?.getAttribute('rel')).toContain('noopener');

    const table = section.querySelector('table')!;
    expect(headerCells(table)).toEqual([
      th.colLabel,
      th.cmpColBankBuy,
      th.cmpColShopBuy,
      th.cmpColBuyDiff,
      th.cmpColBankSell,
      th.cmpColShopSell,
      th.cmpColSellDiff,
    ]);
    const expected = CURRENCIES.flatMap((c) =>
      c.kbank.map((r) => [
        r.label,
        shown(r.bankBuy),
        shown(r.shopBuy),
        shown(r.buyDifference),
        shown(r.bankSell),
        shown(r.shopSell),
        shown(r.sellDifference),
      ]),
    );
    expect(bodyRows(table)).toEqual(expected);
    expect(bodyRows(table)).toContainEqual(['USD 50-100', '33.10516', '33.25000', '9.99999', '33.85113', '33.50000', '-0.35113']);
  });

  it('the comparison follows the search (C13)', async () => {
    mount(fake([answer()]).api);
    await loaded();
    type(host.querySelector<HTMLInputElement>('input[type="search"]')!, 'nok');
    await vi.waitFor(() =>
      expect(bodyRows(bankSection().querySelector('table')!)).toEqual([
        ['NOK', DASH, DASH, DASH, '3.56995', DASH, DASH],
      ]),
    );
  });

  it("no KBank reference: the section says v2's failure sentence, and the cards still show (C11, V6)", async () => {
    mount(fake([answer(CURRENCIES, null)]).api);
    await loaded();
    expect(textOf(bankSection())).toBe(th.kbankFailed);
  });
});

// ── saving (C10, V2, V8): retired for #603 ──────────────────────────────────
//
// Retired by the ticket author of #603 (RE-6, claude/e5ab6c; `docs/design/blueprint/rates.md`
// D-3, §4.8, §9 RE-6): the screen no longer saves a rate, so its save button and rate fields are
// gone and the switch sends `configure` with `{ branchId, code, enabled }` alone. The six tests
// here pinned the old save:
// - "a save sends configure with strings and the row as read, then reads the view again (§9)"
// - "a save of a currency left off says so, and sends no partner as empty and null"
// - "the switch is sent as it is ticked, and the saved line says what the view answers after"
// - "while saving the button is disabled; a refusal shows the server's sentence on that card and
//   frees the button (§9)"
// - "a rate field left empty is checked before sending: no request, the field marked and focused (V8)"
// - "a rate under the minimum is checked before sending too (V8)"
// Their replacements (the switch's exact body, the re-read, the saved line, the disabled switch
// while sending, the refusal on its card) are in KioskCurrenciesScreen.publication.test.tsx. V8
// has no replacement: there is no rate field left to check.
