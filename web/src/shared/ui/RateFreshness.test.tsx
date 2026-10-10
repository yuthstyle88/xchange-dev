// RF-2 (#773; design docs/design/blueprint/rates.md §4.10, PR 843): every rate display shows whether its rates are
// stale, the reference's source date and the publication's time, separately, through ONE shared component. No display
// says "real time" (Blueprint §05, rates.md §4.8).
// Written before the implementation by the ticket author (claude/83f682). Do not edit, delete or weaken.
//
// The API this test pins, in `web/src/shared/ui/RateFreshness.tsx`:
//
//   export function RateFreshness(props: {
//     stale: boolean;
//     sourceDate: string | null | undefined;   // 'YYYY-MM-DD', shown as given
//     publishedAt: string | null | undefined;  // RFC 3339, shown in Asia/Bangkok
//   }): JSX.Element;
//
// It renders one `p.xc-rate-freshness`; when `stale`, a `span.xc-rate-stale[role=status]` with `rate.stale` first.
// The texts are shared keys under a new shared prefix `rate` (web/src/shared/i18n/{th,en,zh}.ts; `SHARED_PREFIXES` in
// web/src/shared/i18n/index.ts gains 'rate'):
//
//   'rate.stale':        'เรทอ้างอิงเก่า'
//   'rate.sourceDate':   'อ้างอิงวันที่ {date}'
//   'rate.noSourceDate': 'ไม่มีเรทอ้างอิง'
//   'rate.publishedAt':  'ประกาศเมื่อ {at}'
//
// `{at}` is `new Date(publishedAt).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' })`, the same call Admin Rates
// uses for its history (`web/src/screens/admin/rates/Rates.tsx:27-30`).
import { afterEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { fill, locale, setLang } from '../i18n';
import { en } from '../i18n/en';
import { th } from '../i18n/th';
import { zh } from '../i18n/zh';
import { RateFreshness } from './RateFreshness';

const KEYS = ['rate.stale', 'rate.sourceDate', 'rate.noSourceDate', 'rate.publishedAt'] as const;
const PUBLISHED = '2026-10-10T03:15:00Z';

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  setLang('th');
});

function mount(props: { stale: boolean; sourceDate: string | null | undefined; publishedAt: string | null | undefined }) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <RateFreshness {...props} />, host);
  return host;
}

