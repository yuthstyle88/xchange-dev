// #2514: ShiftValuationPanel formats the "computed at" time in Asia/Bangkok regardless of device TZ (§8, §9).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import type { ShiftValuation, ShiftValuationCurrency } from '../../../../shared/api/generated/shift_valuation';
import { setLang } from '../../../../shared/i18n';
import { formatMoney } from '../../../../shared/ui/MoneyText';
import { ShiftValuationPanel } from './ShiftValuationPanel';

const VALUATION = '/api/v3/compute/shift_valuation';
const BRANCH = 'B2';
const SHIFT = 'S-0042';
const COMPUTED_ISO = '2026-10-15T18:30:00Z';

function listed(): ShiftValuationCurrency[] {
  return [
    { currency: 'THB', opening: '10000.00', current: '6600.00', needsRate: false },
    { currency: 'USD', opening: '100.00', current: '200.00', needsRate: true },
  ];
}

function computed(): ShiftValuation {
  return {
    currencies: [
      { currency: 'THB', opening: '10000.00', current: '6600.00', needsRate: false },
      { currency: 'USD', opening: '100.00', current: '210.00', needsRate: true },
    ],
    result: {
      rows: [
        { currency: 'THB', rate: '1.000000', value: '6600.00' },
        { currency: 'USD', rate: '34.500000', value: '7245.00' },
      ],
      opening: '13450.00',
      current: '13845.00',
      movements: '500.00',
      difference: '395.00',
      adjusted: '-105.00',
    },
  };
}

async function settle(): Promise<void> {
  for (let round = 0; round < 3; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

function norm(s: string): string {
  return s.replace(/\s+/g, ' ').trim();
}

function setInput(el: HTMLInputElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('ShiftValuationPanel computedAt Bangkok timezone (#2514 §8, §9)', () => {
  let root: HTMLElement | undefined;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    vi.stubEnv('TZ', 'UTC');
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(COMPUTED_ISO));
    setLang('th');

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const url = new URL(raw, 'http://localhost');
        const body =
          typeof init?.body === 'string' ? (JSON.parse(init.body) as Record<string, unknown>) : {};
        if (url.pathname === VALUATION) {
          const payload = body.rates == null ? { currencies: listed() } : computed();
          return new Response(JSON.stringify(payload), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          });
        }
        return new Response(JSON.stringify({}), { status: 404 });
      }),
    );
  });

  afterEach(async () => {
    await settle();
    dispose?.();
    dispose = undefined;
    root?.remove();
    root = undefined;
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
    setLang('th');
  });

  async function mountAndSubmit(): Promise<string> {
    dispose?.();
    root?.remove();
    root = document.createElement('div');
    document.body.appendChild(root);
    const api = new ApiClient();
    dispose = render(() => <ShiftValuationPanel api={api} branch={BRANCH} shiftId={SHIFT} />, root);
    await settle();

    const dateInput = root.querySelector<HTMLInputElement>('input[type="date"]')!;
    const rateInput = root.querySelector<HTMLInputElement>('input[type="number"]')!;
    setInput(dateInput, '2026-10-15');
    setInput(rateInput, '34.5');

    root.querySelector<HTMLButtonElement>('form button.primary')!.click();
    await settle();

    const live = root.querySelector<HTMLElement>('[aria-live="polite"]')!;
    return norm(live.textContent ?? '');
  }

  it("row 1: Computed at 2026-10-15T18:30:00Z, device TZ UTC -> Line shows 16 Oct 01:30 (Bangkok) in the screen's locale (th and en)", async () => {
    vi.stubEnv('TZ', 'UTC');

    setLang('th');
    const thText = await mountAndSubmit();
    // In Bangkok (UTC+7), 2026-10-15T18:30:00Z is 16/10/2569 01:30:00 (not 15/10/2569 18:30:00)
    expect(thText).toContain('คำนวณเมื่อ 16/10/2569');
    expect(thText).toContain('01:30:00');
    expect(thText).not.toContain('15/10/2569');
    expect(thText).not.toContain('18:30');

    setLang('en');
    const enText = await mountAndSubmit();
    // In Bangkok (UTC+7), 2026-10-15T18:30:00Z is 16/10/2026, 01:30:00 (not 15/10/2026, 18:30:00)
    expect(enText).toContain('Calculated at 16/10/2026, 01:30:00');
    expect(enText).not.toContain('15/10/2026');
    expect(enText).not.toContain('18:30');
  });

  it('row 2: Same with device TZ Asia/Bangkok -> Same text as device TZ UTC', async () => {
    setLang('th');
    vi.stubEnv('TZ', 'Asia/Bangkok');
    const thBangkok = await mountAndSubmit();

    vi.stubEnv('TZ', 'UTC');
    const thUtc = await mountAndSubmit();

    expect(thBangkok).toContain('คำนวณเมื่อ 16/10/2569');
    expect(thBangkok).toContain('01:30:00');
    expect(thBangkok).toBe(thUtc);

    setLang('en');
    vi.stubEnv('TZ', 'Asia/Bangkok');
    const enBangkok = await mountAndSubmit();

    vi.stubEnv('TZ', 'UTC');
    const enUtc = await mountAndSubmit();

    expect(enBangkok).toContain('Calculated at 16/10/2026, 01:30:00');
    expect(enBangkok).toBe(enUtc);
  });

  it('row 3 & edge case (§9): Valuation itself is kept while fake clock + TZ stub (America/New_York) still renders Bangkok time', async () => {
    vi.stubEnv('TZ', 'America/New_York');
    setLang('th');

    const rendered = await mountAndSubmit();
    expect(rendered).toContain('คำนวณเมื่อ 16/10/2569');
    expect(rendered).toContain('01:30:00');
    expect(rendered).not.toContain('15/10/2569');
    expect(rendered).not.toContain('14:30');

    // Valuation table and totals stay intact
    expect(rendered).toContain(formatMoney('7245.00'));
    expect(rendered).toContain(formatMoney('13845.00'));
    expect(rendered).toContain(formatMoney('-105.00'));
  });
});
