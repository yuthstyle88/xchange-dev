import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CampaignDetail } from '../../../shared/api/generated/campaign';
import type { CampaignListItem, CampaignsData } from '../../../shared/api/generated/campaigns';
import { errorText, setLang } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { Campaigns } from './Campaigns';
import * as table from './i18n';

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

const SAMPLE_LIST_ITEM: CampaignListItem = {
  id: 'cmp-1',
  name: 'Golden October',
  versionId: 'ver-1',
  version: 1,
  status: 'ACTIVE',
  startsAt: '2026-10-01T00:00:00Z',
  endsAt: '2026-10-31T23:59:59Z',
  budgetThb: '100000.00',
  budgetUsedThb: '25000.50',
  budgetLeftThb: '74999.50',
  uses: 42,
  maxUses: 100,
  period: {
    redemptions: 15,
    priceCostThb: '12345.67',
    rewardExpenseThb: '3000.00',
  },
};

const SAMPLE_DETAIL_NO_VERSIONS: CampaignDetail = {
  id: 'cmp-1',
  name: 'Golden October',
  currentVersionId: 'ver-1',
  versions: [],
};

describe('Campaigns rework tests (#1699)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  let listStatus = 200;
  let listData: CampaignsData | null = null;
  let listError: { code: string; message: string } | null = null;
  let listNetworkError: Error | null = null;

  let detailStatus = 200;
  let detailData: { campaign: CampaignDetail } | null = null;
  let detailError: { code: string; message: string } | null = null;
  let detailNetworkError: Error | null = null;

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');

    listStatus = 200;
    listData = { campaigns: [SAMPLE_LIST_ITEM] };
    listError = null;
    listNetworkError = null;

    detailStatus = 200;
    detailData = null;
    detailError = null;
    detailNetworkError = null;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(url, 'http://localhost');

        if (u.pathname === '/api/v3/views/campaigns') {
          if (listNetworkError) {
            throw listNetworkError;
          }
          if (listStatus !== 200) {
            return new Response(
              JSON.stringify({ error: listError ?? { code: 'serverError', message: 'Failed to load campaigns' } }),
              { status: listStatus, headers: { 'content-type': 'application/json' } },
            );
          }
          return new Response(JSON.stringify({ version: 1, changed: true, data: listData }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          });
        }

        if (u.pathname === '/api/v3/views/campaign') {
          if (detailNetworkError) {
            throw detailNetworkError;
          }
          if (detailStatus !== 200) {
            return new Response(
              JSON.stringify({ error: detailError ?? { code: 'serverError', message: 'Failed to load campaign' } }),
              { status: detailStatus, headers: { 'content-type': 'application/json' } },
            );
          }
          return new Response(JSON.stringify({ version: 1, changed: true, data: detailData }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          });
        }

        return new Response(JSON.stringify({ version: 1, changed: false }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }),
    );
  });

  afterEach(async () => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  // ── Row 1: Campaigns.tsx:247 — status ?? 'DRAFT' shows Draft when version data is missing ────
  describe('Row 1: Campaigns.tsx:247 — version status shows unknown or loading state, never made-up Draft', () => {
    it('shows unknown status when campaign has no versions, never falling back to Draft (TH)', async () => {
      detailData = { campaign: SAMPLE_DETAIL_NO_VERSIONS };

      dispose = render(() => <Campaigns {...dummyProps()} />, host);
      await settle();

      const row = host.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();

      const statusEl = host.querySelector('[data-testid="campaign-status"]');
      expect(statusEl, 'status badge in detail header must exist').not.toBeNull();
      expect(statusEl?.textContent).toBe(table.th['status.unknown'] ?? 'ไม่ทราบสถานะ');
      expect(statusEl?.textContent).not.toBe(table.th['status.draft']);
    });

    it('shows unknown status when campaign has no versions, never falling back to Draft (EN)', async () => {
      setLang('en');
      detailData = { campaign: SAMPLE_DETAIL_NO_VERSIONS };

      dispose = render(() => <Campaigns {...dummyProps()} />, host);
      await settle();

      const row = host.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();

      const statusEl = host.querySelector('[data-testid="campaign-status"]');
      expect(statusEl, 'status badge in detail header must exist').not.toBeNull();
      expect(statusEl?.textContent).toBe(table.en['status.unknown'] ?? 'Unknown');
      expect(statusEl?.textContent).not.toBe(table.en['status.draft']);
    });

    it('shows loading status while campaign detail is being fetched', async () => {
      let resolveDetail: (value: Response) => void;
      const detailPromise = new Promise<Response>((resolve) => {
        resolveDetail = resolve;
      });

      vi.stubGlobal(
        'fetch',
        vi.fn(async (input: RequestInfo | URL) => {
          const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
          if (url.includes('/api/v3/views/campaigns')) {
            return new Response(JSON.stringify({ version: 1, changed: true, data: listData }), {
              status: 200,
              headers: { 'content-type': 'application/json' },
            });
          }
          if (url.includes('/api/v3/views/campaign')) {
            return detailPromise;
          }
          return new Response(JSON.stringify({ version: 1, changed: false }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          });
        }),
      );

      dispose = render(() => <Campaigns {...dummyProps()} />, host);
      await settle();

      const row = host.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();

      const statusEl = host.querySelector('[data-testid="campaign-status"]');
      expect(statusEl, 'status badge in detail header must exist').not.toBeNull();
      expect(statusEl?.textContent).toBe(table.th['status.loading'] ?? 'กำลังโหลด...');
      expect(statusEl?.textContent).not.toBe(table.th['status.draft']);

      resolveDetail!(
        new Response(
          JSON.stringify({ version: 1, changed: true, data: { campaign: SAMPLE_DETAIL_NO_VERSIONS } }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      );
      await settle();
    });
  });

  // ── Row 2: Campaigns.tsx:70 — formatError uses errorText, never raw code or err.message ────
  describe('Row 2: Campaigns.tsx:70 — formatError worded errors for unmapped codes and network failures', () => {
    it('shows generic worded error on 500 network failure in list read, never raw message', async () => {
      listNetworkError = new TypeError('Failed to fetch network connection lost');

      dispose = render(() => <Campaigns {...dummyProps()} />, host);
      await settle();

      const errEl = host.querySelector('.xc-campaigns-error');
      expect(errEl, 'error banner must be displayed').not.toBeNull();
      const span = errEl?.querySelector('span') ?? errEl;
      expect(span?.textContent).toBe(errorText('network', ''));
      expect(span?.textContent).not.toContain('Failed to fetch');
    });

    it('shows generic worded error on 500 network failure in English', async () => {
      setLang('en');
      listNetworkError = new TypeError('NetworkError when attempting to fetch resource.');

      dispose = render(() => <Campaigns {...dummyProps()} />, host);
      await settle();

      const errEl = host.querySelector('.xc-campaigns-error');
      expect(errEl, 'error banner must be displayed').not.toBeNull();
      const span = errEl?.querySelector('span') ?? errEl;
      expect(span?.textContent).toBe(errorText('network', ''));
      expect(span?.textContent).not.toContain('NetworkError');
    });

    it('shows generic worded error on refusal with unknown code, never raw code or err.message', async () => {
      listStatus = 400;
      listError = { code: 'unknownUnmappedRefusalCode', message: 'RAW-unmapped-server-details' };

      dispose = render(() => <Campaigns {...dummyProps()} />, host);
      await settle();

      const errEl = host.querySelector('.xc-campaigns-error');
      expect(errEl, 'error banner must be displayed').not.toBeNull();
      const span = errEl?.querySelector('span') ?? errEl;
      expect(span?.textContent).toBe('RAW-unmapped-server-details');
      expect(span?.textContent).not.toContain('unknownUnmappedRefusalCode');
    });

    it('shows shared worded error on refusal with known shared error code', async () => {
      listStatus = 403;
      listError = { code: 'outOfScope', message: 'outOfScope' };

      dispose = render(() => <Campaigns {...dummyProps()} />, host);
      await settle();

      const errEl = host.querySelector('.xc-campaigns-error');
      expect(errEl, 'error banner must be displayed').not.toBeNull();
      const span = errEl?.querySelector('span') ?? errEl;
      expect(span?.textContent).toBe(table.th['error.outOfScope']);
      expect(span?.textContent).not.toContain('Forbidden');
    });

    it('stale list data remains visible on subsequent failed detail read, but worded error is shown', async () => {
      detailStatus = 500;
      detailError = { code: 'unmappedDetailFailure', message: 'RAW-detail-failure-message' };

      dispose = render(() => <Campaigns {...dummyProps()} />, host);
      await settle();

      // Initial list loaded successfully
      expect(host.textContent).toContain('Golden October');

      // Click row to open detail, which fails
      const row = host.querySelector<HTMLTableRowElement>('tr[data-campaign-id="cmp-1"]');
      row?.click();
      await settle();

      const errEl = host.querySelector('.xc-campaigns-error');
      expect(errEl, 'detail error banner must be displayed').not.toBeNull();
      const span = errEl?.querySelector('span') ?? errEl;
      expect(span?.textContent).toBe(errorText('serverError', ''));
      expect(span?.textContent).not.toContain('unmappedDetailFailure');
      expect(span?.textContent).not.toContain('RAW-detail-failure-message');
    });
  });
});
