// #2513: Admin cash Profit summary — the "computed at" alert is formatted in Asia/Bangkok
// regardless of the device timezone.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import type { ProfitSummary } from '../../../../shared/api/generated/profit_summary';
import { setAudience, setLang } from '../../../../shared/i18n';
import { formatMoney } from '../../../../shared/ui/MoneyText';
import { ProfitSummaryPanel } from './ProfitSummaryPanel';
import * as table from './i18n';

const PROFIT = '/api/v3/compute/profit_summary';
const BRANCH = 'B2';
const SHIFT = 'S-0042';
const SYSTEM_TIME_ISO = '2026-10-15T18:30:00.000Z';

const TYPED = {
  realized: '1000',
  foreignValue: '5000',
  foreignCost: '5200',
  netAssets: '20000',
  capital: '15000',
  reserve: '2000',
  cash: '8000',
  cashMinimum: '5000',
} as const;

const FIELD_KEYS = [
  'realized',
  'foreignValue',
  'foreignCost',
  'netAssets',
  'capital',
  'reserve',
  'cash',
  'cashMinimum',
] as const;

const ANSWER: ProfitSummary = {
  realized: '1234567.89',
  unrealized: '-200.01',
  withdrawable: '42.00',
};

interface SentCall {
  path: string;
  method: string;
  body: Record<string, unknown>;
}

function norm(s: string): string {
  return s.replace(/\s+/g, ' ').trim();
}

describe('#2513: ProfitSummaryPanel formats the computed-at alert in Asia/Bangkok', () => {
  let root: HTMLDivElement | undefined;
  let dispose: (() => void) | undefined;
  let calls: SentCall[];

  beforeEach(() => {
    vi.stubEnv('TZ', 'UTC');
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(SYSTEM_TIME_ISO));
    setAudience('staff');
    setLang('th');
    calls = [];

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const body =
        typeof init?.body === 'string' ? (JSON.parse(init.body) as Record<string, unknown>) : {};
      calls.push({ path: url.pathname, method, body });

      if (method === 'POST' && url.pathname === PROFIT) {
        return new Response(JSON.stringify(ANSWER), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }
      return new Response(JSON.stringify({}), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    root = undefined;
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
    setAudience('staff');
    setLang('th');
  });

  async function settle(): Promise<void> {
    for (let round = 0; round < 3; round++) {
      for (let i = 0; i < 5; i++) await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mountAndSubmit(): Promise<HTMLDivElement> {
    dispose?.();
    root?.remove();
    root = document.createElement('div');
    document.body.appendChild(root);
    const api = new ApiClient();
    dispose = render(() => <ProfitSummaryPanel api={api} branch={BRANCH} shiftId={SHIFT} />, root);
    await settle();

    const numberInputs = [...root.querySelectorAll<HTMLInputElement>('input[type="number"]')];
    expect(numberInputs).toHaveLength(FIELD_KEYS.length);
    FIELD_KEYS.forEach((key, idx) => {
      const input = numberInputs[idx]!;
      input.value = TYPED[key];
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new Event('change', { bubbles: true }));
    });

    const checkbox = root.querySelector<HTMLInputElement>('input[type="checkbox"]');
    expect(checkbox).not.toBeNull();
    if (!checkbox!.checked) checkbox!.click();

    const submitBtn = root.querySelector<HTMLButtonElement>('form button');
    expect(submitBtn).not.toBeNull();
    submitBtn!.click();
    await settle();
    return root;
  }

  function alertText(el: ParentNode): string {
    const alert = el.querySelector<HTMLElement>('[role="alert"]');
    expect(alert).not.toBeNull();
    return norm(alert!.textContent ?? '');
  }

  it("row 1: computed at system time 2026-10-15T18:30:00Z with device TZ UTC shows 16 Oct 01:30 (Bangkok) in the screen's locale (th and en)", async () => {
    vi.stubEnv('TZ', 'UTC');
    expect(new Date(SYSTEM_TIME_ISO).getHours()).toBe(18);

    setLang('th');
    const elTh = await mountAndSubmit();
    const thAlert = alertText(elTh);
    expect(thAlert).toBe('คำนวณเมื่อ 16/10/2569 01:30:00 · ไม่ได้บันทึกหรือถอนเงิน');
    expect(thAlert).toContain('01:30:00');
    expect(thAlert).not.toContain('18:30');
    expect(thAlert).not.toContain('15/10');

    setLang('en');
    const elEn = await mountAndSubmit();
    const enAlert = alertText(elEn);
    expect(enAlert).toBe('Calculated at 16/10/2026, 01:30:00 · nothing recorded or withdrawn');
    expect(enAlert).toContain('16/10/2026');
    expect(enAlert).toContain('01:30:00');
    expect(enAlert).not.toContain('18:30');
    expect(enAlert).not.toContain('15/10');
  });

  it('row 2: computed at system time 2026-10-15T18:30:00Z with device TZ Asia/Bangkok shows the same text as device TZ UTC', async () => {
    setLang('th');

    vi.stubEnv('TZ', 'UTC');
    expect(new Date(SYSTEM_TIME_ISO).getHours()).toBe(18);
    const utcEl = await mountAndSubmit();
    const utcText = alertText(utcEl);

    vi.stubEnv('TZ', 'Asia/Bangkok');
    expect(new Date(SYSTEM_TIME_ISO).getHours()).toBe(1);
    const bkkEl = await mountAndSubmit();
    const bkkText = alertText(bkkEl);

    expect(bkkText).toBe('คำนวณเมื่อ 16/10/2569 01:30:00 · ไม่ได้บันทึกหรือถอนเงิน');
    expect(utcText).toBe(bkkText);
  });

  it('row 3: profit computation sends branch and figures, updates the three cards, and formats the computed-at alert in Bangkok time under TZ=UTC', async () => {
    vi.stubEnv('TZ', 'UTC');
    setLang('th');

    const el = await mountAndSubmit();

    expect(calls).toEqual([
      {
        path: PROFIT,
        method: 'POST',
        body: { branch: BRANCH, ...TYPED },
      },
    ]);

    const allText = norm(el.textContent ?? '');
    expect(allText).toContain(`${table.th.cardRealized}${formatMoney(ANSWER.realized)} THB`);
    expect(allText).toContain(`${table.th.cardUnrealized}${formatMoney(ANSWER.unrealized)} THB`);
    expect(allText).toContain(`${table.th.cardWithdrawable}${formatMoney(ANSWER.withdrawable)} THB`);
    expect(alertText(el)).toBe('คำนวณเมื่อ 16/10/2569 01:30:00 · ไม่ได้บันทึกหรือถอนเงิน');
  });
});
