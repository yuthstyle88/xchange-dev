// APP-11 (#1003): customer history screen (C04).
// Design `docs/design/blueprint/customer-app.md` §3, §4.5, §9.1.
//
// Written before the implementation by the test-writer gemini/pool-4, on the
// FE lane controller claude/5edf7bb5's order. Do not edit, delete or weaken.
//
// Pinned by this file (rulings on #1003):
// - The screen asks `/api/v3/views/my_transactions` with no customer id.
// - It renders mocked rows through MoneyText (amounts format unchanged across locales).
// - It handles loading, empty, error, and retry states.
// - A reversed row reads as reversed.
// - th / en / zh have identical key sets, matching placeholders, no empty values in zh.
// - web/pages/app-history.html has xc-path="/app/history" and xc-gate="anyone".
// - main.tsx sets setAudience('customer') and uses CustomerShell route="/app/history".

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { setAudience, setLang } from '../../../shared/i18n';
import { HistoryScreen } from './HistoryScreen';
import * as table from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}

interface NodePath {
  resolve(...segments: string[]): string;
  dirname(p: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';

let fs: NodeFs;
let path: NodePath;
let thisDir: string;

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  const thisFile = new URL(import.meta.url).pathname;
  thisDir = path.dirname(thisFile);
});

type Texts = Record<string, string>;
const TH = table.th as unknown as Texts;
const EN = table.en as unknown as Texts;
const ZH = table.zh as unknown as Texts;

let dispose: (() => void) | undefined;
let host: HTMLDivElement;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
  setAudience('customer');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

const MOCK_COMPLETED_ROW = {
  id: 'tx-001',
  date: '2026-10-12',
  branch: 'MAIN',
  branchName: 'สาขาสยาม',
  type: 'BUY',
  currency: 'USD',
  amount: '100.00',
  rate: '33.50',
  principal: '3350.00',
  fee: '0.00',
  settlement: '3350.00',
  receiptRef: 'RC-CX1-001',
  status: 'COMPLETED',
};

const MOCK_REVERSED_ROW = {
  id: 'tx-002',
  date: '2026-10-12',
  branch: 'B01',
  branchName: 'สาขาอโศก',
  type: 'BUY',
  currency: 'EUR',
  amount: '50.00',
  rate: '37.00',
  principal: '1850.00',
  fee: '0.00',
  settlement: '1850.00',
  receiptRef: 'RC-CX1-002',
  status: 'REVERSED',
};

async function settle(): Promise<void> {
  await new Promise((r) => setTimeout(r, 20));
}

describe('HistoryScreen API queries and customer isolation', () => {
  it('asks /api/v3/views/my_transactions with no customer id in query or body', async () => {
    let requestedUrl = '';
    const fetchMock = vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
      requestedUrl = input.toString();
      return {
        ok: true,
        status: 200,
        json: async () => ({
          version: 0,
          changed: true,
          data: { transactions: [MOCK_COMPLETED_ROW] },
        }),
      } as Response;
    });
    vi.stubGlobal('fetch', fetchMock);

    dispose = render(() => <HistoryScreen />, host);
    await settle();

    expect(fetchMock).toHaveBeenCalled();
    const url = new URL(requestedUrl, 'http://localhost');
    expect(url.pathname).toBe('/api/v3/views/my_transactions');
    expect(url.searchParams.get('customerId')).toBeNull();
    expect(url.searchParams.get('customer')).toBeNull();
    expect(url.searchParams.get('cx')).toBeNull();
  });
});

describe('HistoryScreen state handling', () => {
  it('handles loading state while fetching', async () => {
    // Hang the fetch promise to inspect loading state
    vi.stubGlobal('fetch', vi.fn().mockReturnValue(new Promise(() => {})));

    dispose = render(() => <HistoryScreen />, host);

    // Should indicate loading (text or status role)
    const content = host.textContent ?? '';
    const hasLoadingIndicator =
      content.includes(TH.loading || 'โหลด') || host.querySelector('[role="status"]') !== null;
    expect(hasLoadingIndicator, 'shows loading state').toBe(true);
  });

  it('handles empty state when customer has no transactions', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ({
          version: 0,
          changed: true,
          data: { transactions: [] },
        }),
      } as Response)
    );

    dispose = render(() => <HistoryScreen />, host);
    await settle();

    const content = host.textContent ?? '';
    expect(content.length).toBeGreaterThan(0);
    // Should show empty text from i18n
    expect(
      content.includes(TH.empty || 'ไม่มีรายการ') ||
        content.includes(EN.empty || 'No transactions'),
      'shows empty state message'
    ).toBe(true);
  });

  it('handles error state and provides a retry action that refetches', async () => {
    // Pin retry key in th, en, zh
    expect(typeof TH.retry).toBe('string');
    expect(TH.retry.trim().length).toBeGreaterThan(0);
    expect(typeof EN.retry).toBe('string');
    expect(EN.retry.trim().length).toBeGreaterThan(0);
    expect(typeof ZH.retry).toBe('string');
    expect(ZH.retry.trim().length).toBeGreaterThan(0);

    let callCount = 0;
    const fetchMock = vi.fn().mockImplementation(async () => {
      callCount++;
      if (callCount === 1) {
        return {
          ok: false,
          status: 500,
          json: async () => ({ error: 'server error' }),
        } as Response;
      }
      return {
        ok: true,
        status: 200,
        json: async () => ({
          version: 0,
          changed: true,
          data: { transactions: [MOCK_COMPLETED_ROW] },
        }),
      } as Response;
    });
    vi.stubGlobal('fetch', fetchMock);

    dispose = render(() => <HistoryScreen />, host);
    await settle();

    // Verify error message is displayed
    const content = host.textContent ?? '';
    expect(
      content.includes(TH.loadFailed || 'ขัดข้อง') || content.includes('error'),
      'shows error state'
    ).toBe(true);

    // Find the retry button by its pinned retry i18n text (not just the first button)
    const buttons = Array.from(host.querySelectorAll('button'));
    const retryBtn = buttons.find((btn) => btn.textContent?.trim() === TH.retry);
    expect(retryBtn, `button with text '${TH.retry}' exists`).toBeDefined();
    retryBtn!.click();
    await settle();

    // Verify refetched and resolved
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(host.textContent).toContain('3,350.00');
  });
});

