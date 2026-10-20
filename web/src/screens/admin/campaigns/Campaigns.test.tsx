// Written before the implementation by the test writer gemini/warut-2 (ticket author claude/e92db6). Do not edit, delete or weaken.
// Amended by gemini/warut-2 per reviewer claude/dell-claude-5 and controller ruling on PR 1654.
// MK-8a (#1645): Admin Marketing Campaigns screen M01 list + M02 campaign page
// (design `docs/design/blueprint/marketing-campaigns.md` §4.5, §9.1 MK-8; D-4, D-9, D-15).

import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../../../shared/api/client';
import {
  makeCampaignApprovalDetail,
  makeCampaignData,
  makeCampaignDetail,
  makeCampaignListItem,
  makeCampaignPauseOutput,
  makeCampaignPeriodTotals,
  makeCampaignResumeOutput,
  makeCampaignRuleDetail,
  makeCampaignSimulationDetail,
  makeCampaignVersionDetail,
  makeCampaignsData,
  makeCouponIssueOutput,
  makeIssuedCouponDto,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { AdminShell } from '../AdminShell';
import type { AdminMenuProps } from '../menus';
import { menus } from '../menus';
import { Campaigns } from './Campaigns';
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

const SAMPLE_LIST_ITEM = makeCampaignListItem({
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
  period: makeCampaignPeriodTotals({
    redemptions: 15,
    priceCostThb: '12345.67',
    rewardExpenseThb: '3000.00',
  }),
});

const SAMPLE_RULE_ID = '00000000-0000-0000-0000-000000000001';

const SAMPLE_DETAIL = makeCampaignDetail({
  id: 'cmp-1',
  name: 'Golden October',
  currentVersionId: 'ver-1',
  versions: [
    makeCampaignVersionDetail({
      id: 'ver-1',
      version: 1,
      status: 'ACTIVE',
      startsAt: '2026-10-01T00:00:00Z',
      endsAt: '2026-10-31T23:59:59Z',
      budgetThb: '100000.00',
      maxUses: 100,
      stackGroup: 'DEFAULT',
      priority: 10,
      branches: ['MAIN', 'B1'],
      rules: [
        makeCampaignRuleDetail({
          id: SAMPLE_RULE_ID,
          ruleNo: 1,
          benefitType: 'RATE_IMPROVEMENT',
          benefitClass: 'PRICE',
          requiresCoupon: true,
          eligibility: {},
          calculation: { spreadShare: '0.20' },
          caps: {},
        }),
      ],
      simulations: [
        makeCampaignSimulationDetail({
          id: 'sim-1',
          costSource: 'LEDGER',
          simulatedAt: '2026-10-02T10:00:00Z',
          input: {},
          result: {},
        }),
      ],
      approval: makeCampaignApprovalDetail({
        id: 'appr-1',
        status: 'APPROVED',
      }),
    }),
  ],
});

const DRAFT_DETAIL = makeCampaignDetail({
  ...SAMPLE_DETAIL,
  versions: [
    makeCampaignVersionDetail({
      ...SAMPLE_DETAIL.versions[0],
      status: 'DRAFT',
      approval: null,
      simulations: [],
    }),
  ],
});

const SIMULATED_DETAIL = makeCampaignDetail({
  ...SAMPLE_DETAIL,
  versions: [
    makeCampaignVersionDetail({
      ...SAMPLE_DETAIL.versions[0],
      status: 'SIMULATED',
      approval: null,
      simulations: [
        makeCampaignSimulationDetail({
          id: 'sim-1',
          costSource: 'LEDGER',
          simulatedAt: '2026-10-02T10:00:00Z',
          input: {},
          result: {},
        }),
      ],
    }),
  ],
});

const DRAFT_WITH_OLD_SIMULATION = makeCampaignDetail({
  ...SAMPLE_DETAIL,
  versions: [
    makeCampaignVersionDetail({
      ...SAMPLE_DETAIL.versions[0],
      status: 'DRAFT',
      approval: null,
      simulations: [
        makeCampaignSimulationDetail({
          id: 'sim-old',
          costSource: 'LEDGER',
          simulatedAt: '2026-10-01T10:00:00Z',
          input: {},
          result: {},
        }),
      ],
    }),
  ],
});

describe('MK-8a: Admin Marketing Campaigns (§9 edge cases and §7 [required])', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];

  let listStatus = 200;
  let listData = makeCampaignsData({ campaigns: [SAMPLE_LIST_ITEM] });
  let listErrorCode: string | undefined = undefined;

  let detailStatus = 200;
  let detailData = makeCampaignData({ campaign: SAMPLE_DETAIL });
  let detailErrorCode: string | undefined = undefined;

  let commandStatus = 200;
  let commandOutput: unknown = {};
  let commandErrorCode: string | undefined = undefined;

  async function openCampaign(campaignId = 'cmp-1'): Promise<void> {
    const itemEl = container.querySelector(`[data-campaign-id="${campaignId}"]`);
    expect(itemEl, `campaign ${campaignId} link/row exists on list`).not.toBeNull();
    (itemEl as HTMLElement).click();
    await settle();
  }

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];

    listStatus = 200;
    listData = makeCampaignsData({ campaigns: [SAMPLE_LIST_ITEM] });
    listErrorCode = undefined;

    detailStatus = 200;
    detailData = makeCampaignData({ campaign: SAMPLE_DETAIL });
    detailErrorCode = undefined;

    commandStatus = 200;
    commandOutput = {};
    commandErrorCode = undefined;

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

      if (u.pathname === '/api/v3/views/campaigns') {
        if (listStatus !== 200) {
          return new Response(
            JSON.stringify({ error: { code: listErrorCode ?? 'serverError', message: 'Failed to load campaigns' } }),
            { status: listStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(JSON.stringify({ version: 1, changed: true, data: listData }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      if (u.pathname === '/api/v3/views/campaign') {
        if (detailStatus !== 200) {
          return new Response(
            JSON.stringify({ error: { code: detailErrorCode ?? 'serverError', message: 'Failed to load campaign' } }),
            { status: detailStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(JSON.stringify({ version: 1, changed: true, data: detailData }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      // Real command routes and methods from each const ROUTE / METHOD:
      // - campaignCreate: POST /api/campaigns
      // - campaignVersionSave: PUT /api/campaigns/{id}/draft
      // - campaignVersionNew: POST /api/campaigns/{id}/versions
      // - campaignSimulate: POST /api/campaigns/{id}/simulate
      // - campaignSubmit: POST /api/campaigns/{id}/submit
      // - campaignPause: POST /api/campaigns/{id}/pause
      // - campaignResume: POST /api/campaigns/{id}/resume
      // - couponIssue: POST /api/campaigns/{id}/coupons
      const isCampaignCommand =
        (method === 'POST' && u.pathname === '/api/campaigns') ||
        (method === 'PUT' && /^\/api\/campaigns\/[^/]+\/draft$/.test(u.pathname)) ||
        (method === 'POST' && /^\/api\/campaigns\/[^/]+\/(versions|simulate|submit|pause|resume|coupons)$/.test(u.pathname));

      if (isCampaignCommand) {
        if (commandStatus !== 200) {
          return new Response(
            JSON.stringify({ error: { code: commandErrorCode ?? 'validationFailed', message: 'Command failed' } }),
            { status: commandStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(JSON.stringify(commandOutput), {
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

  // ── 1. Role gating & menu contract (§9 row 1, §7 [required], D-5) ─────────────
  describe('1. Role gating and menu contract (ticket §9 row 1, §7 [required])', () => {
    it('menu contract: id is campaigns and needs is MarketingCampaigns', () => {
      expect(menu, 'menu is defined').toBeDefined();
      expect(menu?.id).toBe('campaigns');
      expect(menu?.needs).toBe('MarketingCampaigns');
      expect(typeof menu?.label()).toBe('string');
      expect(typeof menu?.heading?.title()).toBe('string');
    });

    it('a non-MARKETING role (e.g. TELLER) has no campaigns menu in AdminShell nav', async () => {
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

      window.history.replaceState(null, '', '/ui/admin/#campaigns');
      dispose = render(() => <AdminShell api={fakeApi} menus={menus} />, container);
      await settle();

      expect(container.querySelector('a[href="#overview"], a[href="#transactions"]'), 'shell rendered nav').not.toBeNull();
      const navLink = container.querySelector('a[href="#campaigns"]');
      expect(navLink, 'TELLER sees no campaigns menu link').toBeNull();
    });

    it('MARKETING role sees campaigns menu in AdminShell nav', async () => {
      const marketingMe = meOf({
        id: 'mkt-1',
        role: 'MARKETING',
        branch: 'MAIN',
        name: 'Marketing Person',
      });

      const fakeApi = {
        async view<T>(name: string): Promise<{ version: number; changed: boolean; data?: T }> {
          if (name === 'me') return { version: 1, changed: true, data: marketingMe as unknown as T };
          if (name === 'admin_branches') return { version: 1, changed: true, data: { branches: [{ id: 'MAIN', name: 'Main' }] } as unknown as T };
          return { version: 1, changed: false };
        },
      } as unknown as ApiClient;

      window.history.replaceState(null, '', '/ui/admin/#campaigns');
      dispose = render(() => <AdminShell api={fakeApi} menus={menus} />, container);
      await settle();

      const navLink = container.querySelector('a[href="#campaigns"]');
      expect(navLink, 'MARKETING sees campaigns menu link').not.toBeNull();
    });
  });

  // ── 2. Data source: views and commands through generated clients (§7) ─────────
  describe('2. Data source conventions (ticket §7 [required])', () => {
    it('data comes only from views campaigns and campaign per generated contracts', async () => {
      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      const getListReq = sentRequests.find((r) => r.path === '/api/v3/views/campaigns');
      expect(getListReq, 'queries /api/v3/views/campaigns').toBeDefined();
      expect(getListReq?.search).toContain('from=2026-10-01');
      expect(getListReq?.search).toContain('to=2026-10-14');
    });
  });

  // ── 3. Empty campaign list shows create action, not error (§9 row 4) ───────────
  describe('3. Empty list handling (ticket §9 row 4)', () => {
    it('an empty campaign list shows the create action, not an error', async () => {
      listData = makeCampaignsData({ campaigns: [] });
      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      expect(container.textContent).not.toContain('Failed');
      expect(container.textContent).not.toContain('error');
      // Must offer create action / button
      const createBtn = container.querySelector('button, [role="button"], a');
      expect(createBtn, 'offers a create action when list is empty').not.toBeNull();
      expect(container.textContent?.toLowerCase()).toMatch(/create|สร้าง|ใหม่|add/i);
    });

    it('calls campaignCreate (POST /api/campaigns) when creating a campaign (§7 [required])', async () => {
      commandStatus = 200;
      commandOutput = { id: 'cmp-new', name: 'New Campaign', status: 'DRAFT' };

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      const createBtn = [...container.querySelectorAll<HTMLElement>('button, a, [role="button"]')].find((b) =>
        /create|สร้าง|new|เพิ่ม/i.test(b.textContent ?? ''),
      );
      expect(createBtn, 'create button exists on list').toBeDefined();
      createBtn?.click();
      await settle();

      // 1. name
      const nameInput = container.querySelector<HTMLInputElement>('input[name="name"]');
      expect(nameInput, 'name input exists in create form').not.toBeNull();
      nameInput!.value = 'New Campaign';
      nameInput!.dispatchEvent(new Event('input', { bubbles: true }));
      nameInput!.dispatchEvent(new Event('change', { bubbles: true }));

      // 2. startsAt (one input[name="startsAt"][type="date"])
      const startsAtInput = container.querySelector<HTMLInputElement>(
        'input[name="startsAt"][type="date"], input[name="startsAt"]',
      );
      expect(startsAtInput, 'startsAt input exists in create form').not.toBeNull();
      startsAtInput!.value = '2026-11-01';
      startsAtInput!.dispatchEvent(new Event('input', { bubbles: true }));
      startsAtInput!.dispatchEvent(new Event('change', { bubbles: true }));

      // 3. endsAt (one input[name="endsAt"][type="date"])
      const endsAtInput = container.querySelector<HTMLInputElement>(
        'input[name="endsAt"][type="date"], input[name="endsAt"]',
      );
      expect(endsAtInput, 'endsAt input exists in create form').not.toBeNull();
      endsAtInput!.value = '2026-11-30';
      endsAtInput!.dispatchEvent(new Event('input', { bubbles: true }));
      endsAtInput!.dispatchEvent(new Event('change', { bubbles: true }));

      // 4. budgetThb (decimal string)
      const budgetInput = container.querySelector<HTMLInputElement>('input[name="budgetThb"]');
      expect(budgetInput, 'budgetThb input exists in create form').not.toBeNull();
      budgetInput!.value = '50000.00';
      budgetInput!.dispatchEvent(new Event('input', { bubbles: true }));
      budgetInput!.dispatchEvent(new Event('change', { bubbles: true }));

      // 5. stackGroup
      const stackGroupInput = container.querySelector<HTMLInputElement | HTMLSelectElement>(
        'select[name="stackGroup"], input[name="stackGroup"]',
      );
      expect(stackGroupInput, 'stackGroup input exists in create form').not.toBeNull();
      stackGroupInput!.value = 'DEFAULT';
      stackGroupInput!.dispatchEvent(new Event('input', { bubbles: true }));
      stackGroupInput!.dispatchEvent(new Event('change', { bubbles: true }));

      // 6. priority (number)
      const priorityInput = container.querySelector<HTMLInputElement>('input[name="priority"]');
      expect(priorityInput, 'priority input exists in create form').not.toBeNull();
      priorityInput!.value = '10';
      priorityInput!.dispatchEvent(new Event('input', { bubbles: true }));
      priorityInput!.dispatchEvent(new Event('change', { bubbles: true }));

      // 7. branches (array of strings)
      const branchesInput = container.querySelector<HTMLInputElement | HTMLSelectElement>(
        'select[name="branches"], input[name="branches"]',
      );
      expect(branchesInput, 'branches input exists in create form').not.toBeNull();
      branchesInput!.value = 'MAIN';
      branchesInput!.dispatchEvent(new Event('input', { bubbles: true }));
      branchesInput!.dispatchEvent(new Event('change', { bubbles: true }));

      const saveBtn = [...container.querySelectorAll('button')].find((b) =>
        /create|สร้าง|save|บันทึก/i.test(b.textContent ?? ''),
      );
      expect(saveBtn, 'submit / save button exists in create form').toBeDefined();
      saveBtn?.click();
      await settle();

      const createReq = sentRequests.find((r) => r.method === 'POST' && r.path === '/api/campaigns');
      expect(createReq, 'calls POST /api/campaigns').toBeDefined();
      expect(createReq?.headers['idempotency-key'], 'includes Idempotency-Key header').toBeTruthy();
      expect(createReq?.body).toEqual({
        name: 'New Campaign',
        startsAt: '2026-11-01T00:00:00+07:00',
        endsAt: '2026-11-30T23:59:59+07:00',
        budgetThb: '50000.00',
        stackGroup: 'DEFAULT',
        priority: 10,
        branches: ['MAIN'],
      });
    });
  });

  // ── 4. Loading state & error state with retry ──────────────────────────────────
  describe('4. Loading and error states with retry action', () => {
    it('shows error message and retry button when view fails', async () => {
      listStatus = 500;
      listErrorCode = 'serverError';
      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      expect(container.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง');
      const retryBtn = [...container.querySelectorAll('button')].find((b) =>
        /retry|ลองใหม่|โหลดใหม่/i.test(b.textContent ?? ''),
      );
      expect(retryBtn, 'retry button is rendered on error').toBeDefined();

      // Clicking retry triggers a new fetch
      listStatus = 200;
      sentRequests = [];
      retryBtn?.click();
      await settle();

      expect(sentRequests.some((r) => r.path === '/api/v3/views/campaigns')).toBe(true);
    });
  });

  // ── 5. Money display from strings, never recomputed (§7 [required]) ─────────────
  describe('5. Money display from strings (ticket §7 [required])', () => {
    it('money is displayed from the strings the views return and is never computed', async () => {
      listData = makeCampaignsData({
        campaigns: [
          makeCampaignListItem({
            ...SAMPLE_LIST_ITEM,
            budgetThb: '987654.32',
            budgetUsedThb: '123456.78',
            budgetLeftThb: '864197.54',
            period: makeCampaignPeriodTotals({
              redemptions: 5,
              priceCostThb: '5555.55',
              rewardExpenseThb: '4444.44',
            }),
          }),
        ],
      });

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      expect(container.textContent).toContain('987,654.32');
      expect(container.textContent).toContain('123,456.78');
      expect(container.textContent).toContain('864,197.54');
    });
  });

  // ── 6. M01 list rendering & derived status (D-2) ──────────────────────────────
  describe('6. M01 list view and status rendering (D-2)', () => {
    it('renders list items with name, version, status, budget and uses', async () => {
      listData = makeCampaignsData({
        campaigns: [
          makeCampaignListItem({
            ...SAMPLE_LIST_ITEM,
            maxUses: 75,
          }),
        ],
      });
      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      expect(container.textContent).toContain('Golden October');
      const expectedStatus = table.th['status.active'] ?? 'ACTIVE';
      expect(container.textContent).toContain(expectedStatus);
      expect(container.textContent).toContain('42');
      expect(container.textContent).toContain('75');
    });

    it('calls campaignVersionNew (POST /api/campaigns/{id}/versions) to create a new draft version from an active version (D-3, §7 [required])', async () => {
      detailData = makeCampaignData({ campaign: SAMPLE_DETAIL }); // ACTIVE version
      commandStatus = 200;
      commandOutput = { versionId: 'ver-2', version: 2, status: 'DRAFT' };

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      const newVersionBtn = [...container.querySelectorAll('button')].find((b) =>
        /new version|สร้างเวอร์ชันใหม่|เวอร์ชันใหม่/i.test(b.textContent ?? ''),
      );
      expect(newVersionBtn, 'new version button exists on active campaign detail').toBeDefined();
      newVersionBtn?.click();
      await settle();

      const newVerReq = sentRequests.find(
        (r) => r.method === 'POST' && r.path === '/api/campaigns/cmp-1/versions',
      );
      expect(newVerReq, 'sends POST /api/campaigns/{id}/versions').toBeDefined();
      expect(newVerReq?.headers['idempotency-key'], 'carries Idempotency-Key header').toBeTruthy();
    });
  });

  // ── 7. Simulation required before submit (D-4, §7 [required], §9 row 2) ────────
  describe('7. Simulation required before submit (D-4, ticket §7 [required], §9 row 2)', () => {
    it('submit is disabled until current version has a fresh simulation', async () => {
      // Draft version with NO simulations: Submit must be disabled
      detailData = makeCampaignData({ campaign: DRAFT_DETAIL });

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      const submitBtn = [...container.querySelectorAll('button')].find((b) =>
        /submit|ส่งอนุมัติ/i.test(b.textContent ?? ''),
      );
      expect(submitBtn, 'submit button exists').toBeDefined();
      expect(submitBtn?.disabled, 'submit is disabled when no simulation exists').toBe(true);
    });

    it('a version edited after its simulation: Submit is disabled again (§9 row 2)', async () => {
      // Version with a simulation: Submit is enabled before edit
      detailData = makeCampaignData({ campaign: SIMULATED_DETAIL });

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      const submitBtn = [...container.querySelectorAll('button')].find((b) =>
        /submit|ส่งอนุมัติ/i.test(b.textContent ?? ''),
      );
      expect(submitBtn, 'submit button exists').toBeDefined();
      expect(submitBtn?.disabled, 'submit is enabled before editing because a simulation exists').toBe(false);

      // Edit a version field (e.g. budget or name)
      const input = (container.querySelector('input[name="budgetThb"]') ??
        container.querySelector('input[name="name"]') ??
        container.querySelector('input[data-field="budgetThb"]') ??
        container.querySelector('input')) as HTMLInputElement | null;
      expect(input, 'form input exists to edit').not.toBeNull();
      input!.value = '200000';
      input!.dispatchEvent(new Event('input', { bubbles: true }));
      await settle();

      expect(submitBtn?.disabled, 'editing after simulation disables submit again').toBe(true);
    });

    it('simulationRequired refusal from server is shown worded (§7 [required], §8)', async () => {
      detailData = makeCampaignData({ campaign: SIMULATED_DETAIL });
      commandStatus = 400;
      commandErrorCode = 'simulationRequired';

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      const submitBtn = [...container.querySelectorAll('button')].find((b) =>
        /submit|ส่งอนุมัติ/i.test(b.textContent ?? ''),
      );
      expect(submitBtn, 'submit button exists').toBeDefined();
      submitBtn?.removeAttribute('disabled');
      submitBtn?.click();
      await settle();

      expect(sentRequests.some((r) => r.method === 'POST' && r.path === '/api/campaigns/cmp-1/submit')).toBe(true);

      const expectedText = table.th['error.simulationRequired'] ?? table.th['err.simulationRequired'];
      expect(expectedText, 'table.th has key for simulationRequired').toBeDefined();
      expect(container.textContent).toContain(expectedText);
      expect(container.textContent).not.toContain('simulationRequired');
    });

    it('calls campaignSimulate (POST /api/campaigns/{id}/simulate) when simulating a version', async () => {
      detailData = makeCampaignData({ campaign: DRAFT_DETAIL });
      commandStatus = 200;
      commandOutput = {
        simulation: {
          id: 'sim-new',
          costSource: 'LEDGER',
          simulatedAt: '2026-10-14T10:00:00Z',
          input: {},
          result: {},
        },
      };

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      const simBtn = [...container.querySelectorAll('button')].find((b) =>
        /simulate|จำลอง/i.test(b.textContent ?? ''),
      );
      expect(simBtn, 'simulate button exists in detail').toBeDefined();
      simBtn?.click();
      await settle();

      expect(sentRequests.some((r) => r.method === 'POST' && r.path === '/api/campaigns/cmp-1/simulate')).toBe(true);
    });

    it('submit is disabled when a saved version is back in DRAFT with an old simulation listed (freshness only from status === SIMULATED) (D-4)', async () => {
      // Version has an old simulation listed, but status is back in DRAFT (e.g. after saved edit)
      detailData = makeCampaignData({ campaign: DRAFT_WITH_OLD_SIMULATION });

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      const submitBtn = [...container.querySelectorAll('button')].find((b) =>
        /submit|ส่งอนุมัติ/i.test(b.textContent ?? ''),
      );
      expect(submitBtn, 'submit button exists').toBeDefined();
      expect(submitBtn?.disabled, 'submit is disabled when status is DRAFT despite old simulations').toBe(true);
    });
  });

  // ── 8. Coupon codes shown once with copy action, never stored (D-9, §7, §9 row 3)
  describe('8. Coupon codes handling (D-9, ticket §7 [required], §9 row 3)', () => {
    it('coupon codes from couponIssue are shown once with a copy action (D-9, §7)', async () => {
      commandStatus = 200;
      const issueOutput = makeCouponIssueOutput({
        coupons: [makeIssuedCouponDto({ id: 'cpn-1', code: 'SAVE100' })],
      });
      commandOutput = issueOutput;

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      const ruleSelect = container.querySelector<HTMLSelectElement>('select[name="ruleId"]');
      if (ruleSelect) {
        ruleSelect.value = SAMPLE_RULE_ID;
        ruleSelect.dispatchEvent(new Event('input', { bubbles: true }));
        ruleSelect.dispatchEvent(new Event('change', { bubbles: true }));
      }
      const countInput = container.querySelector<HTMLInputElement>('input[name="count"]');
      if (countInput) {
        countInput.value = '1';
        countInput.dispatchEvent(new Event('input', { bubbles: true }));
        countInput.dispatchEvent(new Event('change', { bubbles: true }));
      }
      const expiryInput = container.querySelector<HTMLInputElement>(
        'input[name="expiresAt"][type="date"], input[name="expiresAt"]',
      );
      if (expiryInput) {
        expiryInput.value = '2026-12-31';
        expiryInput.dispatchEvent(new Event('input', { bubbles: true }));
        expiryInput.dispatchEvent(new Event('change', { bubbles: true }));
      }

      const issueBtn = [...container.querySelectorAll('button')].find((b) =>
        /issue|ออกคูปอง/i.test(b.textContent ?? ''),
      );
      expect(issueBtn, 'issue button exists in detail').toBeDefined();
      issueBtn?.click();
      await settle();

      expect(sentRequests.some((r) => r.method === 'POST' && r.path === '/api/campaigns/cmp-1/coupons')).toBe(true);
      expect(container.textContent).toContain('SAVE100');
      const copyBtn = [...container.querySelectorAll('button')].find((b) =>
        /copy|คัดลอก/i.test(b.textContent ?? ''),
      );
      expect(copyBtn, 'provides a copy action').toBeDefined();
    });

    it('a replayed couponIssue (same key, code key absent) shows no codes (§9 row 3)', async () => {
      commandStatus = 200;
      // Design §6.2: a replayed couponIssue removes the `code` key entirely, never "code": null
      const replayedOutput = {
        coupons: [{ id: 'cpn-1' }],
      };
      commandOutput = replayedOutput;

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      const ruleSelect = container.querySelector<HTMLSelectElement>('select[name="ruleId"]');
      if (ruleSelect) {
        ruleSelect.value = SAMPLE_RULE_ID;
        ruleSelect.dispatchEvent(new Event('input', { bubbles: true }));
        ruleSelect.dispatchEvent(new Event('change', { bubbles: true }));
      }
      const countInput = container.querySelector<HTMLInputElement>('input[name="count"]');
      if (countInput) {
        countInput.value = '1';
        countInput.dispatchEvent(new Event('input', { bubbles: true }));
        countInput.dispatchEvent(new Event('change', { bubbles: true }));
      }
      const expiryInput = container.querySelector<HTMLInputElement>(
        'input[name="expiresAt"][type="date"], input[name="expiresAt"]',
      );
      if (expiryInput) {
        expiryInput.value = '2026-12-31';
        expiryInput.dispatchEvent(new Event('input', { bubbles: true }));
        expiryInput.dispatchEvent(new Event('change', { bubbles: true }));
      }

      const issueBtn = [...container.querySelectorAll('button')].find((b) =>
        /issue|ออกคูปอง/i.test(b.textContent ?? ''),
      );
      expect(issueBtn, 'issue button exists in detail').toBeDefined();
      issueBtn?.click();
      await settle();

      expect(sentRequests.some((r) => r.method === 'POST' && r.path === '/api/campaigns/cmp-1/coupons')).toBe(true);
      expect(container.textContent).toContain('cpn-1');
      expect(container.textContent).not.toContain('SAVE100');
      expect(container.textContent).not.toContain('undefined');
      expect(container.textContent).not.toContain('null');
    });

    it('couponIssue sends RFC 3339 expiresAt from input and a ruleId chosen from version rules without invented id (§7 [required])', async () => {
      const customRuleId = '99999999-9999-9999-9999-999999999999';
      detailData = makeCampaignData({
        campaign: makeCampaignDetail({
          ...SAMPLE_DETAIL,
          versions: [
            makeCampaignVersionDetail({
              ...SAMPLE_DETAIL.versions[0],
              rules: [
                makeCampaignRuleDetail({
                  id: customRuleId,
                  ruleNo: 1,
                  benefitType: 'RATE_IMPROVEMENT',
                  benefitClass: 'PRICE',
                  requiresCoupon: true,
                  eligibility: {},
                  calculation: { spreadShare: '0.20' },
                  caps: {},
                }),
              ],
            }),
          ],
        }),
      });

      commandStatus = 200;
      commandOutput = { coupons: [{ id: 'cpn-1', code: 'PROMO2026' }] };

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      // Rule select - chosen from version rules (never invented id)
      const ruleSelect = container.querySelector<HTMLSelectElement>('select[name="ruleId"]');
      expect(ruleSelect, 'ruleId select exists in coupon issue form').not.toBeNull();
      ruleSelect!.value = customRuleId;
      ruleSelect!.dispatchEvent(new Event('input', { bubbles: true }));
      ruleSelect!.dispatchEvent(new Event('change', { bubbles: true }));

      // Count input (number)
      const countInput = container.querySelector<HTMLInputElement>('input[name="count"]');
      expect(countInput, 'count input exists in coupon issue form').not.toBeNull();
      countInput!.value = '5';
      countInput!.dispatchEvent(new Event('input', { bubbles: true }));
      countInput!.dispatchEvent(new Event('change', { bubbles: true }));

      // Expiry input (one input[name="expiresAt"][type="date"])
      const expiryInput = container.querySelector<HTMLInputElement>(
        'input[name="expiresAt"][type="date"]',
      );
      expect(expiryInput, 'expiresAt date input exists in coupon issue form').not.toBeNull();
      expiryInput!.value = '2026-12-31';
      expiryInput!.dispatchEvent(new Event('input', { bubbles: true }));
      expiryInput!.dispatchEvent(new Event('change', { bubbles: true }));

      const issueBtn = [...container.querySelectorAll('button')].find((b) =>
        /issue|ออกคูปอง/i.test(b.textContent ?? ''),
      );
      expect(issueBtn, 'issue button exists in detail').toBeDefined();
      issueBtn?.click();
      await settle();

      const issueReq = sentRequests.find(
        (r) => r.method === 'POST' && r.path === '/api/campaigns/cmp-1/coupons',
      );
      expect(issueReq, 'calls POST /api/campaigns/{id}/coupons').toBeDefined();
      expect(issueReq?.headers['idempotency-key'], 'includes Idempotency-Key header').toBeTruthy();
      expect(issueReq?.body).toEqual({
        ruleId: customRuleId,
        count: 5,
        expiresAt: '2026-12-31T23:59:59+07:00',
      });
    });
  });

  // ── 9. staleVersion on save reloads campaign and keeps unsaved fields (§9 row 5)
  describe('9. staleVersion handling on save (D-3, ticket §9 row 5, §8)', () => {
    it('staleVersion reloads campaign and keeps user unsaved fields visible (§9 row 5)', async () => {
      detailData = makeCampaignData({ campaign: DRAFT_DETAIL });
      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      // Edit a field
      const nameInput = container.querySelector('input[name="name"], input[data-field="name"], input[type="text"]') as HTMLInputElement;
      expect(nameInput, 'name input exists in detail form').not.toBeNull();
      nameInput.value = 'My Unsaved Edits';
      nameInput.dispatchEvent(new Event('input', { bubbles: true }));
      await settle();

      // Clear requests before save so we prove reload happened after 400
      sentRequests = [];
      commandStatus = 400;
      commandErrorCode = 'staleVersion';

      const saveBtn = [...container.querySelectorAll('button')].find((b) =>
        /save|บันทึก/i.test(b.textContent ?? ''),
      );
      expect(saveBtn, 'save button exists in detail').toBeDefined();
      saveBtn?.click();
      await settle();

      // Save command sent via PUT /api/campaigns/{id}/draft
      expect(sentRequests.some((r) => r.method === 'PUT' && r.path === '/api/campaigns/cmp-1/draft'), 'calls PUT /api/campaigns/{id}/draft').toBe(true);

      // View reloaded after 400
      expect(sentRequests.some((r) => r.path === '/api/v3/views/campaign'), 'campaign view reloaded after staleVersion').toBe(true);

      // Unsaved user edits remain visible in the form input
      expect(nameInput.value, 'user unsaved edit remains in input').toBe('My Unsaved Edits');

      // Worded error message must be shown
      const expectedText = table.th['error.staleVersion'] ?? table.th['err.staleVersion'];
      expect(expectedText, 'table.th has key for staleVersion').toBeDefined();
      expect(container.textContent).toContain(expectedText);
      expect(container.textContent).not.toContain('staleVersion');
    });
  });

  // ── 10. Idempotency-Key retained across retries (§8) ───────────────────────────
  describe('10. Idempotency-Key header retention across retries (§8)', () => {
    it('commands keep one Idempotency-Key per user action across retries', async () => {
      detailData = makeCampaignData({ campaign: DRAFT_DETAIL });
      commandStatus = 500;
      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      const actionBtn = [...container.querySelectorAll('button')].find((b) =>
        /save|บันทึก|submit|simulate/i.test(b.textContent ?? ''),
      );
      expect(actionBtn, 'action button exists in detail').toBeDefined();
      actionBtn?.click();
      await settle();

      const firstKey = sentRequests[sentRequests.length - 1]?.headers['idempotency-key'];
      expect(firstKey, 'first request includes Idempotency-Key header').toBeDefined();

      // Retry same action
      actionBtn?.click();
      await settle();

      const secondKey = sentRequests[sentRequests.length - 1]?.headers['idempotency-key'];
      expect(secondKey, 'retry reuses identical Idempotency-Key').toBe(firstKey);
    });
  });

  // ── 11. Worded error presentation for domain error codes (§8) ──────────────────
  describe('11. Worded errors for domain error codes (§8)', () => {
    const errorCodes = [
      'simulationRequired',
      'staleVersion',
      'approvalRequired',
      'validationFailed',
      'outOfScope',
      'idempotencyConflict',
      'couponInvalid',
    ];

    for (const code of errorCodes) {
      it(`shows worded message for error ${code}`, async () => {
        detailData = makeCampaignData({ campaign: DRAFT_DETAIL });
        commandStatus = 400;
        commandErrorCode = code;

        dispose = render(() => <Campaigns {...dummyProps()} />, container);
        await settle();

        await openCampaign('cmp-1');

        const saveBtn = [...container.querySelectorAll('button')].find((b) =>
          /save|บันทึก/i.test(b.textContent ?? ''),
        );
        expect(saveBtn, 'save button exists in detail').toBeDefined();
        saveBtn?.click();
        await settle();

        const expectedText = table.th[`error.${code}`] ?? table.th[`err.${code}`];
        expect(expectedText, `table.th has key error.${code} or err.${code}`).toBeDefined();
        expect(container.textContent).toContain(expectedText);
        expect(container.textContent).not.toContain(code); // must not show raw error code
      });
    }
  });

  // ── 12. Pause and Resume actions (§7) ──────────────────────────────────────────
  describe('12. Pause and Resume commands (§7)', () => {
    it('calls campaignPause when pausing an active campaign', async () => {
      commandStatus = 200;
      const pauseOutput = makeCampaignPauseOutput({ campaignId: 'cmp-1', status: 'PAUSED' });
      commandOutput = pauseOutput;

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      const pauseBtn = [...container.querySelectorAll('button')].find((b) =>
        /pause|พัก|หยุด/i.test(b.textContent ?? ''),
      );
      expect(pauseBtn, 'pause button exists').toBeDefined();
      pauseBtn?.click();
      await settle();

      expect(sentRequests.some((r) => r.method === 'POST' && r.path === '/api/campaigns/cmp-1/pause')).toBe(true);
    });

    it('calls campaignResume when resuming a paused campaign', async () => {
      // Mock PAUSED status for both list and detail
      listData = makeCampaignsData({
        campaigns: [
          makeCampaignListItem({
            ...SAMPLE_LIST_ITEM,
            status: 'PAUSED',
          }),
        ],
      });
      detailData = makeCampaignData({
        campaign: makeCampaignDetail({
          ...SAMPLE_DETAIL,
          versions: [
            makeCampaignVersionDetail({
              ...SAMPLE_DETAIL.versions[0],
              status: 'PAUSED',
            }),
          ],
        }),
      });

      commandStatus = 200;
      const resumeOutput = makeCampaignResumeOutput({ campaignId: 'cmp-1', status: 'ACTIVE' });
      commandOutput = resumeOutput;

      dispose = render(() => <Campaigns {...dummyProps()} />, container);
      await settle();

      await openCampaign('cmp-1');

      const resumeBtn = [...container.querySelectorAll('button')].find((b) =>
        /resume|ทำงานต่อ|กลับมาใช้/i.test(b.textContent ?? ''),
      );
      expect(resumeBtn, 'resume button exists on paused campaign').toBeDefined();
      resumeBtn?.click();
      await settle();

      expect(sentRequests.some((r) => r.method === 'POST' && r.path === '/api/campaigns/cmp-1/resume')).toBe(true);
    });
  });

  // ── 13. TH/EN key parity ──────────────────────────────────────────────────────
  describe('13. i18n key parity', () => {
    it('th and en keys have identical sets and are non-empty', () => {
      const thKeys = Object.keys(table.th).sort();
      const enKeys = Object.keys(table.en).sort();
      expect(thKeys).toEqual(enKeys);
      expect(thKeys.length, 'i18n keys are present').toBeGreaterThan(0);
    });
  });
});
