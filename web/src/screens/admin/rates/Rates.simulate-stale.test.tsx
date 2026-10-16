// #2473: Admin Rates — "In bound" simulate result and per-line out-of-bound pills
// must be cleared after Adjust all, Save policy, or Publish succeed (and when draft id changes),
// while keeping the result on Submit with no change or when Adjust all / Save policy fails.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Dialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { Rates } from './Rates';

type Reply = { status: number; body: unknown } | 'network';

interface SentWrite {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

const json = (status: number, body: unknown): Response =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

const view = (data: unknown): Response => json(200, { version: 1, changed: true, data });

const samplePolicy = {
  id: 'pol-1',
  version: 1,
  branch: 'MAIN',
  buyAdjustPct: '-0.5000',
  sellAdjustPct: '0.4000',
  guards: {},
  maxReferenceAgeHours: 24,
  onStale: 'STOP',
  staleLimitThb: null,
  reason: 'default policy',
  valueTiersOn: false,
  valueTiers: [],
  sharpMoveAlertPct: '2.50',
};

function draftItem(id: string, branchId: string, status: string, version = 1) {
  return {
    id,
    branchId,
    status,
    version,
    createdBy: 'rates-9',
    createdAt: '2026-10-16T02:00:00+00:00',
    updatedAt: '2026-10-16T02:00:00+00:00',
    evaluation: null,
  };
}

const usdLine = {
  currency: 'USD',
  buyMode: 'AUTO',
  sellMode: 'AUTO',
  manualBuy: null,
  manualSell: null,
  reason: null,
};

describe('#2473 Rates: clear stale simulateResult after Adjust all, Save policy, or Publish', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let replies: Map<string, Reply[]>;
  let drafts: Record<string, ReturnType<typeof draftItem>[]>;

  beforeEach(() => {
    setLang('en');
    toast('');
    writes = [];
    replies = new Map();
    drafts = {
      MAIN: [draftItem('d1', 'MAIN', 'APPROVED', 1)],
    };
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET') {
        const branch = u.searchParams.get('branch') ?? 'MAIN';
        switch (u.pathname) {
          case '/api/v3/views/me':
            return view({
              kind: 'owner',
              id: 'owner-1',
              role: 'OWNER',
              branch: 'MAIN',
              name: 'Owner',
              username: 'owner',
              permissions: ['OverviewRead', 'RatesDraft', 'RatesHistoryRead', 'RatePolicySave', 'SystemSettings'],
            });
          case '/api/v3/views/rate_policy':
            return view({ policy: { ...structuredClone(samplePolicy), branch } });
          case '/api/v3/views/published_rates':
            return view({
              publication: { id: `pub-${branch}`, version: 7, publishedAt: '2026-10-16T02:00:00Z' },
              sourceDate: '2026-10-16',
              stale: false,
              lines: [
                {
                  currency: 'USD',
                  group: 'ALL',
                  unitSize: '1',
                  buyRate: '33.25000000',
                  sellRate: '34.10000000',
                  lineId: 'line-USD',
                },
              ],
            });
          case '/api/v3/views/rate_drafts':
            return view({ drafts: structuredClone(drafts[branch] ?? []) });
          case '/api/v3/views/rate_draft': {
            const id = u.searchParams.get('id') ?? '';
            const item =
              Object.values(drafts)
                .flat()
                .find((d) => d.id === id) ?? draftItem(id, branch, 'DRAFT');
            return view({ draft: structuredClone(item), lines: [usdLine] });
          }
          case '/api/v3/views/rates_reference':
            return view({ rows: [], fetchedAt: '2026-10-16T02:00:00Z', error: null });
          default:
            return json(200, { version: 1, changed: false });
        }
      }

      const key = new Headers(init?.headers).get('idempotency-key');
      writes.push({
        method,
        path: u.pathname,
        key,
        body: typeof init?.body === 'string' ? init.body : '',
      });

      const queueKey = `${method} ${u.pathname}`;
      const queued = replies.get(queueKey)?.shift() ?? replies.get(u.pathname)?.shift();
      const reply: Reply = queued ?? { status: 200, body: okBody(method, u.pathname) };
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

  function okBody(method: string, path: string): unknown {
    if (method === 'POST' && path === '/api/rate-drafts') {
      drafts.MAIN = [draftItem('d-new-1', 'MAIN', 'DRAFT')];
      return { id: 'd-new-1', version: 1, status: 'DRAFT' };
    }
    if (path.endsWith('/simulate')) return { id: 'd1', isInBound: true, outOfBound: [] };
    if (path.endsWith('/submit')) return { status: 'PENDING_APPROVAL' };
    if (path.endsWith('/publish')) {
      drafts.MAIN = [];
      return { id: 'pub-8', version: 8 };
    }
    if (path === '/api/rate-policies') return { id: 'pol-2', version: 2 };
    if (method === 'PUT' && path.startsWith('/api/rate-drafts/')) {
      return { id: 'd1', version: 2, status: 'DRAFT' };
    }
    return {};
  }

  function queueReply(key: string, ...items: Reply[]): void {
    const existing = replies.get(key) ?? [];
    existing.push(...items);
    replies.set(key, existing);
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mountScreen(): Promise<void> {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-16', to: '2026-10-16' });
    const [shown] = createSignal(true);
    dispose = render(
      () => (
        <>
          <Rates branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );
    await settle();
  }

  function button(selector: string): HTMLButtonElement {
    const btn = root.querySelector<HTMLButtonElement>(selector);
    expect(btn, selector).not.toBeNull();
    return btn!;
  }

  it('row 1: Simulate in bound, then Adjust all -> No "In bound" banner and no stale pills until a new Simulate', async () => {
    await mountScreen();

    // 1. Simulate out-of-bound -> USD line gets out-of-bound pill
    queueReply('/api/rate-drafts/d1/simulate', {
      status: 200,
      body: { id: 'd1', isInBound: false, outOfBound: [{ currency: 'USD', side: 'SELL' }] },
    });
    button('.btn-simulate').click();
    await settle();
    expect(root.querySelector('[data-draft-line="USD"] .out-of-bound')).not.toBeNull();

    // Adjust all re-saves the draft -> stale out-of-bound pill is cleared
    button('.btn-adjust-all').click();
    await settle();
    expect(root.querySelector('[data-draft-line="USD"] .out-of-bound')).toBeNull();

    // 2. Simulate in-bound -> "In bound" banner shows
    queueReply('/api/rate-drafts/d1/simulate', {
      status: 200,
      body: { id: 'd1', isInBound: true, outOfBound: [] },
    });
    button('.btn-simulate').click();
    await settle();
    expect(root.querySelector('.in-bound-status')).not.toBeNull();

    // Adjust all re-saves the draft -> "In bound" banner is cleared until a new Simulate
    button('.btn-adjust-all').click();
    await settle();
    expect(root.querySelector('.in-bound-status')).toBeNull();
    expect(root.querySelector('[data-draft-line="USD"] .out-of-bound')).toBeNull();

    // 3. New Simulate shows the fresh result again
    queueReply('/api/rate-drafts/d1/simulate', {
      status: 200,
      body: { id: 'd1', isInBound: true, outOfBound: [] },
    });
    button('.btn-simulate').click();
    await settle();
    expect(root.querySelector('.in-bound-status')).not.toBeNull();
  });

  it('row 2: Simulate, then Save policy -> Simulate result cleared', async () => {
    await mountScreen();

    queueReply('/api/rate-drafts/d1/simulate', {
      status: 200,
      body: { id: 'd1', isInBound: true, outOfBound: [{ currency: 'USD', side: 'SELL' }] },
    });
    button('.btn-simulate').click();
    await settle();
    expect(root.querySelector('.in-bound-status')).not.toBeNull();
    expect(root.querySelector('[data-draft-line="USD"] .out-of-bound')).not.toBeNull();

    button('[data-action="save-policy"]').click();
    await settle();

    expect(root.querySelector('.in-bound-status')).toBeNull();
    expect(root.querySelector('[data-draft-line="USD"] .out-of-bound')).toBeNull();
  });

  it('row 3: Simulate, then Publish -> No "In bound" banner', async () => {
    await mountScreen();

    queueReply('/api/rate-drafts/d1/simulate', {
      status: 200,
      body: { id: 'd1', isInBound: true, outOfBound: [] },
    });
    button('.btn-simulate').click();
    await settle();
    expect(root.querySelector('.in-bound-status')).not.toBeNull();

    button('.btn-publish').click();
    await settle();

    expect(root.querySelector('.in-bound-status')).toBeNull();
  });

  it('edge case 1 (§9): failed Adjust all keeps the simulate result; subsequent successful Adjust all clears it', async () => {
    await mountScreen();

    queueReply('/api/rate-drafts/d1/simulate', {
      status: 200,
      body: { id: 'd1', isInBound: true, outOfBound: [] },
    });
    button('.btn-simulate').click();
    await settle();
    expect(root.querySelector('.in-bound-status')).not.toBeNull();

    // First Adjust all fails on PUT /api/rate-drafts/d1 -> simulateResult is kept
    queueReply('PUT /api/rate-drafts/d1', {
      status: 500,
      body: { error: { code: 'internalError', message: 'save draft failed' } },
    });
    button('.btn-adjust-all').click();
    await settle();
    expect(root.querySelector('.in-bound-status')).not.toBeNull();

    // Second Adjust all succeeds -> simulateResult is cleared
    button('.btn-adjust-all').click();
    await settle();
    expect(root.querySelector('.in-bound-status')).toBeNull();
  });

  it('edge case 2 (§9) & row 4: failed Save policy and Submit with no change keep the simulate result; subsequent successful Save policy clears it', async () => {
    await mountScreen();

    queueReply('/api/rate-drafts/d1/simulate', {
      status: 200,
      body: { id: 'd1', isInBound: true, outOfBound: [] },
    });
    button('.btn-simulate').click();
    await settle();
    expect(root.querySelector('.in-bound-status')).not.toBeNull();

    // Failed Save policy keeps the result
    queueReply('POST /api/rate-policies', {
      status: 500,
      body: { error: { code: 'internalError', message: 'policy save failed' } },
    });
    button('[data-action="save-policy"]').click();
    await settle();
    expect(root.querySelector('.in-bound-status')).not.toBeNull();

    // Submit with no change keeps the result
    button('.btn-submit').click();
    await settle();
    expect(root.querySelector('.in-bound-status')).not.toBeNull();

    // Subsequent successful Save policy clears the result
    button('[data-action="save-policy"]').click();
    await settle();
    expect(root.querySelector('.in-bound-status')).toBeNull();
  });
});
