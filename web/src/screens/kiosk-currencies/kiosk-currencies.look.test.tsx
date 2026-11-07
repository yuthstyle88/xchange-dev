// OL-6 wave B kiosk-currencies-look: SHOW/ASK/HIDE as a segmented control, KB table headers, card surface. Written
// before the implementation by the test-writer claude/94389a6b on the sub-controller claude/1cc51615's order. Do not
// edit, delete or weaken.
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

type Rule = { selector: string; decls: Map<string, string> };

function parseRules(css: string): Rule[] {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const rules: Rule[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    const decls = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const idx = d.indexOf(':');
      if (idx > 0) decls.set(d.slice(0, idx).trim().toLowerCase(), d.slice(idx + 1).trim());
    }
    rules.push({ selector: m[1]!.trim().replace(/\s+/g, ' '), decls });
  }
  return rules;
}

const USD = makeKioskCurrency({
  code: 'USD',
  name: 'ดอลลาร์',
  nameEn: 'Dollar',
  countries: [],
  buy: '30.0000',
  sell: '31.0000',
  unitSize: '1',
  updated: '2026-10-27T00:00:00.000Z',
  partnerName: '',
  partnerCost: null,
  enabled: true,
  display: 'SHOW',
  quoteEnabled: true,
  kbank: [],
});

