// Written before the implementation by the test writer gemini/warut-2 (ticket author claude/e92db6). Do not edit, delete or weaken.
// MK-8c (#1646): Admin Campaign Performance screen M04
// (design `docs/design/blueprint/marketing-campaigns.md` D-15, §4.5, §9.1 MK-8; §6.1, §8.x).

import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../../../shared/api/client';
import type {
  CampaignPerformanceData,
  VersionPerformanceRow,
} from '../../../shared/api/generated/campaign_performance';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { AdminShell } from '../AdminShell';
import type { AdminMenuProps } from '../menus';
import { menus } from '../menus';
import { CampaignPerformance } from './CampaignPerformance';
import * as table from './i18n';
import { menu } from './menu';

interface SentRequest {
  method: string;
  path: string;
  search: string;
  body: unknown;
  headers: Record<string, string>;
}

function dummyProps(overrides: Partial<AdminMenuProps> = {}): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-14' });
  const [shown] = createSignal(true);
  return {
    branch,
    period,
    shown,
    ...overrides,
  };
}

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

const SAMPLE_PERFORMANCE_ROW: VersionPerformanceRow = {
  campaignId: 'cmp-1',
  campaignName: 'Golden October',
  versionId: 'ver-1',
  version: 1,
  redemptions: 120,
  priceCostThb: '15000.00',
  rewardExpenseThb: '5000.00',
  volumeThb: '2500000.00',
  realizedMarginThb: {
    value: '45000.00',
    unavailable: null,
  },
  attributedContributionThb: {
    value: '40000.00',
    unavailable: null,
  },
};

