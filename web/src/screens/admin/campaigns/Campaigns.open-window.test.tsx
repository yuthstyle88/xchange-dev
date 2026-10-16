// #2441: opening campaign B after campaign A must never show or send A's values for B,
// and a write's answer for A must not land on B's page (addendum from #2438).
// Render and fetch mock copied from `Campaigns.inflight.test.tsx`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Campaigns } from './Campaigns';
import * as table from './i18n';

const LIST_VIEW = '/api/v3/views/campaigns';
const DETAIL_VIEW = '/api/v3/views/campaign';

type Reply = { status: number; body: unknown } | 'network';

interface SentWrite {
  method: string;
  path: string;
  body: string;
}

interface Held {
  path: string;
  release: (reply: Reply) => void;
}

interface Fixture {
  name: string;
  status: string;
  startsAt: string;
  endsAt: string;
  budgetThb: string;
  stackGroup: string;
  priority: number;
  branches: string[];
}

const json = (status: number, body: unknown): Response =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

const view = (data: unknown): Response => json(200, { version: 1, changed: true, data });

function freshFixtures(): Record<string, Fixture> {
  return {
    'cmp-1': {
      name: 'Golden October',
      status: 'ACTIVE',
      startsAt: '2026-10-01T00:00:00+07:00',
      endsAt: '2026-10-31T23:59:59+07:00',
      budgetThb: '100000.00',
      stackGroup: 'GOLD',
      priority: 10,
      branches: ['MAIN'],
    },
    'cmp-2': {
      name: 'Silver November',
      status: 'DRAFT',
      startsAt: '2026-11-01T00:00:00+07:00',
      endsAt: '2026-11-30T23:59:59+07:00',
      budgetThb: '55555.00',
      stackGroup: 'SILVER',
      priority: 3,
      branches: ['NORTH'],
    },
    'cmp-3': {
      name: 'Bronze December',
      status: 'DRAFT',
      startsAt: '2026-12-01T00:00:00+07:00',
      endsAt: '2026-12-31T23:59:59+07:00',
      budgetThb: '777.00',
      stackGroup: 'BRONZE',
      priority: 1,
      branches: ['SOUTH'],
    },
  };
}

function listItem(id: string, f: Fixture) {
  return {
    id,
    name: f.name,
    versionId: `ver-${id}`,
    version: 1,
    status: f.status,
    startsAt: f.startsAt,
    endsAt: f.endsAt,
    budgetThb: f.budgetThb,
    budgetUsedThb: '0.00',
    budgetLeftThb: f.budgetThb,
    uses: 0,
    maxUses: 100,
    period: { redemptions: 0, priceCostThb: '0.00', rewardExpenseThb: '0.00' },
  };
}

function detail(id: string, f: Fixture) {
  return {
    campaign: {
      id,
      name: f.name,
      currentVersionId: `ver-${id}`,
      versions: [
        {
          id: `ver-${id}`,
          version: 1,
          status: f.status,
          startsAt: f.startsAt,
          endsAt: f.endsAt,
          budgetThb: f.budgetThb,
          maxUses: 100,
          stackGroup: f.stackGroup,
          priority: f.priority,
          branches: [...f.branches],
          rules: [
            {
              id: `rule-${id}`,
              ruleNo: 1,
              benefitType: 'RATE_BONUS',
              benefitClass: 'PRICE',
              requiresCoupon: true,
              eligibility: {},
              calculation: {},
              caps: {},
            },
          ],
          simulations: [],
          approval: null,
        },
      ],
    },
  };
}

