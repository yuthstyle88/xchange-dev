// RF-3 (design docs/design/blueprint/rates.md §4.10, PR 843): the kiosk currencies page shows the current
// publication's freshness — the stale badge, the reference's source date and the publication's time — through the ONE
// shared component `web/src/shared/ui/RateFreshness.tsx` (RF-2; its own tests: RateFreshness.test.tsx), embedded ONCE
// per page, not once per currency card. The data is the view's new `data.freshness` (`RateFreshnessDto`:
// `{ stale, sourceDate, publishedAt }`, RF-1), passed through unchanged.
// Written before the implementation by the ticket author (claude/dell-claude-3). Do not edit, delete or weaken; add
// tests in a file of your own.
//
// What is pinned:
// 1. KioskCurrenciesScreen.tsx imports the shared RateFreshness and declares no freshness markup of its own.
// 2. Exactly one `p.xc-rate-freshness` on the page, whatever the number of cards, and none inside a card.
// 3. It shows the view's `freshness`: the badge when `stale`; `rate.sourceDate` with the date; `rate.publishedAt`
//    with the time; `rate.noSourceDate` and no time before the first publication.
// 4. After a switch is ticked the screen reads the view again; the freshness shown is the new answer's, still once.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { ApiClient, ViewAnswer, ViewParams } from '../../shared/api/client';
import type { KioskCurrencies, KioskCurrency } from '../../shared/api/generated/kiosk_currencies';
import { fill, locale, setLang } from '../../shared/i18n';
import { th } from '../../shared/i18n/th';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

const sources = import.meta.glob<string>('./KioskCurrenciesScreen.tsx', {
  query: '?raw',
  import: 'default',
  eager: true,
});

/** `RateFreshnessDto` on the wire (RF-1, `dto/v3/common.rs`). Declared here so this file does not wait on codegen. */
interface Freshness {
  stale: boolean;
  sourceDate: string | null;
  publishedAt: string | null;
}
type Data = KioskCurrencies & { freshness: Freshness };

const PUBLISHED_AT = '2026-10-10T03:15:00.123Z';

function currency(code: string, name: string, nameEn: string): KioskCurrency {
  return {
    code,
    name,
    nameEn,
    countries: [],
    buy: '32.10000000',
    sell: '33.00000000',
    unitSize: '1',
    updated: PUBLISHED_AT,
    partnerName: '',
    partnerCost: null,
    enabled: false,
    kbank: [],
  };
}

const CURRENCIES = [
  currency('EUR', 'ยูโร', 'Euro'),
  currency('JPY', 'เยนญี่ปุ่น', 'Japanese Yen'),
  currency('USD', 'ดอลลาร์สหรัฐ', 'US Dollar'),
];

function answer(freshness: Freshness, version = 7): ViewAnswer<Data> {
  return {
    version,
    changed: true,
    data: {
      currencies: CURRENCIES,
      kbank: { source: 'https://www.kasikornbank.com/', publishedAt: '2026-10-02T22:16:01+07:00', round: 10 },
      freshness,
    },
  };
}

interface Fake {
  api: Pick<ApiClient, 'view' | 'post'>;
  views: string[];
}

/** `view` answers `answers` in turn (the last one again after that); `post` saves. */
function fake(answers: Array<ViewAnswer<Data>>): Fake {
  const views: string[] = [];
  const api: Pick<ApiClient, 'view' | 'post'> = {
    view<T>(name: string, _q?: ViewParams): Promise<ViewAnswer<T>> {
      views.push(name);
      const a = answers[Math.min(views.length - 1, answers.length - 1)]!;
      return Promise.resolve(structuredClone(a) as unknown as ViewAnswer<T>);
    },
    post<B, R>(_path: string, _body: B): Promise<R> {
      return Promise.resolve({ saved: true } as unknown as R);
    },
  };
  return { api, views };
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

async function mount(api: Fake['api']): Promise<HTMLDivElement> {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <KioskCurrenciesScreen api={api} branch="MAIN" />, host);
  await settle();
  return host;
}

