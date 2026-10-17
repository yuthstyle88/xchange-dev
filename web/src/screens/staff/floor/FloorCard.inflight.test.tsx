// #2276: Staff floor in-flight busy guard for Clock in/out, Cut-off, Save target, and Start service (§8, §9).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { StaffFloorView } from '../../../shared/api/generated/staff_floor';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import type { StaffCtx } from '../cards';
import { FloorCard } from './FloorCard';
import { th } from './i18n';

const BRANCH = 'MAIN';
const PUBLISHED_AT = '2026-10-11T02:30:00Z';
const ALL_PERMS = ['AttendanceClock', 'BranchDayOperate', 'CashTargetSet'];

function me(permissions: string[] = ALL_PERMS): StaffMeView {
  return {
    userId: '00000000-0000-4000-8000-0000000000f1',
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  };
}

function floor(over: Partial<StaffFloorView> = {}): StaffFloorView {
  return {
    phase: 'OPEN',
    hours: null,
    myClockIn: null,
    myShift: null,
    cutoff: null,
    target: { amounts: { USD: '1000.00' }, at: '2026-10-11T01:00:00Z' },
    safe: null,
    ...over,
  };
}

const RATES: PublishedRatesData = {
  publication: {
    id: '11111111-2222-4333-8444-555555555555',
    version: 7,
    publishedAt: PUBLISHED_AT,
  },
  sourceDate: '2026-10-10',
  stale: false,
  lines: [
    {
      currency: 'USD',
      group: 'ALL',
      unitSize: '1',
      buyRate: '32.4500',
      sellRate: '32.9000',
      lineId: 'l-usd',
    },
  ],
};

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

interface SentPost {
  path: string;
  key: string | null;
  body: string | undefined;
}

