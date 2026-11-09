// CE-1 (round 13, parent #564): staff pages printed Buddhist-era stamps ("8/11/2569 02:25:47") beside CE dates
// ("2026-11-07"). The cause is not one formatter: F55 already gave web one (`formatDateTimeInZone` /
// `formatBangkokDateTime` in ./bangkok-time.ts, `dd/mm/yyyy HH:mm`, CE year, the same in every language), and the
// stamps that stayed Buddhist are callers that bypass it with `toLocaleString(locale())` or `'th-TH'`
// (th-TH's default calendar is Buddhist).
//
// This file pins two things:
//   1. RateFreshness (the "ประกาศเมื่อ" line on /staff rates, admin Rates, kiosk-currencies, customer home) prints the
//      shared shape in both languages.
//   2. No source file outside ALLOWED formats a date with `toLocale*String(locale…)`, `toLocale*String('th-TH')`,
//      `toLocale*String()` or `Intl.DateTimeFormat(locale…|'th-TH')`. ALLOWED lists the callers left for other
//      tickets; it may only shrink: a listed file that no longer matches fails too, so whoever fixes it removes it.
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../i18n';
import { RateFreshness } from '../ui/RateFreshness';

const BYPASS =
  /toLocale(?:Date|Time)?String\(\s*(?:locale\b|['"]th-TH['"]|\))|Intl\.DateTimeFormat\(\s*(?:locale\b|['"]th-TH['"])/;

/** Paths relative to web/src. Each still bypasses the shared formatter at 13598bc5 and is owned elsewhere. */
const ALLOWED = [
  'screens/admin/approvals/Approvals.tsx',
  'screens/admin/booking-slots/BookingSlots.tsx',
  'screens/admin/pricing-formula/PricingFormula.tsx',
  'screens/admin/rates/Rates.tsx',
  'screens/admin/rates/RatesHistory.tsx',
  'screens/customer/queue/RequestQr.tsx',
  'screens/customer/rate-alerts/AlertFacts.tsx',
  'screens/customer/rate-alerts/RateAlerts.tsx',
  'screens/customer/rate-display/board/BoardClock.tsx',
];

const sources = import.meta.glob<string>(['/src/**/*.ts', '/src/**/*.tsx', '!/src/**/*.test.*'], {
  query: '?raw',
  import: 'default',
  eager: true,
});

function rel(key: string): string {
  return key.replace(/^\/src\//, '');
}

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  setLang('th');
});

function freshnessText(): string {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <RateFreshness stale={false} sourceDate="2026-10-09" publishedAt="2026-10-10T03:15:00Z" />,
    host,
  );
  return (host.textContent ?? '').replace(/\s+/g, ' ').trim();
}

describe('CE-1: staff timestamps use the shared CE formatter', () => {
  it('RateFreshness prints the published time as 10/10/2026 10:15 in Thai (no 2569, no seconds)', () => {
    setLang('th');
    const t = freshnessText();
    expect(t).toContain('10/10/2026 10:15');
    expect(t).not.toContain('2569');
    expect(t).not.toContain('10:15:00');
  });

  it('RateFreshness prints the same 10/10/2026 10:15 in English', () => {
    setLang('en');
    const t = freshnessText();
    expect(t).toContain('10/10/2026 10:15');
    expect(t).not.toContain('10:15:00');
  });

  it('the glob sees the sources (guard is not vacuous)', () => {
    const keys = Object.keys(sources).map(rel);
    expect(keys).toContain('shared/format/bangkok-time.ts');
    expect(keys).toContain('screens/staff-report/StaffReport.tsx');
    expect(keys.some((k) => k.includes('.test.'))).toBe(false);
  });

  it('StaffReport, QueuePanel and RateFreshness format through ../format/bangkok-time', () => {
    for (const f of ['screens/staff-report/StaffReport.tsx', 'shared/ui/QueuePanel.tsx', 'shared/ui/RateFreshness.tsx']) {
      const src = sources[`/src/${f}`];
      expect(src, `${f} is read`).toBeTypeOf('string');
      expect(src, `${f} has no toLocale*/th-TH date call`).not.toMatch(BYPASS);
      expect(src, `${f} imports the shared formatter`).toMatch(/from '[./]+(?:shared\/)?format\/bangkok-time'/);
    }
  });

  it('no file outside ALLOWED formats a date with toLocale*String(locale | th-TH | nothing)', () => {
    const offenders = Object.entries(sources)
      .filter(([, src]) => BYPASS.test(src))
      .map(([k]) => rel(k))
      .filter((f) => !f.startsWith('test-support/') && !f.includes('/testing/'))
      .sort();
    expect(offenders.filter((f) => !ALLOWED.includes(f)), 'new bypassing callers').toEqual([]);
  });

  it('ALLOWED only shrinks: every listed file still bypasses (remove it here once fixed)', () => {
    const fixed = ALLOWED.filter((f) => {
      const src = sources[`/src/${f}`];
      return typeof src !== 'string' || !BYPASS.test(src);
    });
    expect(fixed).toEqual([]);
  });
});