describe('MK-8c: Admin Campaign Performance M04 (§9 edge cases and §7 [required])', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];

  let viewStatus = 200;
  let viewData: CampaignPerformanceData = { versions: [SAMPLE_PERFORMANCE_ROW] };
  let viewErrorCode: string | undefined = undefined;

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];

    viewStatus = 200;
    viewData = { versions: [SAMPLE_PERFORMANCE_ROW] };
    viewErrorCode = undefined;

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers: Record<string, string> = {};
      if (init?.headers) {
        new Headers(init.headers).forEach((v, k) => {
          headers[k.toLowerCase()] = v;
        });
      }
      let body: unknown = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }

      sentRequests.push({ method, path: u.pathname, search: u.search, body, headers });

      if (u.pathname === '/api/v3/views/campaign_performance') {
        if (viewStatus !== 200) {
          return new Response(
            JSON.stringify({ error: { code: viewErrorCode ?? 'serverError', message: 'Failed to load performance' } }),
            { status: viewStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(JSON.stringify({ version: 1, changed: true, data: viewData }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      return new Response(JSON.stringify({ version: 1, changed: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(async () => {
    dispose?.();
    dispose = null;
    container.remove();
    vi.unstubAllGlobals();
    await settle();
  });

  // ── 1. Role gating & menu contract (§9 row 1, §7 [required]) ──────────────────
  describe('1. Role gating: TELLER has no menu; MARKETING and EXECUTIVE both see it (ticket §9 row 1)', () => {
    it('menu contract: id is campaign-performance and needs is MarketingPerformance', () => {
      expect(menu, 'menu is defined').toBeDefined();
      expect(menu?.id).toBe('campaign-performance');
      expect(menu?.needs).toBe('MarketingPerformance');
      expect(typeof menu?.label()).toBe('string');
      expect(typeof menu?.heading?.title()).toBe('string');
    });

    it('TELLER has no campaign-performance menu in AdminShell nav', async () => {
      const tellerMe = meOf({
        id: 'teller-1',
        role: 'TELLER',
        branch: 'MAIN',
        name: 'Teller 1',
      });

      const fakeApi = {
        async view<T>(name: string): Promise<{ version: number; changed: boolean; data?: T }> {
          if (name === 'me') return { version: 1, changed: true, data: tellerMe as unknown as T };
          if (name === 'admin_branches') return { version: 1, changed: true, data: { branches: [{ id: 'MAIN', name: 'Main' }] } as unknown as T };
          return { version: 1, changed: false };
        },
      } as unknown as ApiClient;

      window.history.replaceState(null, '', '/ui/admin/#campaign-performance');
      dispose = render(() => <AdminShell api={fakeApi} menus={menus} />, container);
      await settle();

      expect(container.querySelector('a[href="#overview"], a[href="#transactions"]'), 'shell rendered nav').not.toBeNull();
      const navLink = container.querySelector('a[href="#campaign-performance"]');
      expect(navLink, 'TELLER sees no campaign-performance menu link').toBeNull();
    });

    it('MARKETING role sees campaign-performance menu in AdminShell nav', async () => {
      const marketingMe = meOf({
        id: 'mkt-1',
        role: 'MARKETING',
        branch: 'MAIN',
        name: 'Marketing Staff',
      });

      const fakeApi = {
        async view<T>(name: string): Promise<{ version: number; changed: boolean; data?: T }> {
          if (name === 'me') return { version: 1, changed: true, data: marketingMe as unknown as T };
          if (name === 'admin_branches') return { version: 1, changed: true, data: { branches: [{ id: 'MAIN', name: 'Main' }] } as unknown as T };
          return { version: 1, changed: false };
        },
      } as unknown as ApiClient;

      window.history.replaceState(null, '', '/ui/admin/#campaign-performance');
      dispose = render(() => <AdminShell api={fakeApi} menus={menus} />, container);
      await settle();

      const navLink = container.querySelector('a[href="#campaign-performance"]');
      expect(navLink, 'MARKETING sees campaign-performance menu link').not.toBeNull();
    });

    it('EXECUTIVE role sees campaign-performance menu in AdminShell nav', async () => {
      const execMe = meOf({
        id: 'exec-1',
        role: 'EXECUTIVE',
        branch: 'MAIN',
        name: 'Executive Owner',
      });

      const fakeApi = {
        async view<T>(name: string): Promise<{ version: number; changed: boolean; data?: T }> {
          if (name === 'me') return { version: 1, changed: true, data: execMe as unknown as T };
          if (name === 'admin_branches') return { version: 1, changed: true, data: { branches: [{ id: 'MAIN', name: 'Main' }] } as unknown as T };
          return { version: 1, changed: false };
        },
      } as unknown as ApiClient;

      window.history.replaceState(null, '', '/ui/admin/#campaign-performance');
      dispose = render(() => <AdminShell api={fakeApi} menus={menus} />, container);
      await settle();

      const navLink = container.querySelector('a[href="#campaign-performance"]');
      expect(navLink, 'EXECUTIVE sees campaign-performance menu link').not.toBeNull();
    });
  });

  // ── 2. Data source: view campaign_performance per version and period (§7) ──────
  describe('2. Data source conventions (ticket §7 [required])', () => {
    it('data comes only from view campaign_performance per version and period', async () => {
      dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
      await settle();

      const getReq = sentRequests.find((r) => r.path === '/api/v3/views/campaign_performance');
      expect(getReq, 'queries /api/v3/views/campaign_performance').toBeDefined();
      expect(getReq?.search).toContain('from=2026-10-01');
      expect(getReq?.search).toContain('to=2026-10-14');
    });
  });

  // ── 3. No field, label or text says "profit" or "lift" (D-15, §7, §9 row 3) ────
  describe('3. Forbidden wording: no field, label or text says profit or lift (D-15, ticket §7, §9 row 3)', () => {
    it('no visible text matches /profit|lift|กำไร/i', async () => {
      dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
      await settle();

      // First assert that the table rendered (fails on stub)
      expect(container.textContent, 'table rendered campaign row').toContain('Golden October');

      const text = container.textContent ?? '';
      expect(text).not.toMatch(/profit|lift|กำไร/i);

      // Verify table headers / labels also do not use profit or lift
      const labels = [...container.querySelectorAll('th, td, label, h1, h2, h3, h4, span')].map(
        (el) => el.textContent ?? '',
      );
      for (const label of labels) {
        expect(label).not.toMatch(/profit|lift|กำไร/i);
      }

      // Check i18n keys and values in th and en
      const allThEntries = [...Object.keys(table.th), ...Object.values(table.th)];
      const allEnEntries = [...Object.keys(table.en), ...Object.values(table.en)];
      for (const entry of [...allThEntries, ...allEnEntries]) {
        expect(entry).not.toMatch(/profit|lift|กำไร/i);
      }
    });
  });

  // ── 4. Unavailable contribution shows "unavailable", never 0 (§7, §9 row 2, 4) ─
  describe('4. Unavailable contribution handling (ticket §7 [required], §9 row 2, row 4)', () => {
    it('cost_source = UNAVAILABLE (awaitsLedger) gives contribution unavailable, not 0.00 (§9 row 2)', async () => {
      viewData = {
        versions: [
          {
            ...SAMPLE_PERFORMANCE_ROW,
            realizedMarginThb: { value: null, unavailable: 'awaitsLedger' },
            attributedContributionThb: { value: null, unavailable: 'awaitsLedger' },
          },
        ],
      };

      dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
      await settle();

      // Scope check to contribution cell, must show unavailable text and never match 0 or 0.00
      const contributionCell = container.querySelector(
        '[data-figure="attributedContribution"], [data-col="attributedContribution"], td.xc-contribution',
      );
      expect(contributionCell, 'attributedContribution cell exists').not.toBeNull();
      const cellText = contributionCell?.textContent?.trim() ?? '';
      expect(cellText.toLowerCase()).toMatch(/unavailable|ยังไม่พร้อม|ไม่พร้อม/i);
      expect(cellText).not.toMatch(/^\s*0(\.00)?\s*$/);
    });

    it('campaign with no redemptions shows zeros for counts but unavailable for contribution when cost unavailable (§9 row 4)', async () => {
      viewData = {
        versions: [
          {
            ...SAMPLE_PERFORMANCE_ROW,
            redemptions: 0,
            volumeThb: '0.00',
            priceCostThb: '0.00',
            rewardExpenseThb: '0.00',
            realizedMarginThb: { value: null, unavailable: 'awaitsLedger' },
            attributedContributionThb: { value: null, unavailable: 'awaitsLedger' },
          },
        ],
      };

      dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
      await settle();

      // Redemptions cell reads 0
      const redemptionsCell = container.querySelector(
        '[data-figure="redemptions"], [data-col="redemptions"], td.xc-redemptions',
      );
      expect(redemptionsCell, 'redemptions cell exists').not.toBeNull();
      expect(redemptionsCell?.textContent?.trim()).toBe('0');

      // Contribution cell shows unavailable text, never 0 or 0.00
      const contributionCell = container.querySelector(
        '[data-figure="attributedContribution"], [data-col="attributedContribution"], td.xc-contribution',
      );
      expect(contributionCell, 'attributedContribution cell exists').not.toBeNull();
      const cellText = contributionCell?.textContent?.trim() ?? '';
      expect(cellText.toLowerCase()).toMatch(/unavailable|ยังไม่พร้อม|ไม่พร้อม/i);
      expect(cellText).not.toMatch(/^\s*0(\.00)?\s*$/);
    });

    it('commission awaitsBroker shows "awaits broker" (§7 [required], §6.1)', async () => {
      viewData = {
        versions: [
          {
            ...SAMPLE_PERFORMANCE_ROW,
            realizedMarginThb: { value: '50000.00', unavailable: null },
            attributedContributionThb: { value: null, unavailable: 'awaitsBroker' },
          },
        ],
      };

      dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
      await settle();

      const contributionCell = container.querySelector(
        '[data-figure="attributedContribution"], [data-col="attributedContribution"], td.xc-contribution',
      );
      expect(contributionCell, 'attributedContribution cell exists').not.toBeNull();
      const cellText = contributionCell?.textContent?.trim() ?? '';
      expect(cellText.toLowerCase()).toMatch(/awaits broker|รอนายหน้า|รอโบรกเกอร์/i);
      expect(cellText).not.toMatch(/^\s*0(\.00)?\s*$/);
    });
  });

  // ── 5. Money display from strings, never computed in screen (§7 [required]) ────
  describe('5. Money display from strings (ticket §7 [required])', () => {
    it('money figures are shown directly from view strings, never computed in screen', async () => {
      viewData = {
        versions: [
          {
            ...SAMPLE_PERFORMANCE_ROW,
            priceCostThb: '12345.67',
            rewardExpenseThb: '7654.32',
            volumeThb: '9876543.21',
            realizedMarginThb: { value: '45678.90', unavailable: null },
            attributedContributionThb: { value: '38024.58', unavailable: null },
          },
        ],
      };

      dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
      await settle();

      expect(container.textContent).toContain('12,345.67');
      expect(container.textContent).toContain('7,654.32');
      expect(container.textContent).toContain('9,876,543.21');
      expect(container.textContent).toContain('45,678.90');
      expect(container.textContent).toContain('38,024.58');
    });
  });

  // ── 6. M04 table rendering per version and period ─────────────────────────────
  describe('6. M04 table rendering and period reactivity', () => {
    it('renders performance table headers and rows', async () => {
      viewData = { versions: [SAMPLE_PERFORMANCE_ROW] };
      dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
      await settle();

      expect(container.textContent).toContain('Golden October');
      expect(container.textContent).toContain('120'); // redemptions
    });

    it('refetches view when period changes', async () => {
      const [branch] = createSignal('MAIN');
      const [period, setPeriod] = createSignal({ from: '2026-10-01', to: '2026-10-14' });
      const [shown] = createSignal(true);

      dispose = render(() => <CampaignPerformance branch={branch} period={period} shown={shown} />, container);
      await settle();

      expect(sentRequests.some((r) => r.search.includes('from=2026-10-01'))).toBe(true);

      sentRequests = [];
      setPeriod({ from: '2026-09-01', to: '2026-09-30' });
      await settle();

      expect(sentRequests.some((r) => r.search.includes('from=2026-09-01'))).toBe(true);
    });
  });

  // ── 7. Loading, empty and error states (§8) ───────────────────────────────────
  describe('7. Loading, empty and error states with retry (§8)', () => {
    it('empty state is shown when no versions exist in period', async () => {
      viewData = { versions: [] };
      dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
      await settle();

      expect(container.textContent).not.toContain('Failed');
      expect(container.textContent?.toLowerCase()).toMatch(/no data|ไม่มีข้อมูล|ไม่พบ/i);
    });

    it('shows worded error message and retry button on view failure (§8)', async () => {
      viewStatus = 500;
      viewErrorCode = 'serverError';

      dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
      await settle();

      expect(container.textContent).toContain('Failed to load performance');
      const retryBtn = [...container.querySelectorAll('button')].find((b) =>
        /retry|ลองใหม่|โหลดใหม่/i.test(b.textContent ?? ''),
      );
      expect(retryBtn, 'retry button exists').toBeDefined();

      viewStatus = 200;
      sentRequests = [];
      retryBtn?.click();
      await settle();

      expect(sentRequests.some((r) => r.path === '/api/v3/views/campaign_performance')).toBe(true);
    });

    it('shows worded error for outOfScope refusal (§8)', async () => {
      viewStatus = 400;
      viewErrorCode = 'outOfScope';

      dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
      await settle();

      const expectedText = table.th['error.outOfScope'] ?? table.th['err.outOfScope'];
      expect(expectedText, 'table.th has key error.outOfScope or err.outOfScope').toBeDefined();
      expect(container.textContent).toContain(expectedText);
      expect(container.textContent).not.toContain('outOfScope');
    });

    it('shows worded error for validationFailed (§8)', async () => {
      viewStatus = 400;
      viewErrorCode = 'validationFailed';

      dispose = render(() => <CampaignPerformance {...dummyProps()} />, container);
      await settle();

      const expectedText = table.th['error.validationFailed'] ?? table.th['err.validationFailed'];
      expect(expectedText, 'table.th has key error.validationFailed or err.validationFailed').toBeDefined();
      expect(container.textContent).toContain(expectedText);
      expect(container.textContent).not.toContain('validationFailed');
    });
  });

  // ── 8. TH/EN key parity ───────────────────────────────────────────────────────
  describe('8. i18n key parity', () => {
    it('th and en keys have identical sets and are non-empty', () => {
      const thKeys = Object.keys(table.th).sort();
      const enKeys = Object.keys(table.en).sort();
      expect(thKeys).toEqual(enKeys);
      expect(thKeys.length, 'i18n keys are present').toBeGreaterThan(0);
    });
  });
});