function text(n: Element): string {
  return (n.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function key(k: string): string {
  return (th as Record<string, string>)[k] ?? `<missing ${k}>`;
}

function at(iso: string): string {
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

/** The page's one freshness line. */
function theLine(el: HTMLElement): HTMLElement {
  const lines = el.querySelectorAll<HTMLElement>('p.xc-rate-freshness');
  expect(lines, 'one p.xc-rate-freshness on the page').toHaveLength(1);
  return lines[0]!;
}

beforeEach(() => {
  setLang('th');
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('offline in tests'))));
});

afterEach(async () => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
  await settle();
});

describe('the kiosk currencies page shows the publication freshness once (RF-3)', () => {
  it('KioskCurrenciesScreen.tsx imports the shared RateFreshness, and declares no freshness markup of its own', () => {
    const src = sources['./KioskCurrenciesScreen.tsx'];
    expect(src, 'KioskCurrenciesScreen.tsx').toBeTypeOf('string');
    expect(src).toMatch(/import\s*\{[^}]*\bRateFreshness\b[^}]*\}\s*from\s*['"][./]*shared\/ui\/RateFreshness['"]/);
    expect(src).not.toMatch(/xc-rate-stale|xc-rate-freshness/);
    expect(src).not.toMatch(/real[\s-]?time|เรียลไทม์/i);
  });

  it('renders RateFreshness once per page, not once per card', async () => {
    const el = await mount(fake([answer({ stale: false, sourceDate: '2026-10-09', publishedAt: PUBLISHED_AT })]).api);
    expect(el.querySelectorAll('form.quote'), 'the three cards').toHaveLength(CURRENCIES.length);
    const p = theLine(el);
    expect(p.closest('form.quote'), 'not inside a card').toBeNull();
  });

  it('a fresh publication: no badge, the source date and the published time', async () => {
    const el = await mount(fake([answer({ stale: false, sourceDate: '2026-10-09', publishedAt: PUBLISHED_AT })]).api);
    const p = theLine(el);
    expect(p.querySelector('.xc-rate-stale')).toBeNull();
    expect(text(p)).toContain(fill(key('rate.sourceDate'), { date: '2026-10-09' }));
    expect(text(p)).toContain(fill(key('rate.publishedAt'), { at: at(PUBLISHED_AT) }));
  });

  it('a stale publication (its only old reference on a MANUAL side): the badge and the oldest date', async () => {
    const el = await mount(fake([answer({ stale: true, sourceDate: '2026-10-06', publishedAt: PUBLISHED_AT })]).api);
    const p = theLine(el);
    const badge = p.querySelector('span.xc-rate-stale');
    expect(badge, 'the stale badge').not.toBeNull();
    expect(text(badge!)).toBe(key('rate.stale'));
    expect(text(p)).toContain(fill(key('rate.sourceDate'), { date: '2026-10-06' }));
  });

  it('before the first publication: no reference text and no published time', async () => {
    const el = await mount(fake([answer({ stale: false, sourceDate: null, publishedAt: null })]).api);
    const p = theLine(el);
    expect(text(p)).toContain(key('rate.noSourceDate'));
    const prefix = key('rate.publishedAt').split('{at}')[0]!.trim();
    expect(text(p)).not.toContain(prefix);
    expect(text(p)).not.toContain('Invalid Date');
  });

  it('after a switch is ticked, the freshness shown is the view read again, still once', async () => {
    const f = fake([
      answer({ stale: false, sourceDate: '2026-10-09', publishedAt: PUBLISHED_AT }, 7),
      answer({ stale: true, sourceDate: '2026-10-06', publishedAt: PUBLISHED_AT }, 8),
    ]);
    const el = await mount(f.api);
    expect(theLine(el).querySelector('.xc-rate-stale'), 'fresh at first').toBeNull();
    const box = el.querySelector<HTMLInputElement>('form.quote input[type="checkbox"]')!;
    box.checked = true;
    box.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    expect(f.views.length, 'the view was read again').toBeGreaterThanOrEqual(2);
    const p = theLine(el);
    expect(p.querySelector('span.xc-rate-stale'), 'stale after the re-read').not.toBeNull();
    expect(text(p)).toContain(fill(key('rate.sourceDate'), { date: '2026-10-06' }));
  });
});
