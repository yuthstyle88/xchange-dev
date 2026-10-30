// #5640 (DESK-SHIFT T3, docs/design/desk-shift-open.md §2.1, §3 T3 rows X1–X5): the staff_me + staff_floor reader
// /staff and /desk share. Written before the implementation by the ticket author claude/studio-claude-15.
// Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { makeStaffFloorView, makeStaffMeView } from '../../shared/testing/fixtures';
import { publishFreshNotices } from '../../shared/ui/notice-bell/notice-feed';
import { createStaffContext, type StaffContext } from './staff-context';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const ME = makeStaffMeView({ userId: 'u-1', name: 'Somchai', roles: [], branchId: 'MAIN', branchName: 'XC-B', permissions: ['ShiftOperate'] });
const FLOOR = makeStaffFloorView({ phase: 'OPEN', hours: null, myClockIn: null, myShift: null, cutoff: null, target: null, safe: null });

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let reads: string[] = [];
let route: (view: string, n: number) => Response | Promise<Response>;

function stub() {
  reads = [];
  const counts: Record<string, number> = {};
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      reads.push(url);
      const view = url.split('?')[0]!.replace('/api/v3/views/', '');
      counts[view] = (counts[view] ?? 0) + 1;
      return route(view, counts[view]!);
    }),
  );
}

const ok = (data: unknown) => json(200, { version: 1, changed: true, data });

function mount(branch?: string) {
  const navigate = vi.fn();
  let sc!: StaffContext;
  dispose = render(() => {
    sc = createStaffContext({
      api: new ApiClient(),
      branch: branch === undefined ? undefined : () => branch,
      signInUrl: '/login?next=%2Fdesk',
      navigate,
    });
    return <span />;
  }, host);
  return { sc: () => sc, navigate };
}

async function settle() {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('#5640 createStaffContext', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    route = (view) => (view === 'staff_me' ? ok(ME) : ok(FLOOR));
    stub();
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
  });

  it('X5 refresh reads staff_me then staff_floor and sets both; ctx carries me, floor and refresh', async () => {
    const { sc } = mount();
    expect(sc().ctx()).toBeUndefined();
    await sc().refresh();
    await settle();
    expect(reads.map((u) => u.split('?')[0])).toEqual(['/api/v3/views/staff_me', '/api/v3/views/staff_floor']);
    expect(sc().me()?.userId).toBe('u-1');
    expect(sc().floor()?.phase).toBe('OPEN');
    expect(sc().error()).toBeUndefined();
    const ctx = sc().ctx()!;
    expect(ctx.me.userId).toBe('u-1');
    expect(ctx.floor?.phase).toBe('OPEN');
    expect(ctx.refresh).toBe(sc().refresh);
  });

  it('X1 a given branch goes on both reads; a newer refresh wins over an older one that answers last', async () => {
    let releaseFirst!: (r: Response) => void;
    route = (view, n) => {
      if (view === 'staff_me' && n === 1) return new Promise<Response>((r) => (releaseFirst = r));
      if (view === 'staff_me') return ok({ ...ME, name: 'newer' });
      return ok(FLOOR);
    };
    const { sc } = mount('B2');
    const first = sc().refresh();
    await settle();
    await sc().refresh();
    await settle();
    releaseFirst(ok({ ...ME, name: 'older' }));
    await first;
    await settle();
    expect(sc().me()?.name).toBe('newer');
    for (const u of reads) expect(new URL(u, 'http://x').searchParams.get('branch')).toBe('B2');
  });

  it('X2 staff_me 401 signIn: navigate to signInUrl; nothing set', async () => {
    route = () => json(401, { error: { code: 'signIn', message: 'sign in' } });
    const { sc, navigate } = mount();
    await sc().refresh();
    await settle();
    expect(navigate).toHaveBeenCalledWith('/login?next=%2Fdesk');
    expect(sc().me()).toBeUndefined();
    expect(reads).toHaveLength(1);
  });

  it('X3 staff_me fails otherwise: a worded error, me unchanged, floor not read', async () => {
    route = () => json(500, { error: { code: 'internal', message: 'boom' } });
    const { sc, navigate } = mount();
    await sc().refresh();
    await settle();
    expect(navigate).not.toHaveBeenCalled();
    expect(sc().error()).toBeTruthy();
    expect(sc().me()).toBeUndefined();
    expect(reads).toHaveLength(1);
  });

  it('X4 staff_floor fails: me set, floor kept as it was, a worded error; the next good refresh clears it', async () => {
    const { sc } = mount();
    await sc().refresh();
    await settle();
    route = (view) => (view === 'staff_me' ? ok(ME) : json(403, { error: { code: 'forbidden', message: 'x' } }));
    await sc().refresh();
    await settle();
    expect(sc().me()?.userId).toBe('u-1');
    expect(sc().floor()?.phase).toBe('OPEN');
    expect(sc().error()).toBeTruthy();
    route = (view) => (view === 'staff_me' ? ok(ME) : ok(FLOOR));
    await sc().refresh();
    await settle();
    expect(sc().error()).toBeUndefined();
  });

  it('X7 a fresh cash_count notice while mounted refreshes; another type does not; after unmount nothing', async () => {
    const { sc } = mount();
    await sc().refresh();
    await settle();
    const n = () => reads.length;
    const before = n();
    publishFreshNotices([{ seq: 1, objectType: 'booking', objectId: null }]);
    await settle();
    expect(n()).toBe(before);
    publishFreshNotices([{ seq: 2, objectType: 'cash_count', objectId: 'c-1' }]);
    await settle();
    expect(reads.slice(before).map((u) => u.split('?')[0])).toEqual([
      '/api/v3/views/staff_me',
      '/api/v3/views/staff_floor',
    ]);
    dispose?.();
    dispose = undefined;
    const after = n();
    publishFreshNotices([{ seq: 3, objectType: 'cash_count', objectId: 'c-2' }]);
    await settle();
    expect(n()).toBe(after);
  });
});
