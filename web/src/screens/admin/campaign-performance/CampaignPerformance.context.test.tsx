import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeCampaignPerformanceData,
  makePerformanceFigure,
  makeVersionPerformanceRow,
  type CampaignPerformanceData,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { CampaignPerformance } from './CampaignPerformance';
import * as table from './i18n';

const OCT_ROW = makeVersionPerformanceRow({
  campaignId: 'cmp-oct',
  campaignName: 'Golden October',
  versionId: 'ver-oct-1',
  version: 1,
  redemptions: 120,
  priceCostThb: '15000.00',
  rewardExpenseThb: '5000.00',
  volumeThb: '2500000.00',
  realizedMarginThb: makePerformanceFigure({ value: '45000.00', unavailable: null }),
  attributedContributionThb: makePerformanceFigure({ value: '40000.00', unavailable: null }),
});

const NOV_ROW = makeVersionPerformanceRow({
  campaignId: 'cmp-nov',
  campaignName: 'November Blitz',
  versionId: 'ver-nov-1',
  version: 2,
  redemptions: 85,
  priceCostThb: '9000.00',
  rewardExpenseThb: '3000.00',
  volumeThb: '1800000.00',
  realizedMarginThb: makePerformanceFigure({ value: '32000.00', unavailable: null }),
  attributedContributionThb: makePerformanceFigure({ value: '29000.00', unavailable: null }),
});

const SILOM_ROW = makeVersionPerformanceRow({
  campaignId: 'cmp-silom',
  campaignName: 'Silom Special',
  versionId: 'ver-silom-1',
  version: 1,
  redemptions: 64,
  priceCostThb: '7200.00',
  rewardExpenseThb: '2100.00',
  volumeThb: '1100000.00',
  realizedMarginThb: makePerformanceFigure({ value: '21000.00', unavailable: null }),
  attributedContributionThb: makePerformanceFigure({ value: '18900.00', unavailable: null }),
});