function fakeApi(): Pick<ApiClient, 'view' | 'post'> {
  const answer = makeViewAnswer(
    makeKioskCurrencies({
      currencies: [USD],
      kbank: null,
      freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
    }),
    { version: 1, changed: true },
  );
  return {
    view<T>(): Promise<ViewAnswer<T>> {
      return Promise.resolve(structuredClone(answer) as never);
    },
    post: () => Promise.resolve({ saved: true } as never),
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

async function mount(): Promise<void> {
  dispose = render(() => <KioskCurrenciesScreen api={fakeApi()} branch="B1" />, host);
  await vi.waitFor(() => expect(host.querySelector('form.quote')).not.toBeNull());
}

const segments = () => [...host.querySelectorAll<HTMLLabelElement>('form.quote .kc-segments label.kc-segment')];

describe('kiosk-currencies look (OL-6 wave B)', () => {
  let screen: string;
  let rules: Rule[];
  const rule = (selector: string) => rules.find((r) => r.selector === selector);

  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    screen = fs.readFileSync(here + 'KioskCurrenciesScreen.tsx', 'utf8');
    rules = parseRules(fs.readFileSync(here + 'kiosk-currencies.css', 'utf8'));
  });

  beforeEach(() => {
    setLang('th');
    vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('offline in tests'))));
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

  it('KL-1: three segments, each a label around its radio, in SHOW, ASK, HIDE order', async () => {
    await mount();
    expect(segments().map((s) => s.querySelector<HTMLInputElement>('input[type="radio"]')?.value)).toEqual([
      'SHOW',
      'ASK',
      'HIDE',
    ]);
  });

  it('KL-2: Thai labels, no English word left', async () => {
    await mount();
    expect(segments().map((s) => s.textContent?.trim())).toEqual([th.displayShow, th.displayAsk, th.displayHide]);
    expect([th.displayShow, th.displayAsk, th.displayHide]).toEqual(['แสดงเรท', 'ติดต่อพนักงาน', 'ไม่แสดง']);
  });

  it('KL-3: English labels follow the language', async () => {
    setLang('en');
    await mount();
    expect(segments().map((s) => s.textContent?.trim())).toEqual([en.displayShow, en.displayAsk, en.displayHide]);
    expect([en.displayShow, en.displayAsk, en.displayHide]).toEqual(['Show rate', 'Contact staff', 'Hide']);
  });

  it('KL-4: the group keeps its radiogroup role and name, and its caption label (KioskCurrenciesScreen.test.tsx finds cards by it)', async () => {
    await mount();
    const group = host.querySelector('form.quote [role="radiogroup"]');
    expect(group?.getAttribute('aria-label')).toBe(th.enabled);
    expect(group?.querySelector('.kc-display-label')?.tagName).toBe('LABEL');
    expect(group?.querySelector('.kc-display-label')?.textContent).toBe(th.enabled);
  });

  it('KL-5: the segment bar is three equal columns with one line around it', () => {
    const r = rule('.xc-kiosk .xc-shell-main .kc-segments');
    expect(r?.decls.get('display')).toBe('grid');
    expect(r?.decls.get('grid-template-columns')).toBe('repeat(3, 1fr)');
    expect(r?.decls.get('border')).toBe('1px solid var(--line)');
    expect(r?.decls.get('overflow')).toBe('hidden');
  });

  it('KL-6: a segment is a 44px centred target that does not inherit the label margin', () => {
    const r = rule('.xc-kiosk .xc-shell-main .kc-segment');
    expect(r?.decls.get('display')).toBe('flex');
    expect(r?.decls.get('min-height')).toBe('44px');
    expect(r?.decls.get('margin')).toBe('0');
    expect(r?.decls.get('position')).toBe('relative');
    expect(r?.decls.get('justify-content')).toBe('center');
    expect(r?.decls.get('align-items')).toBe('center');
  });

  it('KL-7: the native radio is hidden but stays focusable', () => {
    const r = rule('.xc-kiosk .xc-shell-main .kc-segment input');
    expect(r?.decls.get('position')).toBe('absolute');
    expect(r?.decls.get('opacity')).toBe('0');
    expect(r?.decls.get('width')).toBe('1px');
    expect(r?.decls.get('min-height')).toBe('0');
    expect(r?.decls.get('display')).toBeUndefined();
  });

  it('KL-8: the hide rule comes after the coarse-pointer field rule, so it wins on touch screens', () => {
    const coarse = rules.findIndex((r) => r.selector.includes('input:not([type="checkbox"])'));
    const hide = rules.findIndex((r) => r.selector === '.xc-kiosk .xc-shell-main .kc-segment input');
    expect(coarse).toBeGreaterThanOrEqual(0);
    expect(hide).toBeGreaterThan(coarse);
  });

  it('KL-9: checked is brand fill', () => {
    const r = rule('.xc-kiosk .xc-shell-main .kc-segment:has(input:checked)');
    expect(r?.decls.get('background')).toBe('var(--xc-brand)');
    expect(r?.decls.get('color')).toBe('var(--xc-on-brand)');
  });

  it('KL-10: keyboard focus shows on the segment', () => {
    expect(rule('.xc-kiosk .xc-shell-main .kc-segment:has(input:focus-visible)')?.decls.get('outline')).toBe(
      '3px solid var(--xc)',
    );
  });

  it('KL-11: disabled while saving is solid grey, checked or not', () => {
    const r = rule('.xc-kiosk .xc-shell-main .kc-segment:has(input:disabled)');
    expect(r?.decls.get('background')).toBe('var(--xc-gray-100)');
    expect(r?.decls.get('color')).toBe('var(--xc-gray-600)');
    expect(r?.decls.get('cursor')).toBe('not-allowed');
    const checked = rules.findIndex((x) => x.selector === '.xc-kiosk .xc-shell-main .kc-segment:has(input:checked)');
    const disabled = rules.findIndex((x) => x.selector === '.xc-kiosk .xc-shell-main .kc-segment:has(input:disabled)');
    expect(disabled).toBeGreaterThan(checked);
  });

  it('KL-12: rate rows put the amount apart from the caption', () => {
    const r = rule('.xc-kiosk .xc-shell-main .rate-side');
    expect(r?.decls.get('display')).toBe('flex');
    expect(r?.decls.get('justify-content')).toBe('space-between');
    expect(r?.decls.get('gap')).toBe('12px');
  });

  it('KL-13: KB headers stay on one line', () => {
    expect(rule('.xc-kiosk .xc-shell-main .kbank-box th')?.decls.get('white-space')).toBe('nowrap');
  });

  it('KL-14: the card is the shared card part', async () => {
    expect(screen).toContain("import '../../shared/ui/card.css';");
    await mount();
    expect(host.querySelector('form.quote')?.classList.contains('card')).toBe(true);
  });

  it('KL-15: .quote no longer copies the card values, so they cannot drift', () => {
    const r = rule('.xc-kiosk .xc-shell-main .quote');
    expect(r?.decls.get('margin')).toBe('20px 0');
    for (const copied of ['background', 'border', 'border-radius', 'padding']) {
      expect(r?.decls.has(copied), copied).toBe(false);
    }
  });
});
