// SB-3 (#842): the /kiosk-queue.html entry takes its branch from the session, not from a 'MAIN'
// fallback (design docs/design/blueprint/org-roles-branches.md §14.4; the same rule as SB-2's
// `web/src/screens/devices/main.branch.test.tsx`).
// Written before the implementation by the test-writer claude/5d45b017, on the FE lane controller
// claude/5edf7bb5's order. Do not edit, delete or weaken; add tests in a file of your own.
//
// `main.tsx` renders into `#app` when it is imported. Each test sets the page URL, stubs the global
// `fetch` (the shared session's `me` read and the queue reads both go through it), imports a fresh
// copy of the module and looks at the URLs read.
//
// The rule pinned here ([required], #842 §3, §8 and §9):
//   - `?branch=` present → that branch, and the queue branch is set to it;
//   - otherwise `me.branch` from the shared session store, and `queueBranch()` is that branch;
//   - never the literal 'MAIN'; no branch-scoped read before `me` has answered when the URL names
//     none, and `queueBranch()` is `undefined` meanwhile.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `me` fixture (`kind`/`role`/`username`) → `meOf(…)` and `employee {id, role}` → `employeeOf(…)`
// (`web/src/test-support/me.ts`, design §3.3), after main's SB-3 tests reached this branch. Nothing else changes.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { employeeOf, meOf } from '../../test-support/me';
import { QUEUE_CARD_PATH, QUEUE_PANEL_PATH } from '../../shared/testing/page-wide';
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
  id: 'E1',
  role: 'FRONT',
  branch: 'B2',
  name: 'Nok',
  branchName: 'สาขาสอง',
  branches: [{ id: 'B2', code: 'XC-02', name: 'สาขาสอง' }],
});

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let urls: string[];
let meAnswer: () => Promise<Response>;

function serve(input: unknown): Promise<Response> | Response {
  const u = String(input);
  urls.push(u);
  const url = new URL(u, 'http://localhost');
  const branch = url.searchParams.get('branch') ?? '';
  switch (url.pathname) {
    case '/api/v3/views/me':
      return meAnswer();
    case QUEUE_CARD_PATH:
      return json({
        version: 7,
        changed: true,
        data: {
          serverNow: new Date().toISOString(),
          branchId: branch,
          branchName: branch,
          count: 0,
          latest: null,
          workingCount: 0,
          working: [],
          employee: employeeOf(ME_B2),
        },
      });
    case QUEUE_PANEL_PATH:
      return json({ version: 7, changed: true, data: [] });
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

function queueReads(): string[] {
  return urls.filter((u) => u.startsWith(QUEUE_PANEL_PATH));
}

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i += 1) await vi.advanceTimersByTimeAsync(0);
}

async function start(url: string): Promise<{ queueBranch: () => string | undefined }> {
  window.history.replaceState(null, '', url);
  await import('./main');
  await flush();
  const store = await import('../../shared/queue/queue-store');
  return { queueBranch: store.queueBranch as () => string | undefined };
}

beforeAll(
  () =>
    warmModules(
      () => import('./main'),
      () => import('../../shared/queue/queue-store'),
    ),
  60_000,
);

beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers({ now: new Date('2026-10-10T05:00:00Z') });
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
  document.body.innerHTML = '<div id="app"></div>';
  urls = [];
  meAnswer = async () => json({ version: 1, changed: true, data: ME_B2 });
  vi.stubGlobal('fetch', vi.fn(async (input: unknown) => serve(input)));
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

describe('SB-3 /kiosk-queue.html entry: the branch', () => {
  it('no ?branch=: every read names me.branch (B2), none MAIN, and queueBranch() is B2', async () => {
    const { queueBranch } = await start('/kiosk-queue.html');
    expect(queueReads().length, 'the panel read the queue').toBeGreaterThan(0);
    expect(new Set(branchesRead())).toEqual(new Set(['B2']));
    expect(queueBranch()).toBe('B2');
  });

  it('?branch=B3: every read names B3, and queueBranch() is B3', async () => {
    const { queueBranch } = await start('/kiosk-queue.html?branch=B3');
    expect(queueReads().length, 'the panel read the queue').toBeGreaterThan(0);
    expect(new Set(branchesRead())).toEqual(new Set(['B3']));
    expect(queueBranch()).toBe('B3');
  });

  it('no ?branch= and me not answered yet: no branch-scoped read, queueBranch() undefined; then B2', async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    meAnswer = async () => {
      await gate;
      return json({ version: 1, changed: true, data: ME_B2 });
    };
    const { queueBranch } = await start('/kiosk-queue.html');
    expect(branchesRead()).toEqual([]);
    expect(queueBranch()).toBeUndefined();
    release();
    await flush();
    expect(queueReads().length, 'the panel read the queue once me answered').toBeGreaterThan(0);
    expect(new Set(branchesRead())).toEqual(new Set(['B2']));
    expect(queueBranch()).toBe('B2');
  });

  it("no ?branch= and me refused (signed out): nothing names 'MAIN'", async () => {
    meAnswer = async () => json({ error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } }, 401);
    const { queueBranch } = await start('/kiosk-queue.html');
    expect(branchesRead()).not.toContain('MAIN');
    expect(queueBranch()).not.toBe('MAIN');
  });
});
