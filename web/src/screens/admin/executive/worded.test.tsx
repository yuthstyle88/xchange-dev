// WE-16 (#1765): screens/admin/executive/ — show failed calls through wordedError
// Tests for §4.2 (three tests per changed catch: load and poll)
// and §4.3 (stale data cleared on query/filter switch before failed read).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';
import type {
  BranchRowDto,
  DefinitionDto,
  ExecutiveOverviewData,
  MetricDto,
} from '../../../shared/api/generated/executive_overview';
import { Executive } from './Executive';

const ENDPOINT = '/api/v3/views/executive_overview';
const FROM = '2026-10-05';
const TO = '2026-10-06';
const VERSIONS = { B1: 4, B2: 9 };

function metric(key: string, value: string | null): MetricDto {
  return { key, value, unavailable: null, drilldown: null };
}

function branchRow(id: string): BranchRowDto {
  return {
    branchId: id,
    name: id === 'B1' ? 'สาขาหนึ่ง' : 'สาขาสอง',
    volumeThb: '2000.50',
    availableCash: metric('available_cash', '150000.00'),
    inTransit: metric('in_transit', '5000.00'),
    shifts: { open: 1, closedVerified: 2, closedUnverified: 3 },
    asOf: '2026-10-06T03:00:00Z',
    pendingEvents: 0,
    drilldown: { view: 'admin_overview', filter: { branch: id }, dropped: [] },
  };
}

const SAMPLE_DEFINITIONS: DefinitionDto[] = [
  { key: 'buy_volume_thb', th: 'มูลค่าบาทของรายการซื้อที่สำเร็จ ไม่ใช่รายได้', en: 'THB value of completed buys; not revenue' },
];

const SAMPLE_KPIS: MetricDto[] = [
  { key: 'buy_volume_thb', value: '1234567.89', unavailable: null, drilldown: null },
];

const SAMPLE_BRANCHES: BranchRowDto[] = [branchRow('B1')];

