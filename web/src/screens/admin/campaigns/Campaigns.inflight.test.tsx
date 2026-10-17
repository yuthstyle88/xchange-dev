// #2426: a second click on a Campaigns write button while the first write is in flight.
// The fake server answers a write whose idempotency key is still PENDING with
// 409 idempotencyConflict, like `api/src/infrastructure/idempotency/pg_store.rs`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
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
  key: string | null;
  body: string;
}

interface HeldWrite {
  path: string;
  release: (reply: Reply) => void;
}

const json = (status: number, body: unknown): Response =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

const view = (data: unknown): Response => json(200, { version: 1, changed: true, data });

function listItem(id: string, name: string, status: string) {
  return {
    id,
    name,
    versionId: `ver-${id}`,
    version: 1,
    status,
    startsAt: '2026-10-01T00:00:00+07:00',
    endsAt: '2026-10-31T23:59:59+07:00',
    budgetThb: '100000.00',
    budgetUsedThb: '0.00',
    budgetLeftThb: '100000.00',
    uses: 0,
    maxUses: 100,
    period: { redemptions: 0, priceCostThb: '0.00', rewardExpenseThb: '0.00' },
  };
}

function detail(id: string, name: string, status: string) {
  return {
    campaign: {
      id,
      name,
      currentVersionId: `ver-${id}`,
      versions: [
        {
          id: `ver-${id}`,
          version: 1,
          status,
          startsAt: '2026-10-01T00:00:00+07:00',
          endsAt: '2026-10-31T23:59:59+07:00',
          budgetThb: '100000.00',
          maxUses: 100,
          stackGroup: 'DEFAULT',
          priority: 10,
          branches: ['MAIN'],
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

describe('#2426 Campaigns: write buttons while a write is in flight', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let held: HeldWrite[];
  let holdPaths: Set<string>;
  let pendingKeys: Set<string>;
  let statuses: Record<string, string>;

  beforeEach(() => {
    setLang('en');
    writes = [];
    held = [];
    holdPaths = new Set();
    pendingKeys = new Set();
    statuses = { 'cmp-1': 'ACTIVE', 'cmp-2': 'DRAFT' };
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET') {
        if (u.pathname === LIST_VIEW) {
          return view({
            campaigns: [
              listItem('cmp-1', 'Golden October', statuses['cmp-1']!),
              listItem('cmp-2', 'Silver November', statuses['cmp-2']!),
            ],
          });
        }
        if (u.pathname === DETAIL_VIEW) {
          const id = u.searchParams.get('id') ?? 'cmp-1';
          const name = id === 'cmp-2' ? 'Silver November' : 'Golden October';
          return view(detail(id, name, statuses[id] ?? 'DRAFT'));
        }
        return json(200, { version: 1, changed: false });
      }

      const key = new Headers(init?.headers).get('idempotency-key');
      writes.push({ method, path: u.pathname, key, body: typeof init?.body === 'string' ? init.body : '' });
      if (key && pendingKeys.has(key)) {
        return json(409, { error: { code: 'idempotencyConflict', message: 'request still in progress' } });
      }
      if (key) pendingKeys.add(key);

      let reply: Reply;
      if (holdPaths.has(u.pathname)) {
        reply = await new Promise<Reply>((resolve) => held.push({ path: u.pathname, release: resolve }));
      } else {
        reply = { status: 200, body: okBody(u.pathname) };
      }
      if (key) pendingKeys.delete(key);
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
    if (path === '/api/campaigns') return { id: 'cmp-1', versionId: 'ver-cmp-1', version: 1 };
    if (path.endsWith('/coupons')) {
      return { coupons: [{ id: 'cpn-1', code: 'GOLD-0001' }] };
    }
    return { versionId: 'ver-cmp-1', version: 1 };
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mountScreen(): Promise<void> {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
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

  function findButton(textKey: string): HTMLButtonElement | undefined {
    return Array.from(root.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === table.en[textKey],
    );
  }

  function button(textKey: string): HTMLButtonElement {
    const btn = findButton(textKey);
    expect(btn, `${textKey} button exists`).toBeDefined();
    return btn!;
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

  function releaseAll(path: string, reply: Reply): void {
    for (const h of held.filter((x) => x.path === path)) h.release(reply);
    held = held.filter((x) => x.path !== path);
  }

  it('row 1: double click Issue coupons sends one POST to the coupon-issue route; the issued list shows and no error', async () => {
    const path = '/api/campaigns/cmp-1/coupons';
    holdPaths.add(path);
    await mountScreen();
    await openCampaign('cmp-1');

    button('issueCoupons').click();
    await settle();
    button('issueCoupons').click();
    await settle();

    releaseAll(path, { status: 200, body: okBody(path) });
    await settle();

    expect(sent('POST', path).length).toBe(1);
    expect(root.querySelector('.xc-issued-coupons')?.textContent).toContain('GOLD-0001');
    expect(errorText()).toBe('');
  });

  it('row 2: click Pause while a pause is in flight sends one pause POST and shows no error', async () => {
    const path = '/api/campaigns/cmp-1/pause';
    holdPaths.add(path);
    await mountScreen();
    await openCampaign('cmp-1');

    button('pause').click();
    await settle();
    button('pause').click();
    await settle();
    expect(errorText(), 'no error while the first pause is in flight').toBe('');

    statuses['cmp-1'] = 'PAUSED';
    releaseAll(path, { status: 200, body: okBody(path) });
    await settle();

    expect(sent('POST', path).length).toBe(1);
    expect(errorText()).toBe('');
  });

  it('row 3: network error on Save (with a second click while in flight), then Save again: same key, byte-equal body', async () => {
    const path = '/api/campaigns/cmp-1/draft';
    holdPaths.add(path);
    await mountScreen();
    await openCampaign('cmp-1');

    button('save').click();
    await settle();
    button('save').click();
    await settle();

    holdPaths.delete(path);
    releaseAll(path, 'network');
    await settle();

    button('save').click();
    await settle();

    const saves = sent('PUT', path);
    expect(saves.length).toBe(2);
    expect(saves[0]!.key).toBeTruthy();
    expect(saves[1]!.key).toBe(saves[0]!.key);
    expect(saves[1]!.body).toBe(saves[0]!.body);
  });

  it('row 4: SIMULATED campaign: Save, Simulate, Submit, Issue coupons are disabled while a write is in flight and enabled again after the first answer', async () => {
    statuses['cmp-1'] = 'SIMULATED';
    const path = '/api/campaigns/cmp-1/simulate';
    holdPaths.add(path);
    await mountScreen();
    await openCampaign('cmp-1');

    // SIMULATED shows Submit enabled; Pause / New version need ACTIVE, checked below.
    const simulatedKeys = ['save', 'simulate', 'submit', 'issueCoupons'];
    for (const k of simulatedKeys) expect(button(k).disabled, `${k} before`).toBe(false);

    button('simulate').click();
    await settle();
    for (const k of simulatedKeys) expect(button(k).disabled, `${k} while simulate is in flight`).toBe(true);

    holdPaths.delete(path);
    releaseAll(path, { status: 200, body: okBody(path) });
    await settle();
    for (const k of simulatedKeys) expect(button(k).disabled, `${k} after`).toBe(false);
  });

  it('row 4: ACTIVE campaign: Pause and New version are disabled while Issue coupons is in flight and enabled again after the first answer', async () => {
    const path = '/api/campaigns/cmp-1/coupons';
    holdPaths.add(path);
    await mountScreen();
    await openCampaign('cmp-1');

    const keys = ['save', 'simulate', 'pause', 'newVersion', 'issueCoupons'];
    for (const k of keys) expect(button(k).disabled, `${k} before`).toBe(false);

    button('issueCoupons').click();
    await settle();
    for (const k of keys) expect(button(k).disabled, `${k} while issue is in flight`).toBe(true);

    holdPaths.delete(path);
    releaseAll(path, { status: 200, body: okBody(path) });
    await settle();
    for (const k of keys) expect(button(k).disabled, `${k} after`).toBe(false);
  });

  it('row 4: PAUSED campaign: Resume is disabled while a resume is in flight (one POST) and enabled again after the first answer', async () => {
    statuses['cmp-1'] = 'PAUSED';
    const path = '/api/campaigns/cmp-1/resume';
    holdPaths.add(path);
    await mountScreen();
    await openCampaign('cmp-1');

    expect(button('resume').disabled, 'resume before').toBe(false);
    button('resume').click();
    await settle();
    expect(button('resume').disabled, 'resume while in flight').toBe(true);
    button('resume').click();
    await settle();

    holdPaths.delete(path);
    releaseAll(path, { status: 200, body: okBody(path) });
    await settle();

    expect(sent('POST', path).length).toBe(1);
    expect(button('resume').disabled, 'resume after').toBe(false);
  });

  it('row 4: Create is disabled while a create is in flight (one POST) and enabled again after a refusal', async () => {
    const path = '/api/campaigns';
    holdPaths.add(path);
    await mountScreen();

    button('create').click();
    await settle();
    expect(root.querySelector('.xc-campaign-create'), 'create view is open').not.toBeNull();

    expect(button('create').disabled, 'create before').toBe(false);
    button('create').click();
    await settle();
    expect(button('create').disabled, 'create while in flight').toBe(true);
    button('create').click();
    await settle();

    holdPaths.delete(path);
    releaseAll(path, { status: 400, body: { error: { code: 'validationFailed', message: 'bad input' } } });
    await settle();

    expect(sent('POST', path).length).toBe(1);
    expect(button('create').disabled, 'create after').toBe(false);
  });

  it('edge: writing is cleared after a refusal: a second click in flight is ignored, a click after the refusal is sent', async () => {
    const path = '/api/campaigns/cmp-1/pause';
    holdPaths.add(path);
    await mountScreen();
    await openCampaign('cmp-1');

    button('pause').click();
    await settle();
    button('pause').click();
    await settle();

    holdPaths.delete(path);
    releaseAll(path, { status: 409, body: { error: { code: 'staleVersion', message: 'stale' } } });
    await settle();
    expect(sent('POST', path).length).toBe(1);
    expect(errorText()).not.toBe('');

    button('pause').click();
    await settle();
    expect(sent('POST', path).length).toBe(2);
  });

  it('edge: opening another campaign while a write is in flight still loads its detail; its writes wait for the first to end', async () => {
    const path = '/api/campaigns/cmp-1/pause';
    holdPaths.add(path);
    await mountScreen();
    await openCampaign('cmp-1');

    button('pause').click();
    await settle();

    button('back').click();
    await settle();
    await openCampaign('cmp-2');

    const nameInput = root.querySelector('input[data-field="name"]') as HTMLInputElement | null;
    expect(nameInput?.value, 'cmp-2 detail loaded').toBe('Silver November');
    expect(root.querySelector('[data-testid="campaign-status"]')?.textContent).toBe(table.en['status.draft']);

    button('save').click();
    await settle();
    expect(sent('PUT', '/api/campaigns/cmp-2/draft').length, 'no second write while the first is in flight').toBe(0);
    expect(button('save').disabled).toBe(true);

    releaseAll(path, { status: 200, body: okBody(path) });
    await settle();
    expect(sent('POST', path).length).toBe(1);
  });
});
