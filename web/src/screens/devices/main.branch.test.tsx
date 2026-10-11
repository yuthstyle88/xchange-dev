// SB-2 (session branch, web half): the /devices entry takes its branch from the session, not from a
// 'MAIN' fallback (design docs/design/blueprint/org-roles-branches.md §14.0 R2, §14.4 "Devices").
// Written before the implementation by the ticket author (claude/dell-claude-3). Do not edit,
// delete or weaken; add tests in a file of your own.
//
// `web/src/screens/devices/main.tsx` renders into `#app` when it is imported. Each test sets the
// page URL, stubs the global `fetch` (the shared session's `me` read and the screen's view reads
// both go through it), imports a fresh copy of the module and looks at the URLs read.
//
// The rule pinned here ([required]):
//   - `?branch=` present → that branch (a named in-scope branch still wins per request, §14.2, the
//     judgement accepted with the design in PR 831);
//   - otherwise `me.branch` from the shared session store;
//   - never the literal 'MAIN'; and no branch-scoped read before `me` has answered when the URL names
//     none.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `me` answer fixture (`kind`/`role`/`username`) → `meOf(…)` (`web/src/test-support/me.ts`). It is
// untyped, so tsc did not list it, but the shell's queue card and account bar read `permissions`/`roles`
// from it. Nothing else changes.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { meOf } from '../../test-support/me';

const ME_B2 = meOf({
  id: 'u-7',
  role: 'TELLER',
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

/** The branch every read other than `me` named, in order (`<none>` when it named none). */
function branchesRead(): string[] {
  return urls
    .filter((u) => u.startsWith('/api/v3/views/') && !u.startsWith('/api/v3/views/me'))
    .map((u) => new URL(u, 'http://localhost').searchParams.get('branch') ?? '<none>');
}

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i += 1) await vi.advanceTimersByTimeAsync(0);
}

async function start(url: string): Promise<void> {
  window.history.replaceState(null, '', url);
  await import('./main');
  await flush();
}

beforeEach(() => {
  vi.resetModules();
  vi.useFakeTimers({ now: new Date('2026-10-10T05:00:00Z') });
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
  document.body.innerHTML = '<div id="app"></div>';
  urls = [];
  meAnswer = async () => json({ version: 1, changed: true, data: ME_B2 });
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: string) => {
      const u = String(input);
      urls.push(u);
      if (u.startsWith('/api/v3/views/me')) return meAnswer();
      if (u.includes('/health/ready')) return json({ version: '3.0.0', build: 'abc', channel: 'staging', builtAt: '2026-10-10T00:00:00Z' });
      return json({ error: { code: 'notFound', message: 'not found' } }, 404);
    }),
  );
});

afterEach(() => {
  document.body.innerHTML = '';
  vi.useRealTimers();
  vi.unstubAllGlobals();
  delete (document as unknown as Record<string, unknown>).hidden;
  delete (document as unknown as Record<string, unknown>).visibilityState;
  window.history.replaceState(null, '', '/');
});

describe('SB-2 /devices entry: the branch', () => {
  it('no ?branch=: every read names me.branch (B2), none MAIN', async () => {
    await start('/devices');
    const read = branchesRead();
    expect(read.length, 'the screen read its views').toBeGreaterThan(0);
    expect(new Set(read)).toEqual(new Set(['B2']));
  });

  it('?branch=B3: every read names B3', async () => {
    await start('/devices?branch=B3');
    const read = branchesRead();
    expect(read.length, 'the screen read its views').toBeGreaterThan(0);
    expect(new Set(read)).toEqual(new Set(['B3']));
  });

  it('no ?branch= and me not answered yet: no branch-scoped read at all', async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    meAnswer = async () => {
      await gate;
      return json({ version: 1, changed: true, data: ME_B2 });
    };
    await start('/devices');
    expect(branchesRead()).toEqual([]);
    release();
    await flush();
    expect(new Set(branchesRead())).toEqual(new Set(['B2']));
  });

  it("no ?branch= and me refused (signed out): nothing names 'MAIN'", async () => {
    meAnswer = async () => json({ error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } }, 401);
    await start('/devices');
    expect(branchesRead()).not.toContain('MAIN');
  });
});
