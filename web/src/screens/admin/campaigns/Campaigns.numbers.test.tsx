import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeCampaignApprovalDetail,
  makeCampaignData,
  makeCampaignDetail,
  makeCampaignListItem,
  makeCampaignPeriodTotals,
  makeCampaignRuleDetail,
  makeCampaignSimulationDetail,
  makeCampaignVersionDetail,
  makeCampaignsData,
  makeCouponIssueOutput,
  makeIssuedCouponDto,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { Campaigns } from './Campaigns';
import * as table from './i18n';

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

describe('Admin Campaigns coupon count and priority validation (#2184)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];

  let listData = makeCampaignsData({ campaigns: [SAMPLE_LIST_ITEM] });
  let detailData = makeCampaignData({ campaign: SAMPLE_DETAIL });
  let commandStatus = 200;
  let commandOutput: unknown = {};

  async function openCampaign(campaignId = 'cmp-1'): Promise<void> {
    const itemEl = container.querySelector(`[data-campaign-id="${campaignId}"]`);
    expect(itemEl, `campaign ${campaignId} row exists on list`).not.toBeNull();
    (itemEl as HTMLElement).click();
    await settle();
  }

  function fillCreateFormDefaults(): void {
    const nameInput = container.querySelector<HTMLInputElement>('input[name="name"]');
    expect(nameInput).not.toBeNull();
    nameInput!.value = 'New Campaign';
    nameInput!.dispatchEvent(new Event('input', { bubbles: true }));
    nameInput!.dispatchEvent(new Event('change', { bubbles: true }));

    const startsAtInput = container.querySelector<HTMLInputElement>('input[name="startsAt"]');
    expect(startsAtInput).not.toBeNull();
    startsAtInput!.value = '2026-11-01';
    startsAtInput!.dispatchEvent(new Event('input', { bubbles: true }));
    startsAtInput!.dispatchEvent(new Event('change', { bubbles: true }));

    const endsAtInput = container.querySelector<HTMLInputElement>('input[name="endsAt"]');
    expect(endsAtInput).not.toBeNull();
    endsAtInput!.value = '2026-11-30';
    endsAtInput!.dispatchEvent(new Event('input', { bubbles: true }));
    endsAtInput!.dispatchEvent(new Event('change', { bubbles: true }));

    const budgetInput = container.querySelector<HTMLInputElement>('input[name="budgetThb"]');
    expect(budgetInput).not.toBeNull();
    budgetInput!.value = '50000.00';
    budgetInput!.dispatchEvent(new Event('input', { bubbles: true }));
    budgetInput!.dispatchEvent(new Event('change', { bubbles: true }));
  }

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];

    listData = makeCampaignsData({ campaigns: [SAMPLE_LIST_ITEM] });
    detailData = makeCampaignData({ campaign: SAMPLE_DETAIL });
    commandStatus = 200;
    commandOutput = {};

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
        return new Response(JSON.stringify({ version: 1, changed: true, data: listData }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      if (u.pathname === '/api/v3/views/campaign') {
        return new Response(JSON.stringify({ version: 1, changed: true, data: detailData }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      return new Response(JSON.stringify(commandOutput), {
        status: commandStatus,
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

  it("row 1: Count '' -> Issue sends no POST and shows validationFailed error", async () => {
    dispose = render(() => <Campaigns {...dummyProps()} />, container);
    await settle();
    await openCampaign('cmp-1');

    const countInput = container.querySelector<HTMLInputElement>('input[name="count"]');
    expect(countInput).not.toBeNull();
    countInput!.value = '';
    countInput!.dispatchEvent(new Event('input', { bubbles: true }));
    countInput!.dispatchEvent(new Event('change', { bubbles: true }));

    const issueBtn = [...container.querySelectorAll('button')].find((b) =>
      /issue|ออกคูปอง/i.test(b.textContent ?? ''),
    );
    expect(issueBtn).toBeDefined();
    issueBtn?.click();
    await settle();

    const issueReqs = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === '/api/campaigns/cmp-1/coupons',
    );
    expect(issueReqs).toHaveLength(0);
    expect(container.textContent).toContain(table.th['error.validationFailed']);
    expect(countInput!.value).toBe('');
  });

  it("row 2: Count '0' -> Issue sends no POST and shows validationFailed error", async () => {
    dispose = render(() => <Campaigns {...dummyProps()} />, container);
    await settle();
    await openCampaign('cmp-1');

    const countInput = container.querySelector<HTMLInputElement>('input[name="count"]');
    expect(countInput).not.toBeNull();
    countInput!.value = '0';
    countInput!.dispatchEvent(new Event('input', { bubbles: true }));
    countInput!.dispatchEvent(new Event('change', { bubbles: true }));

    const issueBtn = [...container.querySelectorAll('button')].find((b) =>
      /issue|ออกคูปอง/i.test(b.textContent ?? ''),
    );
    expect(issueBtn).toBeDefined();
    issueBtn?.click();
    await settle();

    const issueReqs = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === '/api/campaigns/cmp-1/coupons',
    );
    expect(issueReqs).toHaveLength(0);
    expect(container.textContent).toContain(table.th['error.validationFailed']);
  });

  it("row 3: Count '5' -> Issue sends POST with count: 5 after clearing count keeps raw empty string", async () => {
    const issueOutput = makeCouponIssueOutput({
      coupons: [makeIssuedCouponDto({ id: 'cpn-1', code: 'PROMO5' })],
    });
    commandOutput = issueOutput;

    dispose = render(() => <Campaigns {...dummyProps()} />, container);
    await settle();
    await openCampaign('cmp-1');

    const countInput = container.querySelector<HTMLInputElement>('input[name="count"]');
    expect(countInput).not.toBeNull();

    // Clearing the field preserves raw '' rather than coercing to '0'
    countInput!.value = '';
    countInput!.dispatchEvent(new Event('input', { bubbles: true }));
    expect(countInput!.value).toBe('');

    countInput!.value = '5';
    countInput!.dispatchEvent(new Event('input', { bubbles: true }));
    countInput!.dispatchEvent(new Event('change', { bubbles: true }));

    const issueBtn = [...container.querySelectorAll('button')].find((b) =>
      /issue|ออกคูปอง/i.test(b.textContent ?? ''),
    );
    expect(issueBtn).toBeDefined();
    issueBtn?.click();
    await settle();

    const issueReqs = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === '/api/campaigns/cmp-1/coupons',
    );
    expect(issueReqs).toHaveLength(1);
    expect(issueReqs[0].body).toMatchObject({ count: 5 });
  });

  it('row 4: Create (and Save) with priority cleared sends no request and shows validationFailed error', async () => {
    dispose = render(() => <Campaigns {...dummyProps()} />, container);
    await settle();

    const createBtn = [...container.querySelectorAll<HTMLElement>('button')].find((b) =>
      /create|สร้าง/i.test(b.textContent ?? ''),
    );
    expect(createBtn).toBeDefined();
    createBtn?.click();
    await settle();

    fillCreateFormDefaults();

    const priorityInput = container.querySelector<HTMLInputElement>('input[name="priority"]');
    expect(priorityInput).not.toBeNull();
    priorityInput!.value = '';
    priorityInput!.dispatchEvent(new Event('input', { bubbles: true }));
    priorityInput!.dispatchEvent(new Event('change', { bubbles: true }));

    const submitCreateBtn = [...container.querySelectorAll('button')].find((b) =>
      /create|สร้าง/i.test(b.textContent ?? ''),
    );
    expect(submitCreateBtn).toBeDefined();
    submitCreateBtn?.click();
    await settle();

    expect(sentRequests.filter((r) => r.method === 'POST' && r.path === '/api/campaigns')).toHaveLength(0);
    expect(container.textContent).toContain(table.th['error.validationFailed']);
    expect(priorityInput!.value).toBe('');

    // Also verify detail Save refuses cleared priority
    const backBtn = [...container.querySelectorAll('button')].find((b) =>
      /back|กลับ/i.test(b.textContent ?? ''),
    );
    backBtn?.click();
    await settle();
    await openCampaign('cmp-1');

    const detailPriorityInput = container.querySelector<HTMLInputElement>('input[name="priority"]');
    expect(detailPriorityInput).not.toBeNull();
    detailPriorityInput!.value = '';
    detailPriorityInput!.dispatchEvent(new Event('input', { bubbles: true }));

    const saveBtn = [...container.querySelectorAll('button')].find((b) =>
      /save|บันทึก/i.test(b.textContent ?? ''),
    );
    saveBtn?.click();
    await settle();

    expect(sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/campaigns/cmp-1/draft')).toHaveLength(0);
    expect(container.textContent).toContain(table.th['error.validationFailed']);
    expect(detailPriorityInput!.value).toBe('');
  });

  it('row 5: Create with the default priority sends priority: 10 after refusing cleared priority', async () => {
    commandOutput = { id: 'cmp-new', name: 'New Campaign', status: 'DRAFT' };

    dispose = render(() => <Campaigns {...dummyProps()} />, container);
    await settle();

    const openCreateBtn = [...container.querySelectorAll<HTMLElement>('button')].find((b) =>
      /create|สร้าง/i.test(b.textContent ?? ''),
    );
    openCreateBtn?.click();
    await settle();

    fillCreateFormDefaults();

    // First clear priority and attempt Create -> refused with no POST
    const priorityInput = container.querySelector<HTMLInputElement>('input[name="priority"]');
    expect(priorityInput).not.toBeNull();
    priorityInput!.value = '';
    priorityInput!.dispatchEvent(new Event('input', { bubbles: true }));
    priorityInput!.dispatchEvent(new Event('change', { bubbles: true }));

    const submitCreateBtn = [...container.querySelectorAll('button')].find((b) =>
      /create|สร้าง/i.test(b.textContent ?? ''),
    );
    submitCreateBtn?.click();
    await settle();
    expect(sentRequests.filter((r) => r.method === 'POST' && r.path === '/api/campaigns')).toHaveLength(0);

    // Re-open Create so the untouched default priority ('10') is used
    const backBtn = [...container.querySelectorAll('button')].find((b) =>
      /back|กลับ/i.test(b.textContent ?? ''),
    );
    backBtn?.click();
    await settle();

    const reopenCreateBtn = [...container.querySelectorAll<HTMLElement>('button')].find((b) =>
      /create|สร้าง/i.test(b.textContent ?? ''),
    );
    reopenCreateBtn?.click();
    await settle();

    fillCreateFormDefaults();

    const submitDefaultBtn = [...container.querySelectorAll('button')].find((b) =>
      /create|สร้าง/i.test(b.textContent ?? ''),
    );
    submitDefaultBtn?.click();
    await settle();

    const createReqs = sentRequests.filter((r) => r.method === 'POST' && r.path === '/api/campaigns');
    expect(createReqs).toHaveLength(1);
    expect(createReqs[0].body).toMatchObject({ priority: 10 });
  });

  it('row 6: Edit after simulate disables Submit and refuses Save when priority is cleared', async () => {
    detailData = { campaign: SIMULATED_DETAIL };

    dispose = render(() => <Campaigns {...dummyProps()} />, container);
    await settle();
    await openCampaign('cmp-1');

    const submitBtn = [...container.querySelectorAll('button')].find((b) =>
      /submit|ส่งอนุมัติ/i.test(b.textContent ?? ''),
    );
    expect(submitBtn).toBeDefined();
    expect(submitBtn?.disabled).toBe(false);

    const priorityInput = container.querySelector<HTMLInputElement>('input[name="priority"]');
    expect(priorityInput).not.toBeNull();
    priorityInput!.value = '';
    priorityInput!.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();

    expect(submitBtn?.disabled).toBe(true);

    // Attempting to Save with cleared priority must fail validation and keep Submit disabled
    const saveBtn = [...container.querySelectorAll('button')].find((b) =>
      /save|บันทึก/i.test(b.textContent ?? ''),
    );
    saveBtn?.click();
    await settle();

    expect(sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/campaigns/cmp-1/draft')).toHaveLength(0);
    expect(submitBtn?.disabled).toBe(true);
    expect(priorityInput!.value).toBe('');
  });

  it("edge case 1: Priority '0' typed explicitly is allowed and sent as 0 (unlike cleared '')", async () => {
    commandOutput = { id: 'cmp-zero', name: 'Zero Priority Campaign', status: 'DRAFT' };

    dispose = render(() => <Campaigns {...dummyProps()} />, container);
    await settle();

    const createBtn = [...container.querySelectorAll<HTMLElement>('button')].find((b) =>
      /create|สร้าง/i.test(b.textContent ?? ''),
    );
    createBtn?.click();
    await settle();

    fillCreateFormDefaults();

    const priorityInput = container.querySelector<HTMLInputElement>('input[name="priority"]');
    expect(priorityInput).not.toBeNull();

    // Clearing priority is refused
    priorityInput!.value = '';
    priorityInput!.dispatchEvent(new Event('input', { bubbles: true }));
    priorityInput!.dispatchEvent(new Event('change', { bubbles: true }));

    const submitBtn = [...container.querySelectorAll('button')].find((b) =>
      /create|สร้าง/i.test(b.textContent ?? ''),
    );
    submitBtn?.click();
    await settle();
    expect(sentRequests.filter((r) => r.method === 'POST' && r.path === '/api/campaigns')).toHaveLength(0);

    // Explicitly typing '0' is allowed and sent as 0
    priorityInput!.value = '0';
    priorityInput!.dispatchEvent(new Event('input', { bubbles: true }));
    priorityInput!.dispatchEvent(new Event('change', { bubbles: true }));

    submitBtn?.click();
    await settle();

    const createReqs = sentRequests.filter((r) => r.method === 'POST' && r.path === '/api/campaigns');
    expect(createReqs).toHaveLength(1);
    expect(createReqs[0].body).toMatchObject({ priority: 0 });
  });

  it("edge case 2: Count '2.5' is refused with validationFailed and sends no POST", async () => {
    dispose = render(() => <Campaigns {...dummyProps()} />, container);
    await settle();
    await openCampaign('cmp-1');

    const countInput = container.querySelector<HTMLInputElement>('input[name="count"]');
    expect(countInput).not.toBeNull();
    countInput!.value = '2.5';
    countInput!.dispatchEvent(new Event('input', { bubbles: true }));
    countInput!.dispatchEvent(new Event('change', { bubbles: true }));

    const issueBtn = [...container.querySelectorAll('button')].find((b) =>
      /issue|ออกคูปอง/i.test(b.textContent ?? ''),
    );
    expect(issueBtn).toBeDefined();
    issueBtn?.click();
    await settle();

    const issueReqs = sentRequests.filter(
      (r) => r.method === 'POST' && r.path === '/api/campaigns/cmp-1/coupons',
    );
    expect(issueReqs).toHaveLength(0);
    expect(container.textContent).toContain(table.th['error.validationFailed']);
  });
});