function text(host: HTMLElement): string {
  return (host.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function at(iso: string): string {
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

/** `th[key]` read through a string index, so the file runs before the keys exist (they then read undefined). */
function tableText(table: Record<string, string>, key: string): string | undefined {
  return table[key];
}

describe('RateFreshness texts (RF-2)', () => {
  it('the shared th table carries the four rate.* keys, in Thai, with their placeholders', () => {
    expect(tableText(th, 'rate.stale')).toBe('เรทอ้างอิงเก่า');
    expect(tableText(th, 'rate.sourceDate')).toBe('อ้างอิงวันที่ {date}');
    expect(tableText(th, 'rate.noSourceDate')).toBe('ไม่มีเรทอ้างอิง');
    expect(tableText(th, 'rate.publishedAt')).toBe('ประกาศเมื่อ {at}');
  });

  it('en and zh carry all four keys, not in Thai, with the same placeholders', () => {
    for (const table of [en, zh] as Record<string, string>[]) {
      for (const key of KEYS) {
        const value = table[key];
        expect(value, key).toBeTypeOf('string');
        expect(value!.trim().length, key).toBeGreaterThan(0);
        expect(value, key).not.toMatch(/[฀-๿]/);
      }
      expect(table['rate.sourceDate']).toMatch(/\{date\}/);
      expect(table['rate.publishedAt']).toMatch(/\{at\}/);
    }
  });

  it('no rate text says "real time" or "live", in any language (§05)', () => {
    for (const table of [th, en, zh] as Record<string, string>[]) {
      for (const key of KEYS) {
        expect(table[key] ?? '', key).not.toMatch(/real[\s-]?time|\blive\b|เรียลไทม์|ล่าสุดแบบสด|实时/i);
      }
    }
  });
});

describe('RateFreshness (RF-2)', () => {
  it('stale: the badge first, in a status span, then both dates', () => {
    const host = mount({ stale: true, sourceDate: '2026-10-08', publishedAt: PUBLISHED });
    const p = host.querySelector('p.xc-rate-freshness');
    expect(p, 'one p.xc-rate-freshness').not.toBeNull();
    expect(host.querySelectorAll('p.xc-rate-freshness')).toHaveLength(1);
    const badge = p!.querySelector('span.xc-rate-stale');
    expect(badge, 'the stale badge').not.toBeNull();
    expect(badge!.getAttribute('role')).toBe('status');
    expect((badge!.textContent ?? '').trim()).toBe(th['rate.stale' as keyof typeof th]);
    expect(text(host).startsWith(th['rate.stale' as keyof typeof th] as string)).toBe(true);
    expect(text(host)).toContain(fill(th['rate.sourceDate' as keyof typeof th] as string, { date: '2026-10-08' }));
    expect(text(host)).toContain(fill(th['rate.publishedAt' as keyof typeof th] as string, { at: at(PUBLISHED) }));
  });

  it('not stale: no badge at all, the two dates still shown', () => {
    const host = mount({ stale: false, sourceDate: '2026-10-10', publishedAt: PUBLISHED });
    expect(host.querySelector('.xc-rate-stale')).toBeNull();
    expect(text(host)).not.toContain(th['rate.stale' as keyof typeof th] as string);
    expect(text(host)).toContain(fill(th['rate.sourceDate' as keyof typeof th] as string, { date: '2026-10-10' }));
    expect(text(host)).toContain(fill(th['rate.publishedAt' as keyof typeof th] as string, { at: at(PUBLISHED) }));
  });

  it('the source date and the published time are separate texts, never one merged date', () => {
    const host = mount({ stale: false, sourceDate: '2026-10-08', publishedAt: PUBLISHED });
    const s = text(host);
    const source = fill(th['rate.sourceDate' as keyof typeof th] as string, { date: '2026-10-08' });
    const published = fill(th['rate.publishedAt' as keyof typeof th] as string, { at: at(PUBLISHED) });
    expect(s.indexOf(source)).toBeGreaterThanOrEqual(0);
    expect(s.indexOf(published)).toBeGreaterThan(s.indexOf(source));
  });

  it('no reference: rate.noSourceDate instead of a date', () => {
    const host = mount({ stale: false, sourceDate: null, publishedAt: PUBLISHED });
    expect(text(host)).toContain(th['rate.noSourceDate' as keyof typeof th] as string);
    expect(text(host)).not.toContain('{date}');
    expect(text(host)).not.toContain('null');
  });

  it('no publication: the published time is left out, not printed empty', () => {
    for (const publishedAt of [null, undefined]) {
      const host = mount({ stale: false, sourceDate: '2026-10-08', publishedAt });
      const prefix = (th['rate.publishedAt' as keyof typeof th] as string | undefined)?.split('{at}')[0]?.trim() ?? '';
      expect(prefix.length, 'rate.publishedAt has text before {at}').toBeGreaterThan(0);
      expect(text(host)).not.toContain(prefix);
      expect(text(host)).not.toContain('Invalid Date');
      dispose?.();
      dispose = undefined;
      document.body.innerHTML = '';
    }
  });

  it('follows setLang: English texts and the English locale for the time', () => {
    setLang('en');
    const host = mount({ stale: true, sourceDate: '2026-10-08', publishedAt: PUBLISHED });
    expect(text(host)).toContain(en['rate.stale' as keyof typeof en] as string);
    expect(text(host)).toContain(fill(en['rate.publishedAt' as keyof typeof en] as string, { at: at(PUBLISHED) }));
    expect(text(host)).not.toMatch(/[฀-๿]/);
  });

  it('reacts when its props change (a refreshed view turns stale)', async () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const [stale, setStale] = createSignal(false);
    dispose = render(() => <RateFreshness stale={stale()} sourceDate="2026-10-08" publishedAt={PUBLISHED} />, host);
    expect(host.querySelector('.xc-rate-stale')).toBeNull();
    setStale(true);
    await Promise.resolve();
    expect(host.querySelector('.xc-rate-stale')).not.toBeNull();
  });
});