describe('FloorCard in-flight guard (#2276 §8, §9)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  it('row 1: Double click Clock in -> One POST; no error', async () => {
    const posts: SentPost[] = [];
    let releaseFirstClock: (() => void) | undefined;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && path === '/api/v3/views/staff_published_rates') {
          return jsonResponse(200, { version: 1, changed: true, data: RATES });
        }
        if (method === 'POST' && path === '/api/attendance') {
          const headers = new Headers(init?.headers);
          posts.push({
            path,
            key: headers.get('idempotency-key'),
            body: init?.body as string | undefined,
          });
          if (posts.length === 1) {
            await new Promise<void>((r) => {
              releaseFirstClock = r;
            });
            return jsonResponse(200, {});
          }
          // Server returns 409 idempotencyConflict if a duplicate in-flight POST arrives
          return jsonResponse(409, {
            error: { code: 'idempotencyConflict', message: 'Request already in progress' },
          });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx: StaffCtx = {
      me: me(['AttendanceClock']),
      floor: floor(),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <FloorCard ctx={ctx} />, host);
    await flush();

    const clockBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]')!;
    // Double click while first POST is in flight
    clockBtn.click();
    clockBtn.click();
    await flush();

    expect(posts).toHaveLength(1);
    expect(clockBtn.disabled).toBe(true);

    releaseFirstClock!();
    await flush();

    expect(posts).toHaveLength(1);
    expect(host.querySelector('[data-testid="floor-error"]')).toBeNull();
    expect(host.querySelector('[data-testid="floor-notice"]')?.textContent).toBe(th.saved);
    expect(clockBtn.disabled).toBe(false);
  });

  it('row 2: Click again before the refresh answers -> Nothing sent', async () => {
    const posts: SentPost[] = [];
    let releaseRefresh: (() => void) | undefined;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && path === '/api/v3/views/staff_published_rates') {
          return jsonResponse(200, { version: 1, changed: true, data: RATES });
        }
        if (method === 'POST') {
          const headers = new Headers(init?.headers);
          posts.push({
            path,
            key: headers.get('idempotency-key'),
            body: init?.body as string | undefined,
          });
          return jsonResponse(200, {});
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx: StaffCtx = {
      me: me(ALL_PERMS),
      floor: floor(),
      refresh: vi.fn(
        () =>
          new Promise<void>((r) => {
            releaseRefresh = r;
          }),
      ),
    };
    dispose = render(() => <FloorCard ctx={ctx} />, host);
    await flush();

    const clockBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]')!;
    clockBtn.click();
    await flush();

    // First POST has finished, ctx.refresh() is still pending
    expect(posts).toHaveLength(1);
    expect(ctx.refresh).toHaveBeenCalledTimes(1);

    // Click again before refresh answers -> nothing sent
    clockBtn.click();
    await flush();
    expect(posts).toHaveLength(1);
    expect(clockBtn.disabled).toBe(true);

    releaseRefresh!();
    await flush();
    expect(clockBtn.disabled).toBe(false);
  });

  it('row 3: Double click Cut-off (and Target Save / Start Service) -> One POST', async () => {
    const posts: SentPost[] = [];
    let releaseCutoff: (() => void) | undefined;
    let releaseTarget: (() => void) | undefined;
    let releaseStart: (() => void) | undefined;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && path === '/api/v3/views/staff_published_rates') {
          return jsonResponse(200, { version: 1, changed: true, data: RATES });
        }
        if (method === 'POST') {
          const headers = new Headers(init?.headers);
          posts.push({
            path,
            key: headers.get('idempotency-key'),
            body: init?.body as string | undefined,
          });
          if (path === '/api/branch-day/cutoff' && posts.filter((p) => p.path === path).length === 1) {
            await new Promise<void>((r) => {
              releaseCutoff = r;
            });
          }
          if (path === '/api/branch-day/target' && posts.filter((p) => p.path === path).length === 1) {
            await new Promise<void>((r) => {
              releaseTarget = r;
            });
          }
          if (path === '/api/branch-day/start' && posts.filter((p) => p.path === path).length === 1) {
            await new Promise<void>((r) => {
              releaseStart = r;
            });
          }
          return jsonResponse(200, {});
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx: StaffCtx = {
      me: me(ALL_PERMS),
      floor: floor({ phase: 'OPEN' }),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <FloorCard ctx={ctx} />, host);
    await flush();

    // 1. Double click Cut-off
    const cutoffBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-cutoff"]')!;
    cutoffBtn.click();
    cutoffBtn.click();
    await flush();
    expect(posts.filter((p) => p.path === '/api/branch-day/cutoff')).toHaveLength(1);
    expect(cutoffBtn.disabled).toBe(true);
    releaseCutoff!();
    await flush();
    expect(cutoffBtn.disabled).toBe(false);

    // 2. Double submit Save target
    const form = host.querySelector<HTMLFormElement>('[data-testid="floor-target-form"]')!;
    const saveBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-target-save"]')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flush();
    expect(posts.filter((p) => p.path === '/api/branch-day/target')).toHaveLength(1);
    expect(saveBtn.disabled).toBe(true);
    releaseTarget!();
    await flush();
    expect(saveBtn.disabled).toBe(false);

    // 3. Start service confirm while busy
    dispose();
    host.innerHTML = '';
    const readyCtx: StaffCtx = {
      me: me(ALL_PERMS),
      floor: floor({ phase: 'READY' }),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <FloorCard ctx={readyCtx} />, host);
    await flush();

    const startBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
    startBtn.click();
    await flush();
    const confirmBtn = host.querySelectorAll<HTMLButtonElement>('dialog button')[1]!;
    confirmBtn.click();
    confirmBtn.click();
    await flush();
    expect(posts.filter((p) => p.path === '/api/branch-day/start')).toHaveLength(1);
    expect(startBtn.disabled).toBe(true);
    releaseStart!();
    await flush();
    expect(startBtn.disabled).toBe(false);
  });

  it('row 4: Clock in -> Sends {direction:"IN"} (kept)', async () => {
    const posts: SentPost[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'POST') {
          const headers = new Headers(init?.headers);
          posts.push({
            path,
            key: headers.get('idempotency-key'),
            body: init?.body as string | undefined,
          });
          return jsonResponse(200, {});
        }
        return jsonResponse(404, {});
      }),
    );

    const ctx: StaffCtx = {
      me: me(['AttendanceClock']),
      floor: floor({ myClockIn: null }),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <FloorCard ctx={ctx} />, host);
    await flush();

    host.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]')!.click();
    await flush();

    expect(posts).toHaveLength(1);
    expect(posts[0]!.path).toBe('/api/attendance');
    expect(JSON.parse(posts[0]!.body!)).toEqual({ branch: BRANCH, direction: 'IN' });
  });

  it('row 5: A retry after a network error -> Resends the same key and body (kept)', async () => {
    const posts: SentPost[] = [];
    let attempt = 0;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'POST' && path === '/api/attendance') {
          attempt += 1;
          const headers = new Headers(init?.headers);
          posts.push({
            path,
            key: headers.get('idempotency-key'),
            body: init?.body as string | undefined,
          });
          if (attempt === 1) {
            throw new TypeError('Failed to fetch');
          }
          return jsonResponse(200, {});
        }
        return jsonResponse(404, {});
      }),
    );

    const ctx: StaffCtx = {
      me: me(['AttendanceClock']),
      floor: floor(),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <FloorCard ctx={ctx} />, host);
    await flush();

    const clockBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]')!;
    clockBtn.click();
    await flush();
    expect(clockBtn.disabled).toBe(false);

    clockBtn.click();
    await flush();

    expect(posts).toHaveLength(2);
    expect(posts[0]!.key).toBeTruthy();
    expect(posts[1]!.key).toBe(posts[0]!.key);
    expect(posts[1]!.body).toBe(posts[0]!.body);
  });

  it('edge case 1 (§9): busy resets when the refresh fails', async () => {
    const posts: SentPost[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'POST') {
          const headers = new Headers(init?.headers);
          posts.push({
            path,
            key: headers.get('idempotency-key'),
            body: init?.body as string | undefined,
          });
          return jsonResponse(200, {});
        }
        return jsonResponse(404, {});
      }),
    );

    let refreshCall = 0;
    const ctx: StaffCtx = {
      me: me(['AttendanceClock']),
      floor: floor(),
      refresh: vi.fn(async () => {
        refreshCall += 1;
        if (refreshCall === 1) {
          throw new Error('refresh failed');
        }
      }),
    };
    dispose = render(() => <FloorCard ctx={ctx} />, host);
    await flush();

    const clockBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]')!;
    clockBtn.click();
    await flush();

    // busy must be reset in finally even when ctx.refresh() throws
    expect(clockBtn.disabled).toBe(false);

    clockBtn.click();
    await flush();
    expect(posts).toHaveLength(2);
  });

  it('edge case 2 (§9): Start service still reads the rates first and resets busy when rate read fails', async () => {
    const calls: string[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();
        calls.push(`${method} ${path}`);

        if (path === '/api/v3/views/staff_published_rates') {
          return new Response('Internal Server Error', {
            status: 500,
            headers: { 'content-type': 'text/plain' },
          });
        }
        return jsonResponse(200, {});
      }),
    );

    const ctx: StaffCtx = {
      me: me(ALL_PERMS),
      floor: floor({ phase: 'READY' }),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <FloorCard ctx={ctx} />, host);
    await flush();

    const startBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
    startBtn.click();
    await flush();

    const confirmBtn = host.querySelectorAll<HTMLButtonElement>('dialog button')[1]!;
    confirmBtn.click();
    await flush();

    expect(calls.filter((c) => c === 'GET /api/v3/views/staff_published_rates')).toHaveLength(2);
    expect(calls.filter((c) => c === 'POST /api/branch-day/start')).toHaveLength(0);
    expect(host.querySelector('[data-testid="floor-error"]')?.textContent).toBe(
      errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'),
    );
    expect(startBtn.disabled).toBe(false);
  });
});
