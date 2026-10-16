// #2410: a second click on a Rates write button while the first write is in flight.
// The fake server answers a write whose idempotency key is still PENDING with
// 409 idempotencyConflict, like `api/src/infrastructure/idempotency/pg_store.rs`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
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

interface HeldWrite {
  path: string;
  release: (reply: Reply) => void;
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

function draftItem(id: string, branchId: string, status: string) {
  return {
    id,
    branchId,
    status,
    version: 1,
    createdBy: 'rates-9',
    createdAt: '2026-10-16T02:00:00+00:00',
    updatedAt: '2026-10-16T02:00:00+00:00',
    evaluation: null,
  };
}

const usdLine = { currency: 'USD', buyMode: 'AUTO', sellMode: 'AUTO', manualBuy: null, manualSell: null, reason: null };

describe('#2410 Rates: write buttons while a write is in flight', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let setBranch: Setter<string>;
  let writes: SentWrite[];
  let held: HeldWrite[];
  let holdPaths: Set<string>;
  let replies: Map<string, Reply[]>;
  let pendingKeys: Set<string>;
  let drafts: Record<string, ReturnType<typeof draftItem>[]>;

  beforeEach(() => {
    setLang('en');
    // The toast text is module state that outlives a test; start each test with none.
    toast('');
    writes = [];
    held = [];
    holdPaths = new Set();
    replies = new Map();
    pendingKeys = new Set();
    drafts = {
      MAIN: [draftItem('d1', 'MAIN', 'APPROVED')],
      B9: [draftItem('d9', 'B9', 'APPROVED')],
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
              lines: [{ currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '33.25000000', sellRate: '34.10000000', lineId: 'line-USD' }],
            });
          case '/api/v3/views/rate_drafts':
            return view({ drafts: structuredClone(drafts[branch] ?? []) });
          case '/api/v3/views/rate_draft': {
            const id = u.searchParams.get('id') ?? '';
            const item = Object.values(drafts).flat().find((d) => d.id === id) ?? draftItem(id, branch, 'DRAFT');
            return view({ draft: structuredClone(item), lines: [usdLine] });
          }
          case '/api/v3/views/rates_reference':
            return view({ rows: [], fetchedAt: '2026-10-16T02:00:00Z', error: null });
          default:
            return json(200, { version: 1, changed: false });
        }
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
        reply = replies.get(u.pathname)?.shift() ?? { status: 200, body: okBody(method, u.pathname) };
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

  function okBody(method: string, path: string): unknown {
    if (method === 'POST' && path === '/api/rate-drafts') {
      drafts.MAIN = [draftItem('d-new-1', 'MAIN', 'DRAFT')];
      return { id: 'd-new-1', version: 1, status: 'DRAFT' };
    }
    if (path.endsWith('/simulate')) return { id: 'sim-1', isInBound: true, outOfBound: [] };
    if (path.endsWith('/submit')) return { status: 'PENDING_APPROVAL' };
    if (path.endsWith('/publish')) return { id: 'pub-8', version: 8 };
    if (path === '/api/rate-policies') return { id: 'pol-2', version: 2 };
    return {};
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mountScreen(): Promise<void> {
    const [branch, set] = createSignal('MAIN');
    setBranch = set;
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

  function sent(method: string, path: string): SentWrite[] {
    return writes.filter((w) => w.method === method && w.path === path);
  }

  function toastText(): string {
    return Array.from(document.querySelectorAll('.toast'))
      .map((el) => el.textContent ?? '')
      .join('')
      .trim();
  }

  function releaseAll(path: string, reply: Reply): void {
    for (const h of held.filter((x) => x.path === path)) h.release(reply);
    held = held.filter((x) => x.path !== path);
  }

  it('row 1: double click Publish sends one POST /api/rate-drafts/{id}/publish', async () => {
    holdPaths.add('/api/rate-drafts/d1/publish');
    await mountScreen();

    button('.btn-publish').click();
    await settle();
    button('.btn-publish').click();
    await settle();

    releaseAll('/api/rate-drafts/d1/publish', { status: 200, body: { id: 'pub-8', version: 8 } });
    await settle();

    expect(sent('POST', '/api/rate-drafts/d1/publish').length).toBe(1);
    expect(toastText()).toBe('');
  });

  it('row 2: click New draft while it is in flight: no toast and exactly one POST /api/rate-drafts', async () => {
    drafts.MAIN = [];
    holdPaths.add('/api/rate-drafts');
    await mountScreen();

    button('.btn-new-draft').click();
    await settle();
    expect(sent('POST', '/api/rate-drafts').length).toBe(1);

    // The reference fetch has answered; the create is still in flight.
    button('.btn-new-draft').click();
    await settle();

    holdPaths.delete('/api/rate-drafts');
    releaseAll('/api/rate-drafts', { status: 200, body: okBody('POST', '/api/rate-drafts') });
    await settle();

    expect(sent('POST', '/api/rate-drafts').length).toBe(1);
    expect(sent('POST', '/api/rate-references/fetch').length).toBe(1);
    expect(toastText()).toBe('');
  });

  it('row 3: network error on Save policy (with a second click while in flight), then retry: same key', async () => {
    holdPaths.add('/api/rate-policies');
    await mountScreen();

    button('[data-testid="save-policy"]').click();
    await settle();
    button('[data-testid="save-policy"]').click();
    await settle();

    holdPaths.delete('/api/rate-policies');
    releaseAll('/api/rate-policies', 'network');
    await settle();

    button('[data-testid="save-policy"]').click();
    await settle();

    const saves = sent('POST', '/api/rate-policies');
    expect(saves.length).toBe(2);
    expect(saves[0]!.key).toBeTruthy();
    expect(saves[1]!.key).toBe(saves[0]!.key);
    expect(saves[1]!.body).toBe(saves[0]!.body);
  });

  it('all six write buttons are disabled while a write is in flight, and enabled again after it ends', async () => {
    holdPaths.add('/api/rate-drafts/d1/publish');
    await mountScreen();

    const selectors = [
      '.btn-new-draft',
      '.btn-simulate',
      '.btn-submit',
      '.btn-publish',
      '.btn-adjust-all',
      '[data-testid="save-policy"]',
    ];
    for (const s of selectors) expect(button(s).disabled, `${s} before`).toBe(false);

    button('.btn-publish').click();
    await settle();
    for (const s of selectors) expect(button(s).disabled, `${s} while publish is in flight`).toBe(true);

    holdPaths.delete('/api/rate-drafts/d1/publish');
    releaseAll('/api/rate-drafts/d1/publish', { status: 200, body: { id: 'pub-8', version: 8 } });
    await settle();
    for (const s of selectors) expect(button(s).disabled, `${s} after`).toBe(false);
  });

  it('edge: busy is cleared after a refusal: a second click in flight is ignored, a click after the refusal is sent', async () => {
    holdPaths.add('/api/rate-drafts/d1/submit');
    await mountScreen();

    button('.btn-submit').click();
    await settle();
    button('.btn-submit').click();
    await settle();

    holdPaths.delete('/api/rate-drafts/d1/submit');
    releaseAll('/api/rate-drafts/d1/submit', {
      status: 409,
      body: { error: { code: 'staleVersion', message: 'stale' } },
    });
    await settle();
    expect(sent('POST', '/api/rate-drafts/d1/submit').length).toBe(1);
    expect(toastText()).not.toBe('');

    button('.btn-submit').click();
    await settle();
    expect(sent('POST', '/api/rate-drafts/d1/submit').length).toBe(2);
  });

  it('edge: branch switch while busy follows the #2308 generation rules: no toast from the old branch, no extra send', async () => {
    holdPaths.add('/api/rate-drafts/d1/publish');
    await mountScreen();

    button('.btn-publish').click();
    await settle();
    button('.btn-publish').click();
    await settle();

    setBranch('B9');
    await settle();

    holdPaths.delete('/api/rate-drafts/d1/publish');
    releaseAll('/api/rate-drafts/d1/publish', {
      status: 409,
      body: { error: { code: 'staleVersion', message: 'stale' } },
    });
    await settle();

    expect(sent('POST', '/api/rate-drafts/d1/publish').length).toBe(1);
    expect(toastText()).toBe('');

    // After the old branch's write ends, the new branch's draft can be published.
    button('.btn-publish').click();
    await settle();
    expect(sent('POST', '/api/rate-drafts/d9/publish').length).toBe(1);
  });
});