const SAMPLE_DATA: ExecutiveOverviewData = {
  period: { from: FROM, to: TO },
  kpis: SAMPLE_KPIS,
  branches: SAMPLE_BRANCHES,
  alerts: [],
  definitions: SAMPLE_DEFINITIONS,
  buyReversedThb: '0.00',
  sellReversedThb: '0.00',
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const realSetTimeout = globalThis.setTimeout;

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

describe('Executive worded error handling (WE-16 #1765)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let overviewHandler: (url: URL) => Promise<Response>;

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);

    overviewHandler = async () =>
      jsonResponse({
        changed: true,
        versions: VERSIONS,
        data: SAMPLE_DATA,
      });

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: string | URL | Request) => {
        const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const url = new URL(raw, 'http://localhost');

        if (PAGE_WIDE.some((p) => url.pathname.includes(p)) || url.pathname === '/api/v3/views/me') {
          return jsonResponse({ version: 1, changed: false });
        }

        if (url.pathname === ENDPOINT) {
          return overviewHandler(url);
        }

        return jsonResponse({ error: { code: 'notFound', message: 'not found' } }, 404);
      }),
    );
  });

  afterEach(async () => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
    await settle();
  });

  function mount(initialPeriod = { from: FROM, to: TO }) {
    const [branch, setBranch] = createSignal('MAIN');
    const [period, setPeriod] = createSignal(initialPeriod);
    const [shown, setShown] = createSignal(true);
    dispose = render(
      () => <Executive branch={branch} period={period} shown={shown} />,
      root,
    );
    return { setBranch, setPeriod, setShown };
  }

  function errorBannerText(): string {
    const el = root.querySelector('.error-banner');
    return (el?.textContent ?? '').trim();
  }

  function tiles(): HTMLElement[] {
    return Array.from(root.querySelectorAll<HTMLElement>('[data-kpi]'));
  }

  function branchRows(): HTMLElement[] {
    return Array.from(root.querySelectorAll<HTMLElement>('[data-branch]'));
  }

  // ── Catch 1: load ─────────────────────────────────────────────────────────
  describe('Catch 1: load overview', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      overviewHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();
      await settle();

      expect(errorBannerText()).toBe(th['error.network']);
      expect(errorBannerText()).not.toContain('Failed to fetch');

      dispose?.();
      root.innerHTML = '';
      setLang('en');
      mount();
      await settle();

      expect(errorBannerText()).toBe(en['error.network']);
    });

    it('(2) 500 status with keyed non-server code shows error.serverError', async () => {
      overviewHandler = async () =>
        jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'role not allowed' } }, 500);
      mount();
      await settle();

      expect(errorBannerText()).toBe(th['error.serverError']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      const serverMessage = 'ข้อความปฏิเสธจากเซิร์ฟเวอร์ผู้บริหาร';
      overviewHandler = async () =>
        jsonResponse({ error: { code: 'unknownExecRefusal999', message: serverMessage } }, 400);
      mount();
      await settle();

      expect(errorBannerText()).toBe(serverMessage);
    });
  });

  // ── Catch 2: poll ─────────────────────────────────────────────────────────
  describe('Catch 2: poll overview', () => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
    });

    it('(1) network TypeError shows error.network in TH and EN', async () => {
      overviewHandler = async () =>
        jsonResponse({ changed: true, versions: VERSIONS, data: SAMPLE_DATA });
      mount();
      await settle();
      expect(tiles().length).toBe(1);
      expect(errorBannerText()).toBe('');

      // Trigger poll failure with network TypeError
      overviewHandler = () => Promise.reject(new TypeError('Network dropped during poll'));
      await vi.advanceTimersByTimeAsync(60_000);
      await settle();

      expect(errorBannerText()).toBe(th['error.network']);
      expect(errorBannerText()).not.toContain('Network dropped');

      // EN check
      dispose?.();
      root.innerHTML = '';
      setLang('en');
      overviewHandler = async () =>
        jsonResponse({ changed: true, versions: VERSIONS, data: SAMPLE_DATA });
      mount();
      await settle();
      expect(tiles().length).toBe(1);

      overviewHandler = () => Promise.reject(new TypeError('Network dropped during poll'));
      await vi.advanceTimersByTimeAsync(60_000);
      await settle();

      expect(errorBannerText()).toBe(en['error.network']);
    });

    it('(2) 500 status with keyed non-server code shows error.serverError', async () => {
      overviewHandler = async () =>
        jsonResponse({ changed: true, versions: VERSIONS, data: SAMPLE_DATA });
      mount();
      await settle();
      expect(tiles().length).toBe(1);
      expect(errorBannerText()).toBe('');

      // Trigger poll failure with 500
      overviewHandler = async () =>
        jsonResponse({ error: { code: 'roleNotAllowedHere', message: 'role not allowed' } }, 500);
      await vi.advanceTimersByTimeAsync(60_000);
      await settle();

      expect(errorBannerText()).toBe(th['error.serverError']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      const serverMessage = 'ข้อความปฏิเสธจากการโพลล์ข้อมูลผู้บริหาร';
      overviewHandler = async () =>
        jsonResponse({ changed: true, versions: VERSIONS, data: SAMPLE_DATA });
      mount();
      await settle();
      expect(tiles().length).toBe(1);
      expect(errorBannerText()).toBe('');

      // Trigger poll failure with unkeyed refusal
      overviewHandler = async () =>
        jsonResponse({ error: { code: 'unknownPollRefusal888', message: serverMessage } }, 400);
      await vi.advanceTimersByTimeAsync(60_000);
      await settle();

      expect(errorBannerText()).toBe(serverMessage);
    });
  });

  // ── Stale data: query switch ──────────────────────────────────────────────
  describe('Stale data handling on query/branch switch failure (§4.3)', () => {
    it('clears previous data on filter switch when the new read fails', async () => {
      overviewHandler = async () =>
        jsonResponse({ changed: true, versions: VERSIONS, data: SAMPLE_DATA });
      const { setPeriod } = mount();
      await settle();

      expect(tiles().length).toBe(1);
      expect(branchRows().length).toBe(1);
      expect(errorBannerText()).toBe('');

      // Switch period / query and fail
      overviewHandler = async () =>
        jsonResponse({ error: { code: 'serverError', message: 'DB down' } }, 500);

      setPeriod({ from: '2026-11-01', to: '2026-11-02' });
      await settle();

      expect(errorBannerText()).toBe(th['error.serverError']);
      expect(tiles().length).toBe(0);
      expect(branchRows().length).toBe(0);
      expect(root.querySelectorAll('[data-alert-kind]').length).toBe(0);
    });

    it('clears previous data when branch filter is changed and the new read fails', async () => {
      overviewHandler = async () =>
        jsonResponse({ changed: true, versions: VERSIONS, data: SAMPLE_DATA });
      mount();
      await settle();

      expect(tiles().length).toBe(1);
      expect(branchRows().length).toBe(1);
      expect(errorBannerText()).toBe('');

      // Toggle branch checkbox
      const checkbox = root.querySelector<HTMLInputElement>('input[name="branch"][value="B1"]');
      expect(checkbox).not.toBeNull();

      // Next read fails
      overviewHandler = async () =>
        jsonResponse({ error: { code: 'outOfScope', message: 'สาขานี้อยู่นอกขอบเขต' } }, 403);

      checkbox!.click();
      await settle();

      expect(errorBannerText()).toBe('สาขานี้อยู่นอกขอบเขต');
      expect(tiles().length).toBe(0);
      expect(branchRows().length).toBe(0);
    });
  });
});