function okResponse(data: CampaignPerformanceData): Response {
  return new Response(JSON.stringify({ version: 1, changed: true, data }), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
}

function errorResponse(status = 500, code = 'serverError'): Response {
  return new Response(
    JSON.stringify({ error: { code, message: 'Failed to load campaign performance' } }),
    { status, headers: { 'content-type': 'application/json' } },
  );
}

describe('Admin Campaign Performance context change (#2337)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let pendingResolvers: Array<(res: Response) => void> = [];
  let holdNext = false;
  let nextData: CampaignPerformanceData = makeCampaignPerformanceData({ versions: [OCT_ROW] });

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    pendingResolvers = [];
    holdNext = false;
    nextData = makeCampaignPerformanceData({ versions: [OCT_ROW] });

    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      if (u.pathname === '/api/v3/views/campaign_performance') {
        if (holdNext) {
          return new Promise<Response>((resolve) => {
            pendingResolvers.push(resolve);
          });
        }
        return okResponse(nextData);
      }
      return new Response(JSON.stringify({ version: 1, changed: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = null;
    container.remove();
    vi.unstubAllGlobals();
  });

  it('period changes, answer pending: no rows from the old period and loading text is shown', async () => {
    const [branch] = createSignal('MAIN');
    const [period, setPeriod] = createSignal({ from: '2026-10-01', to: '2026-10-14' });
    const [shown] = createSignal(true);

    dispose = render(() => <CampaignPerformance branch={branch} period={period} shown={shown} />, container);
    await vi.waitFor(() => {
      expect(container.textContent).toContain('Golden October');
      expect(container.textContent).toContain('120');
    });

    holdNext = true;
    setPeriod({ from: '2026-10-07', to: '2026-10-14' });

    await vi.waitFor(() => {
      expect(pendingResolvers).toHaveLength(1);
    });

    expect(container.querySelector('.xc-cp-table')).toBeNull();
    expect(container.textContent).not.toContain('Golden October');
    expect(container.textContent).not.toContain('120');
    expect(container.querySelector('.xc-cp-loading')?.textContent).toBe(table.th.loading);
  });

  it('period changes, answer arrives: clears old rows while pending and shows only the new period rows', async () => {
    const [branch] = createSignal('MAIN');
    const [period, setPeriod] = createSignal({ from: '2026-10-01', to: '2026-10-14' });
    const [shown] = createSignal(true);

    dispose = render(() => <CampaignPerformance branch={branch} period={period} shown={shown} />, container);
    await vi.waitFor(() => {
      expect(container.textContent).toContain('Golden October');
    });

    holdNext = true;
    setPeriod({ from: '2026-10-01', to: '2026-10-31' });

    await vi.waitFor(() => {
      expect(pendingResolvers).toHaveLength(1);
    });
    expect(container.textContent).not.toContain('Golden October');
    expect(container.querySelector('.xc-cp-loading')?.textContent).toBe(table.th.loading);

    pendingResolvers[0](okResponse(makeCampaignPerformanceData({ versions: [NOV_ROW] })));

    await vi.waitFor(() => {
      expect(container.textContent).toContain('November Blitz');
      expect(container.textContent).toContain('85');
    });
    expect(container.textContent).not.toContain('Golden October');
    expect(container.querySelector('.xc-cp-loading')).toBeNull();
  });

  it('period changes, read fails: clears old rows immediately, then shows error and retry with no rows', async () => {
    const [branch] = createSignal('MAIN');
    const [period, setPeriod] = createSignal({ from: '2026-10-01', to: '2026-10-14' });
    const [shown] = createSignal(true);

    dispose = render(() => <CampaignPerformance branch={branch} period={period} shown={shown} />, container);
    await vi.waitFor(() => {
      expect(container.textContent).toContain('Golden October');
    });

    holdNext = true;
    setPeriod({ from: '2026-09-01', to: '2026-09-30' });

    await vi.waitFor(() => {
      expect(pendingResolvers).toHaveLength(1);
    });
    expect(container.querySelector('.xc-cp-table')).toBeNull();
    expect(container.textContent).not.toContain('Golden October');

    pendingResolvers[0](errorResponse(500, 'serverError'));

    await vi.waitFor(() => {
      expect(container.querySelector('.xc-cp-error')?.textContent).toContain(th['error.serverError']);
    });
    expect(container.querySelector('.xc-cp-retry-btn')).not.toBeNull();
    expect(container.querySelector('.xc-cp-table')).toBeNull();
    expect(container.textContent).not.toContain('Golden October');
  });

  it('shown goes false -> true with the same context: table is not blanked; changing context blanks it', async () => {
    const [branch] = createSignal('MAIN');
    const [period, setPeriod] = createSignal({ from: '2026-10-01', to: '2026-10-14' });
    const [shown, setShown] = createSignal(true);

    dispose = render(() => <CampaignPerformance branch={branch} period={period} shown={shown} />, container);
    await vi.waitFor(() => {
      expect(container.textContent).toContain('Golden October');
    });

    holdNext = true;
    setShown(false);
    setShown(true);

    await vi.waitFor(() => {
      expect(pendingResolvers).toHaveLength(1);
    });
    // Same context: existing rows stay visible while refreshing
    expect(container.querySelector('.xc-cp-table')).not.toBeNull();
    expect(container.textContent).toContain('Golden October');
    expect(container.querySelector('.xc-cp-loading')).toBeNull();

    pendingResolvers[0](okResponse(makeCampaignPerformanceData({ versions: [OCT_ROW] })));
    await vi.waitFor(() => {
      expect(container.textContent).toContain('Golden October');
    });

    // Changing context while hidden and then showing again blanks the stale rows
    setShown(false);
    setPeriod({ from: '2026-08-01', to: '2026-08-31' });
    setShown(true);

    await vi.waitFor(() => {
      expect(pendingResolvers).toHaveLength(2);
    });
    expect(container.querySelector('.xc-cp-table')).toBeNull();
    expect(container.textContent).not.toContain('Golden October');
    expect(container.querySelector('.xc-cp-loading')?.textContent).toBe(table.th.loading);
  });

  it('branch change (same period): treated like a period change, rows cleared while loading', async () => {
    const [branch, setBranch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-14' });
    const [shown] = createSignal(true);

    dispose = render(() => <CampaignPerformance branch={branch} period={period} shown={shown} />, container);
    await vi.waitFor(() => {
      expect(container.textContent).toContain('Golden October');
    });

    holdNext = true;
    setBranch('SILOM');

    await vi.waitFor(() => {
      expect(pendingResolvers).toHaveLength(1);
    });
    expect(container.querySelector('.xc-cp-table')).toBeNull();
    expect(container.textContent).not.toContain('Golden October');
    expect(container.querySelector('.xc-cp-loading')?.textContent).toBe(table.th.loading);

    pendingResolvers[0](okResponse(makeCampaignPerformanceData({ versions: [SILOM_ROW] })));
    await vi.waitFor(() => {
      expect(container.textContent).toContain('Silom Special');
      expect(container.textContent).toContain('64');
    });
    expect(container.textContent).not.toContain('Golden October');
  });
});
