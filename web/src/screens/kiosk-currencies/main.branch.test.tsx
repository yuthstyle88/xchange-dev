// SB-3 (#842): the /kiosk-currencies.html entry takes its branch from the session, not from a 'MAIN'
// fallback (design docs/design/blueprint/org-roles-branches.md §14.4; the same rule as SB-2's
// `web/src/screens/devices/main.branch.test.tsx`), and its switch never writes to a guessed branch
// (the FE lane controller's ruling on #842, issuecomment-6105121433, item 1).
// Written before the implementation by the test-writer claude/5d45b017, on the FE lane controller
// claude/5edf7bb5's order. Do not edit, delete or weaken; add tests in a file of your own.
//
// `main.tsx` renders into `#app` when it is imported. Each test sets the page URL, stubs the global
// `fetch` (the shared session's `me` read, the screen's `kiosk_currencies` read and its `configure`
// POST all go through it), imports a fresh copy of the module and looks at what was sent.
//
// The rule pinned here ([required], #842 §3, §8 and §9 plus the ruling's row):
//   - `?branch=` present → that branch, at once;
//   - otherwise `me.branch` from the shared session store, once `me` answers;
//   - never the literal 'MAIN', in a read or in `configure`'s `branchId`;
//   - before `me` answers (URL naming no branch): no branch-scoped read, and ticking any switch on
//     the page sends nothing.
// Assumed, as today: the switch is the card's `input[type="checkbox"]`; `configure` is
// `POST /api/kiosk` with `{ action: 'configure', payload: { branchId, code, enabled } }`.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `me` fixture (`kind`/`role`/`username`) → `meOf(…)`
// (`web/src/test-support/me.ts`, design §3.3), after main's SB-3 tests reached this branch. Nothing else changes.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { meOf } from '../../test-support/me';
import { warmModules } from '../../shared/testing/warm-modules';

const disposers: Array<() => void> = [];

vi.mock('solid-js/web', async (importOriginal) => {
  const mod = await importOriginal<typeof import('solid-js/web')>();
  return {
    ...mod,
    render: (code: () => any, element: Node, init?: any, options?: any) => {
      const dispose = mod.render(code, element, init, options);
      disposers.push(dispose);
      return dispose;
    },
  };
});

const ME_B2 = meOf({
  id: 'u-7',
  role: 'MANAGER',
  branch: 'B2',
  name: 'Nok',
  branchName: 'สาขาสอง',
  branches: [{ id: 'B2', code: 'XC-02', name: 'สาขาสอง' }],
});

const USD = {
  code: 'USD',
  name: 'ดอลลาร์สหรัฐ',
  nameEn: 'US Dollar',
  countries: [{ code: 'US', en: 'United States', th: 'สหรัฐอเมริกา' }],
  buy: '32.10000000',
  sell: '33.00000000',
  unitSize: '1',
  updated: '2026-10-09T05:00:01.123Z',
  partnerName: '',
  partnerCost: null,
  enabled: false,
  kbank: [],
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let urls: string[];
let posts: Array<{ path: string; body: any }>;
let meAnswer: () => Promise<Response>;

async function serve(input: unknown, init?: RequestInit): Promise<Response> {
  const u = String(input);
  const url = new URL(u, 'http://localhost');
  if ((init?.method ?? 'GET').toUpperCase() === 'POST') {
    posts.push({ path: url.pathname, body: init?.body ? JSON.parse(String(init.body)) : undefined });
    return json({ ok: true });
  }
  urls.push(u);
  switch (url.pathname) {
    case '/api/v3/views/me':
      return meAnswer();
    case '/api/v3/views/kiosk_currencies':
      return json({
        version: 7,
        changed: true,
        data: { currencies: [USD], kbank: null, freshness: { stale: false, sourceDate: null, publishedAt: null } },
      });
    case '/health/ready':
      return json({ status: 'ok', database: true, version: '3.0.0', build: 'abc', builtAt: '2026-10-10T00:00:00Z', channel: 'staging', ui: 'built' });
    default:
      return json({ error: { code: 'notFound', message: 'not found' } }, 404);
  }
}

/** The branch every view read other than `me` named, in order (`<none>` when it named none). */
function branchesRead(): string[] {
  return urls
    .filter((u) => u.startsWith('/api/v3/views/') && !u.startsWith('/api/v3/views/me'))
    .map((u) => new URL(u, 'http://localhost').searchParams.get('branch') ?? '<none>');
}

function currencyReads(): string[] {
  return urls.filter((u) => u.startsWith('/api/v3/views/kiosk_currencies'));
}

/** Ticks every switch on the page. */
function tickAll(): number {
  const boxes = [...document.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')];
  for (const b of boxes) b.click();
  return boxes.length;
}

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i += 1) await vi.advanceTimersByTimeAsync(0);
}

async function start(url: string): Promise<void> {
  window.history.replaceState(null, '', url);
  await import('./main');
  await flush();
}

beforeAll(() => warmModules(() => import('./main')), 60_000);

beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers({ now: new Date('2026-10-10T05:00:00Z') });
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
  document.body.innerHTML = '<div id="app"></div>';
  urls = [];
  posts = [];
  meAnswer = async () => json({ version: 1, changed: true, data: ME_B2 });
  vi.stubGlobal('fetch', vi.fn(serve));
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  delete (document as unknown as Record<string, unknown>).hidden;
  delete (document as unknown as Record<string, unknown>).visibilityState;
  window.history.replaceState(null, '', '/');
});

describe('SB-3 /kiosk-currencies.html entry: the branch', () => {
  it('no ?branch=: every read names me.branch (B2), and a switch configures B2', async () => {
    await start('/kiosk-currencies.html');
    expect(currencyReads().length, 'the screen read its view').toBeGreaterThan(0);
    expect(new Set(branchesRead())).toEqual(new Set(['B2']));
    expect(tickAll(), 'the screen shows a switch').toBeGreaterThan(0);
    await flush();
    expect(posts.map((p) => p.body?.payload?.branchId)).toEqual(['B2']);
  });

  it('?branch=B3: every read names B3, and a switch configures B3', async () => {
    await start('/kiosk-currencies.html?branch=B3');
    expect(currencyReads().length, 'the screen read its view').toBeGreaterThan(0);
    expect(new Set(branchesRead())).toEqual(new Set(['B3']));
    expect(tickAll(), 'the screen shows a switch').toBeGreaterThan(0);
    await flush();
    expect(posts.map((p) => p.body?.payload?.branchId)).toEqual(['B3']);
  });

  it('no ?branch= and me not answered yet: no branch-scoped read, a toggle sends nothing; then B2', async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    meAnswer = async () => {
      await gate;
      return json({ version: 1, changed: true, data: ME_B2 });
    };
    await start('/kiosk-currencies.html');
    expect(branchesRead()).toEqual([]);
    tickAll();
    await flush();
    expect(posts, 'a toggle before me loads sends nothing').toEqual([]);
    release();
    await flush();
    expect(currencyReads().length, 'the screen read its view once me answered').toBeGreaterThan(0);
    expect(new Set(branchesRead())).toEqual(new Set(['B2']));
  });

  it("no ?branch= and me refused (signed out): nothing names 'MAIN', in a read or a configure", async () => {
    meAnswer = async () => json({ error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } }, 401);
    await start('/kiosk-currencies.html');
    tickAll();
    await flush();
    expect(branchesRead()).not.toContain('MAIN');
    expect(posts.map((p) => p.body?.payload?.branchId)).not.toContain('MAIN');
  });
});
