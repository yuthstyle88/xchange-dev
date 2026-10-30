// #5638 (DESK-SHIFT T1, docs/design/desk-shift-open.md §2.1, §3 T1 rows K1–K4): the clock button /staff and /desk share.
// Written before the implementation by the ticket author claude/studio-claude-15. Do not edit, delete or weaken;
// add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeStaffFloorView, makeStaffMeView } from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { ClockButton } from './ClockButton';
import { th } from './i18n';

const BRANCH = 'MAIN';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let posts: Array<{ path: string; body: unknown }> = [];
let hold: Array<(r: Response) => void> = [];

function stub(holdWrites = false, answer: () => Response = () => json(200, { id: 'att-1' })) {
  posts = [];
  hold = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method !== 'POST') return json(404, { error: { code: 'notFound', message: 'x' } });
      posts.push({ path: url.split('?')[0]!, body: JSON.parse(String(init?.body ?? 'null')) });
      if (holdWrites) return new Promise<Response>((r) => hold.push(r));
      return answer();
    }),
  );
}

function mount(clockedIn: boolean) {
  const refresh = vi.fn(async () => {});
  const onNotice = vi.fn();
  const onError = vi.fn();
  const [busy, setBusy] = createSignal(false);
  const ctx: StaffCtx = {
    me: makeStaffMeView({ userId: 'u-1', name: 'Ploy', roles: [], branchId: BRANCH, branchName: 'XC-B', permissions: ['AttendanceClock'] }),
    floor: makeStaffFloorView({ phase: 'OPEN', hours: null, myClockIn: clockedIn ? '2026-10-11T01:00:00Z' : null, myShift: null, cutoff: null, target: null, safe: null }),
    refresh,
  };
  dispose = render(
    () => <ClockButton ctx={ctx} busy={busy} setBusy={setBusy} onNotice={onNotice} onError={onError} />,
    host,
  );
  return { refresh, onNotice, onError, busy };
}

const button = () => host.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]');

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('#5638 ClockButton', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
  });

  afterEach(() => {
    hold.forEach((r) => r(json(200, {})));
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
  });

  it('K1 reads clockIn when not clocked in, clockOut when clocked in', () => {
    stub();
    mount(false);
    expect(button()?.textContent?.trim()).toBe(th.clockIn);
    dispose?.();
    mount(true);
    expect(button()?.textContent?.trim()).toBe(th.clockOut);
  });

  it('K2 a click posts /api/attendance IN for my branch, then notices saved and refreshes', async () => {
    stub();
    const m = mount(false);
    button()!.click();
    await settle();
    expect(posts).toHaveLength(1);
    expect(posts[0]!.path).toBe('/api/attendance');
    expect(posts[0]!.body).toMatchObject({ branch: BRANCH, direction: 'IN' });
    expect(m.onNotice).toHaveBeenLastCalledWith(th.saved);
    expect(m.refresh).toHaveBeenCalledTimes(1);
    expect(m.busy()).toBe(false);
  });

  it('K2b clocked in: the click posts OUT', async () => {
    stub();
    mount(true);
    button()!.click();
    await settle();
    expect(posts[0]!.body).toMatchObject({ direction: 'OUT' });
  });

  it('K3 a second click while the first is in flight sends nothing; the button is disabled', async () => {
    stub(true);
    const m = mount(false);
    button()!.click();
    await settle();
    expect(m.busy()).toBe(true);
    expect(button()!.disabled).toBe(true);
    button()!.click();
    await settle();
    expect(posts).toHaveLength(1);
  });

  it('K4 a refusal: notice cleared, worded error, no refresh, busy false', async () => {
    stub(false, () => json(409, { error: { code: 'alreadyClockedIn', message: 'x' } }));
    const m = mount(false);
    button()!.click();
    await settle();
    expect(m.onNotice).toHaveBeenLastCalledWith(null);
    expect(m.onError).toHaveBeenCalled();
    const last = m.onError.mock.calls.at(-1)![0];
    expect(typeof last).toBe('string');
    expect(last).not.toBe('');
    expect(m.refresh).not.toHaveBeenCalled();
    expect(m.busy()).toBe(false);
  });
});