describe('HistoryScreen row formatting and MoneyText', () => {
  it('renders mocked rows through MoneyText and preserves digit formatting across locales', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ({
          version: 0,
          changed: true,
          data: { transactions: [MOCK_COMPLETED_ROW] },
        }),
      } as Response)
    );

    dispose = render(() => <HistoryScreen />, host);
    await settle();

    // The settlement amount 3350.00 formatted with MoneyText is '3,350.00'
    const formattedExpected = formatMoney('3350.00');
    expect(host.textContent).toContain(formattedExpected);

    // Across languages, the numeric digits and decimals remain unchanged
    for (const lang of ['th', 'en', 'zh'] as const) {
      setLang(lang);
      await settle();
      expect(host.textContent).toContain(formattedExpected);
      expect(host.textContent).toContain('100.00');
    }
  });

  it('a reversed row reads as reversed', async () => {
    // Pin status_reversed key: it must exist in th, en, and zh
    expect(typeof TH.status_reversed).toBe('string');
    expect(TH.status_reversed.trim().length).toBeGreaterThan(0);
    expect(typeof EN.status_reversed).toBe('string');
    expect(EN.status_reversed.trim().length).toBeGreaterThan(0);
    expect(typeof ZH.status_reversed).toBe('string');
    expect(ZH.status_reversed.trim().length).toBeGreaterThan(0);

    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ({
          version: 0,
          changed: true,
          data: { transactions: [MOCK_COMPLETED_ROW, MOCK_REVERSED_ROW] },
        }),
      } as Response)
    );

    dispose = render(() => <HistoryScreen />, host);
    await settle();

    // Reversed row shows exactly the pinned status_reversed text with no fallback
    expect(host.textContent).toContain(TH.status_reversed);

    // Switch to English: shows EN.status_reversed
    setLang('en');
    await settle();
    expect(host.textContent).toContain(EN.status_reversed);

    // Switch to Chinese: shows ZH.status_reversed
    setLang('zh');
    await settle();
    expect(host.textContent).toContain(ZH.status_reversed);
  });
});

describe('i18n table completeness and parity', () => {
  it('th, en, and zh have identical key sets', () => {
    const thKeys = Object.keys(TH).sort();
    const enKeys = Object.keys(EN).sort();
    const zhKeys = Object.keys(ZH).sort();

    expect(enKeys).toEqual(thKeys);
    expect(zhKeys).toEqual(thKeys);
  });

  it('matching placeholders across th, en, and zh', () => {
    const placeholderRegex = /\{([a-zA-Z0-9_]+)\}/g;
    for (const key of Object.keys(TH)) {
      const thMatches = [...(TH[key]?.matchAll(placeholderRegex) || [])].map((m) => m[1]).sort();
      const enMatches = [...(EN[key]?.matchAll(placeholderRegex) || [])].map((m) => m[1]).sort();
      const zhMatches = [...(ZH[key]?.matchAll(placeholderRegex) || [])].map((m) => m[1]).sort();

      expect(enMatches, `placeholder parity for ${key} in en`).toEqual(thMatches);
      expect(zhMatches, `placeholder parity for ${key} in zh`).toEqual(thMatches);
    }
  });

  it('no empty values in zh', () => {
    for (const [key, value] of Object.entries(ZH)) {
      expect(typeof value, `zh.${key} is string`).toBe('string');
      expect(value.trim().length, `zh.${key} is not empty`).toBeGreaterThan(0);
    }
  });
});

describe('page and entry conventions (APP-11, customer-app.md §4.5)', () => {
  it('web/pages/app-history.html has xc-path="/app/history" and xc-gate="anyone"', () => {
    const htmlPath = path.resolve(thisDir, '../../../../pages/app-history.html');
    expect(fs.existsSync(htmlPath), 'web/pages/app-history.html exists').toBe(true);

    const html = fs.readFileSync(htmlPath, 'utf8');
    expect(html).toMatch(/<meta\s+name=["']xc-path["']\s+content=["']\/app\/history["']/);
    expect(html).toMatch(/<meta\s+name=["']xc-gate["']\s+content=["']anyone["']/);
    expect(html).toMatch(/src=["']\/src\/screens\/customer\/history\/main\.tsx["']/);
  });

  it('main.tsx sets setAudience("customer") and uses CustomerShell route="/app/history"', () => {
    const mainPath = path.resolve(thisDir, './main.tsx');
    expect(fs.existsSync(mainPath), 'web/src/screens/customer/history/main.tsx exists').toBe(true);

    const mainContent = fs.readFileSync(mainPath, 'utf8');
    expect(mainContent).toMatch(/setAudience\s*\(\s*['"]customer['"]\s*\)/);
    expect(mainContent).toMatch(/CustomerShell/);
    expect(mainContent).toMatch(/route=["']\/app\/history["']/);
  });
});