describe('#2441 Campaigns: opening another campaign never shows or sends the previous one', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let detailReads: string[];
  let held: Held[];
  let holdPaths: Set<string>;
  let holdDetail: Set<string>;
  let failDetail: Set<string>;
  let fixtures: Record<string, Fixture>;
  let setBranch: Setter<string>;

  beforeEach(() => {
    setLang('en');
    writes = [];
    detailReads = [];
    held = [];
    holdPaths = new Set();
    holdDetail = new Set();
    failDetail = new Set();
    fixtures = freshFixtures();
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET') {
        if (u.pathname === LIST_VIEW) {
          return view({ campaigns: Object.entries(fixtures).map(([id, f]) => listItem(id, f)) });
        }
        if (u.pathname === DETAIL_VIEW) {
          const id = u.searchParams.get('id') ?? 'cmp-1';
          detailReads.push(id);
          if (holdDetail.has(id)) {
            const reply = await new Promise<Reply>((resolve) =>
              held.push({ path: `detail:${id}`, release: resolve }),
            );
            if (reply === 'network') throw new TypeError('Failed to fetch');
            return json(reply.status, reply.body);
          }
          if (failDetail.has(id)) {
            return json(500, { error: { code: 'internal', message: 'boom' } });
          }
          return view(detail(id, fixtures[id]!));
        }
        return json(200, { version: 1, changed: false });
      }

      writes.push({ method, path: u.pathname, body: typeof init?.body === 'string' ? init.body : '' });
      let reply: Reply;
      if (holdPaths.has(u.pathname)) {
        reply = await new Promise<Reply>((resolve) => held.push({ path: u.pathname, release: resolve }));
      } else {
        reply = { status: 200, body: okBody(u.pathname) };
      }
      if (reply === 'network') throw new TypeError('Failed to fetch');
      return json(reply.status, reply.body);
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
  });

  function okBody(path: string): unknown {
    if (path.endsWith('/coupons')) {
      return { coupons: [{ id: 'cpn-a-1', code: 'GOLD-A-0001' }] };
    }
    return { versionId: 'ver-x', version: 1 };
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mountScreen(): Promise<void> {
    const [branch, set] = createSignal('MAIN');
    setBranch = set;
    const [period] = createSignal({ from: '2026-10-01', to: '2026-12-31' });
    const [shown] = createSignal(true);
    dispose = render(() => <Campaigns branch={branch} period={period} shown={shown} />, root);
    await settle();
  }

  async function openCampaign(id: string): Promise<void> {
    const row = root.querySelector(`[data-campaign-id="${id}"]`) as HTMLElement | null;
    expect(row, `campaign row ${id} exists`).not.toBeNull();
    row!.click();
    await settle();
  }

  async function back(): Promise<void> {
    button('back').click();
    await settle();
  }

  function button(textKey: string): HTMLButtonElement {
    const btn = Array.from(root.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === table.en[textKey],
    );
    expect(btn, `${textKey} button exists`).toBeDefined();
    return btn!;
  }

  function field(name: string): string {
    const el = root.querySelector(`.xc-campaign-detail [name="${name}"]`) as HTMLInputElement | HTMLSelectElement | null;
    expect(el, `field ${name} exists`).not.toBeNull();
    return el!.value;
  }

  function release(path: string, reply: Reply): void {
    for (const h of held.filter((x) => x.path === path)) h.release(reply);
    held = held.filter((x) => x.path !== path);
  }

  function sent(method: string, path: string): SentWrite[] {
    return writes.filter((w) => w.method === method && w.path === path);
  }

  function errorText(): string {
    return Array.from(root.querySelectorAll('.xc-campaigns-error'))
      .map((el) => el.textContent ?? '')
      .join('')
      .trim();
  }

  function expectNoValuesOf(id: string): void {
    const f = fixtures[id]!;
    expect(field('name'), 'name').not.toBe(f.name);
    expect(field('budgetThb'), 'budgetThb').not.toBe(f.budgetThb);
    expect(field('startsAt'), 'startsAt').not.toBe(f.startsAt);
    expect(field('endsAt'), 'endsAt').not.toBe(f.endsAt);
    expect(field('stackGroup'), 'stackGroup').not.toBe(f.stackGroup);
    expect(field('priority'), 'priority').not.toBe(String(f.priority));
    expect(field('ruleId'), 'coupon rule').not.toBe(`rule-${id}`);
    expect(field('expiresAt'), 'coupon expiry').not.toBe(f.endsAt.slice(0, 10));
  }

  function expectValuesOf(id: string): void {
    const f = fixtures[id]!;
    expect(field('name'), 'name').toBe(f.name);
    expect(field('budgetThb'), 'budgetThb').toBe(f.budgetThb);
    expect(field('startsAt'), 'startsAt').toBe(f.startsAt);
    expect(field('endsAt'), 'endsAt').toBe(f.endsAt);
    expect(field('stackGroup'), 'stackGroup').toBe(f.stackGroup);
    expect(field('priority'), 'priority').toBe(String(f.priority));
  }

  async function expectSaveSendsValuesOf(id: string): Promise<void> {
    const path = `/api/campaigns/${id}/draft`;
    const before = sent('PUT', path).length;
    expect(button('save').disabled, 'save enabled').toBe(false);
    button('save').click();
    await settle();
    const saves = sent('PUT', path);
    expect(saves.length, `one save for ${id}`).toBe(before + 1);
    const f = fixtures[id]!;
    expect(JSON.parse(saves[saves.length - 1]!.body)).toEqual({
      campaignId: id,
      startsAt: f.startsAt,
      endsAt: f.endsAt,
      budgetThb: f.budgetThb,
      stackGroup: f.stackGroup,
      priority: f.priority,
      branches: f.branches,
    });
  }

  it('row 1: open A (form filled), Back, open B: before B answers the form is empty and Save / Issue coupons are disabled', async () => {
    await mountScreen();
    await openCampaign('cmp-1');
    expectValuesOf('cmp-1');

    await back();
    holdDetail.add('cmp-2');
    await openCampaign('cmp-2');

    expectNoValuesOf('cmp-1');
    expect(field('name')).toBe('');
    expect(field('budgetThb')).toBe('');
    expect(field('startsAt')).toBe('');
    expect(field('endsAt')).toBe('');
    expect(field('expiresAt')).toBe('');
    expect(button('save').disabled, 'save while B loads').toBe(true);
    expect(button('issueCoupons').disabled, 'issue coupons while B loads').toBe(true);

    button('save').click();
    button('issueCoupons').click();
    await settle();
    expect(writes, 'nothing sent before B answers').toEqual([]);
  });

  it('row 2: open B and B\'s detail read fails: no A values shown; Save and Issue coupons disabled', async () => {
    await mountScreen();
    await openCampaign('cmp-1');
    expectValuesOf('cmp-1');

    await back();
    failDetail.add('cmp-2');
    await openCampaign('cmp-2');

    expect(errorText(), 'detail error shown').not.toBe('');
    expectNoValuesOf('cmp-1');
    expect(button('save').disabled, 'save after failed read').toBe(true);
    expect(button('issueCoupons').disabled, 'issue coupons after failed read').toBe(true);

    button('save').click();
    button('issueCoupons').click();
    await settle();
    expect(writes, 'nothing sent for B with A values').toEqual([]);
  });

  it('row 3: branch switch while A\'s detail read is in flight, then open C: C\'s form never shows A\'s values', async () => {
    await mountScreen();
    holdDetail.add('cmp-1');
    await openCampaign('cmp-1');

    setBranch('NORTH');
    await settle();
    release('detail:cmp-1', { status: 200, body: { version: 1, changed: true, data: detail('cmp-1', fixtures['cmp-1']!) } });
    await settle();

    holdDetail.add('cmp-3');
    await openCampaign('cmp-3');
    expectNoValuesOf('cmp-1');
    expect(button('save').disabled, 'save while C loads').toBe(true);

    release('detail:cmp-3', { status: 200, body: { version: 1, changed: true, data: detail('cmp-3', fixtures['cmp-3']!) } });
    await settle();
    expectValuesOf('cmp-3');
    await expectSaveSendsValuesOf('cmp-3');
  });

  it('row 4: open B, B answers: B\'s values are shown and Save sends B\'s values (a late Save answer for A does not refill the form)', async () => {
    await mountScreen();
    await openCampaign('cmp-1');

    holdPaths.add('/api/campaigns/cmp-1/draft');
    button('save').click();
    await settle();

    await back();
    await openCampaign('cmp-2');
    expectValuesOf('cmp-2');

    holdPaths.delete('/api/campaigns/cmp-1/draft');
    release('/api/campaigns/cmp-1/draft', { status: 200, body: okBody('/api/campaigns/cmp-1/draft') });
    await settle();

    expectValuesOf('cmp-2');
    await expectSaveSendsValuesOf('cmp-2');
  });

  it('addendum: Pause A in flight, Back, open B, A\'s pause answers: B\'s detail stays, no read of A is applied, Save sends B\'s values', async () => {
    const path = '/api/campaigns/cmp-1/pause';
    holdPaths.add(path);
    await mountScreen();
    await openCampaign('cmp-1');

    button('pause').click();
    await settle();
    await back();
    await openCampaign('cmp-2');
    expectValuesOf('cmp-2');

    const readsBefore = detailReads.length;
    holdPaths.delete(path);
    release(path, { status: 200, body: okBody(path) });
    await settle();

    expect(detailReads.slice(readsBefore), 'no read of A after its pause answers').not.toContain('cmp-1');
    expectValuesOf('cmp-2');
    expect(root.querySelector('[data-testid="campaign-status"]')?.textContent).toBe(table.en['status.draft']);
    await expectSaveSendsValuesOf('cmp-2');
  });

  it('addendum: Issue coupons on A in flight, open B, A answers: B\'s page shows no coupons from A', async () => {
    const path = '/api/campaigns/cmp-1/coupons';
    holdPaths.add(path);
    await mountScreen();
    await openCampaign('cmp-1');

    button('issueCoupons').click();
    await settle();
    await back();
    await openCampaign('cmp-2');

    holdPaths.delete(path);
    release(path, { status: 200, body: okBody(path) });
    await settle();

    expect(root.querySelector('.xc-issued-coupons'), 'no issued list on B').toBeNull();
    expect(root.textContent).not.toContain('GOLD-A-0001');
    expectValuesOf('cmp-2');
  });

  it('edge: re-opening the same campaign re-reads it and shows fresh values, never the old ones while it reads', async () => {
    await mountScreen();
    await openCampaign('cmp-1');
    expectValuesOf('cmp-1');
    await back();

    const old = { ...fixtures['cmp-1']! };
    fixtures['cmp-1'] = { ...old, name: 'Golden October v2', budgetThb: '120000.00' };
    holdDetail.add('cmp-1');
    await openCampaign('cmp-1');

    expect(field('name'), 'old name not shown while re-reading').not.toBe(old.name);
    expect(field('budgetThb'), 'old budget not shown while re-reading').not.toBe(old.budgetThb);
    expect(button('save').disabled, 'save while re-reading').toBe(true);

    release('detail:cmp-1', { status: 200, body: { version: 1, changed: true, data: detail('cmp-1', fixtures['cmp-1']!) } });
    await settle();
    expect(field('name')).toBe('Golden October v2');
    expect(field('budgetThb')).toBe('120000.00');
    expect(button('save').disabled).toBe(false);
  });

  it('edge: Submit keeps isSubmitDisabled(): a late Simulate answer for A does not enable Submit on a DRAFT B; a SIMULATED B enables it', async () => {
    const path = '/api/campaigns/cmp-1/simulate';
    fixtures['cmp-1']!.status = 'DRAFT';
    holdPaths.add(path);
    await mountScreen();
    await openCampaign('cmp-1');
    expect(button('submit').disabled, 'submit on DRAFT A').toBe(true);

    button('simulate').click();
    await settle();
    await back();
    await openCampaign('cmp-2');
    expect(button('submit').disabled, 'submit on DRAFT B').toBe(true);

    fixtures['cmp-1']!.status = 'SIMULATED';
    holdPaths.delete(path);
    release(path, { status: 200, body: okBody(path) });
    await settle();
    expect(root.querySelector('[data-testid="campaign-status"]')?.textContent).toBe(table.en['status.draft']);
    expect(button('submit').disabled, 'submit on DRAFT B after A\'s simulate answers').toBe(true);

    await back();
    fixtures['cmp-2']!.status = 'SIMULATED';
    await openCampaign('cmp-2');
    expect(button('submit').disabled, 'submit on SIMULATED B').toBe(false);
  });
});
