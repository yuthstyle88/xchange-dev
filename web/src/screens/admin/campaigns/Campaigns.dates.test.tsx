// #2505: Campaign detail form must convert UTC startsAt/endsAt from the server into
// Bangkok (+07:00) wall-clock values and pre-fill coupon expiry with the Bangkok date of endsAt.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

declare const process: { env: Record<string, string | undefined> };

const ORIG_TZ = vi.hoisted(() => {
  const o = process.env.TZ;
  process.env.TZ = 'UTC';
  return o;
});

import { setLang } from '../../../shared/i18n';
import { Campaigns } from './Campaigns';
import * as table from './i18n';

const LIST_VIEW = '/api/v3/views/campaigns';
const DETAIL_VIEW = '/api/v3/views/campaign';

interface SentWrite {
  method: string;
  path: string;
  body: string;
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

function utcFixtures(): Record<string, Fixture> {
  return {
    'cmp-1': {
      name: 'October Campaign',
      status: 'DRAFT',
      startsAt: '2026-09-30T17:00:00+00:00',
      endsAt: '2026-10-31T16:59:59+00:00',
      budgetThb: '100000.00',
      stackGroup: 'DEFAULT',
      priority: 10,
      branches: ['MAIN'],
    },
    'cmp-2': {
      name: 'Early Morning End Campaign',
      status: 'DRAFT',
      startsAt: '2026-09-30T17:00:00+00:00',
      endsAt: '2026-10-31T20:00:00+00:00',
      budgetThb: '50000.00',
      stackGroup: 'DEFAULT',
      priority: 5,
      branches: ['MAIN'],
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

describe('#2505 Campaigns: Bangkok wall-clock dates in edit form and coupon expiry default', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let fixtures: Record<string, Fixture>;

  afterAll(() => {
    if (ORIG_TZ === undefined) {
      delete process.env.TZ;
    } else {
      process.env.TZ = ORIG_TZ;
    }
  });

  beforeEach(() => {
    setLang('en');
    writes = [];
    fixtures = utcFixtures();
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
          return view(detail(id, fixtures[id]!));
        }
        return json(200, { version: 1, changed: false });
      }

      writes.push({ method, path: u.pathname, body: typeof init?.body === 'string' ? init.body : '' });
      return json(200, { versionId: 'ver-x', version: 1 });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mountScreen(): Promise<void> {
    const [branch] = createSignal('MAIN');
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

  function button(textKey: string): HTMLButtonElement {
    const btn = Array.from(root.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === table.en[textKey],
    );
    expect(btn, `${textKey} button exists`).toBeDefined();
    return btn!;
  }

  function field(name: string): string {
    const el = root.querySelector(`.xc-campaign-detail [name="${name}"]`) as
      | HTMLInputElement
      | HTMLSelectElement
      | null;
    expect(el, `field ${name} exists`).not.toBeNull();
    return el!.value;
  }

  it('row 1: Detail startsAt 2026-09-30T17:00:00+00:00 -> Form shows 2026-10-01 (Bangkok)', async () => {
    await mountScreen();
    await openCampaign('cmp-1');

    expect(field('startsAt')).toBe('2026-10-01T00:00:00+07:00');
    expect(field('startsAt')).not.toContain('2026-09-30');
  });

  it('row 2: Detail endsAt 2026-10-31T16:59:59+00:00 -> Form shows 2026-10-31 23:59:59 Bangkok; coupon expiry default 2026-10-31', async () => {
    await mountScreen();
    await openCampaign('cmp-1');

    expect(field('endsAt')).toBe('2026-10-31T23:59:59+07:00');
    expect(field('expiresAt')).toBe('2026-10-31');
  });

  it('row 3: Detail endsAt 2026-10-31T20:00:00+00:00 (03:00 Bangkok on 1 Nov) -> Coupon expiry default 2026-11-01', async () => {
    await mountScreen();
    await openCampaign('cmp-2');

    expect(field('endsAt')).toBe('2026-11-01T03:00:00+07:00');
    expect(field('expiresAt')).toBe('2026-11-01');
  });

  it('row 4: Save without edits -> Sends the same instants as before (no shift)', async () => {
    await mountScreen();
    await openCampaign('cmp-2');

    expect(field('startsAt')).toBe('2026-10-01T00:00:00+07:00');
    expect(field('endsAt')).toBe('2026-11-01T03:00:00+07:00');

    button('save').click();
    await settle();

    const saves = writes.filter((w) => w.method === 'PUT' && w.path === '/api/campaigns/cmp-2/draft');
    expect(saves).toHaveLength(1);
    const sent = JSON.parse(saves[0]!.body) as { startsAt: string; endsAt: string };
    expect(sent.startsAt).toBe('2026-10-01T00:00:00+07:00');
    expect(sent.endsAt).toBe('2026-11-01T03:00:00+07:00');
    expect(Date.parse(sent.startsAt)).toBe(Date.parse('2026-09-30T17:00:00+00:00'));
    expect(Date.parse(sent.endsAt)).toBe(Date.parse('2026-10-31T20:00:00+00:00'));
  });
});
