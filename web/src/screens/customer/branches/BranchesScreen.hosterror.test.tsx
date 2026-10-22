import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { setAudience, setLang } from '../../../shared/i18n';
import { BranchesScreen } from './BranchesScreen';
import * as table from './i18n';

type Texts = Record<string, string>;
const TH = table.th as unknown as Texts;

function text(key: string, t: Texts = TH): string {
  expect(t[key], `i18n key ${key}`).toBeTypeOf('string');
  return t[key]!;
}

type Route = 'host' | 'branches';
type Reply = { status: number; body?: unknown; rawBody?: string; etag?: string } | 'network';

interface Call {
  route: Route | 'other';
  path: string;
}

const SAMPLE_BRANCHES: PublicBranchesData = {
  branches: [
    {
      id: 'XC-B01',
      code: 'XC-B01',
      name: 'สาขาสยามพารากอน',
      address: 'ชั้น G สยามพารากอน',
      openingHours: {
        mon: [{ open: '10:00', close: '20:00' }],
      },
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
    {
      id: 'XC-B02',
      code: 'XC-B02',
      name: 'สาขาสนามบินดอนเมือง',
      address: null,
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: false,
      queueOpen: false,
    },
  ],
};

function routeOf(path: string): Route | 'other' {
  if (path === '/api/public/host') return 'host';
  if (path === '/api/public/orgs/XC/branches') return 'branches';
  return 'other';
}

function server(over: Partial<Record<Route, Reply[]>> = {}) {
  const queue: Record<Route, Reply[]> = {
    host: [{ status: 200, body: { org: 'XC' } }],
    branches: [{ status: 200, body: SAMPLE_BRANCHES, etag: '"b1-v1"' }],
    ...over,
  };
  const calls: Call[] = [];
  const fetch = vi.fn(async (url: string): Promise<Response> => {
    const path = new URL(url, 'http://test.local').pathname;
    const route = routeOf(path);
    calls.push({ route, path });
    if (route === 'other') return new Response('{}', { status: 404 });
    const replies = queue[route];
    const reply = replies.length > 1 ? replies.shift()! : replies[0]!;
    if (reply === 'network') throw new TypeError('network down');
    const headers: Record<string, string> = {};
    if (reply.etag) headers['ETag'] = reply.etag;
    if (reply.status === 304) return new Response(null, { status: 304, headers });
    headers['Content-Type'] = 'application/json';
    if (reply.rawBody !== undefined) {
      return new Response(reply.rawBody, { status: reply.status, headers });
    }
    return new Response(JSON.stringify(reply.body ?? {}), { status: reply.status, headers });
  });
  return {
    fetch,
    calls,
    count: (r: Route | 'other') => calls.filter((c) => c.route === r).length,
  };
}

const POLL_INTERVAL_MS = 600_000;
let dispose: (() => void) | undefined;

function mount(s: ReturnType<typeof server>, props?: { org?: string }) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => <BranchesScreen fetch={s.fetch} org={props?.org} pollIntervalMs={POLL_INTERVAL_MS} />,
    host,
  );
  return host;
}

function unmount() {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
}

async function settle(ms = 0) {
  await vi.advanceTimersByTimeAsync(ms);
  for (let i = 0; i < 5; i += 1) await vi.advanceTimersByTimeAsync(0);
}

function buttonWithText(host: HTMLElement, label: string): HTMLButtonElement | undefined {
  return Array.from(host.querySelectorAll('button')).find(
    (b) => (b.textContent ?? '').trim() === label || b.getAttribute('aria-label') === label,
  );
}

function cards(host: HTMLElement): HTMLElement[] {
  return Array.from(host.querySelectorAll<HTMLElement>('.xc-branch-card'));
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] });
  window.localStorage.clear();
  setAudience('customer');
  setLang('th');
});

afterEach(() => {
  unmount();
  vi.useRealTimers();
  setLang('th');
  setAudience('staff');
});

describe('BranchesScreen host lookup error vs notLinked (#2335)', () => {
  it('row 1: Host answers 503 -> loadFailed + retry; never notLinked', async () => {
    const s = server({
      host: [{ status: 503, body: { error: 'serviceUnavailable' } }],
    });
    const host = mount(s);
    await settle();

    expect(host.textContent).toContain(text('loadFailed'));
    expect(host.textContent).not.toContain(text('notLinked'));
    expect(buttonWithText(host, text('retry'))).toBeDefined();
    expect(s.count('host')).toBe(1);
    expect(s.count('branches')).toBe(0);
    expect(cards(host)).toHaveLength(0);
  });

  it('row 2: Host fetch throws (network) -> loadFailed + retry; retry -> 200 with org -> branch cards shown', async () => {
    const s = server({
      host: ['network', { status: 200, body: { org: 'XC' } }],
    });
    const host = mount(s);
    await settle();

    expect(host.textContent).toContain(text('loadFailed'));
    expect(host.textContent).not.toContain(text('notLinked'));
    expect(cards(host)).toHaveLength(0);

    const retry = buttonWithText(host, text('retry'));
    expect(retry).toBeDefined();
    retry!.click();
    await settle();

    expect(s.count('host')).toBe(2);
    expect(s.count('branches')).toBe(1);
    expect(cards(host)).toHaveLength(2);
    expect(host.textContent).not.toContain(text('loadFailed'));
    expect(host.textContent).not.toContain(text('notLinked'));
  });

  it('row 3: Host 200 {} (no org) -> notLinked + retry', async () => {
    for (const body of [{}, { org: '' }, { org: null }]) {
      const s1 = server({ host: [{ status: 200, body }] });
      const host1 = mount(s1);
      await settle();

      expect(host1.textContent).toContain(text('notLinked'));
      expect(host1.textContent).not.toContain(text('loadFailed'));
      expect(buttonWithText(host1, text('retry'))).toBeDefined();
      expect(s1.count('branches')).toBe(0);
      unmount();
    }

    // After a 503 error (loadFailed), retrying into 200 {} transitions to notLinked + retry
    const s2 = server({
      host: [
        { status: 503, body: {} },
        { status: 200, body: {} },
      ],
    });
    const host2 = mount(s2);
    await settle();
    expect(host2.textContent).toContain(text('loadFailed'));
    expect(host2.textContent).not.toContain(text('notLinked'));

    const retry = buttonWithText(host2, text('retry'));
    expect(retry).toBeDefined();
    retry!.click();
    await settle();

    expect(host2.textContent).toContain(text('notLinked'));
    expect(host2.textContent).not.toContain(text('loadFailed'));
    expect(buttonWithText(host2, text('retry'))).toBeDefined();
    expect(s2.count('branches')).toBe(0);
  });

  it('row 4: Host 404 -> notLinked + retry, branches route never called', async () => {
    const s1 = server({ host: [{ status: 404, body: { error: 'notFound' } }] });
    const host1 = mount(s1);
    await settle();

    expect(host1.textContent).toContain(text('notLinked'));
    expect(host1.textContent).not.toContain(text('loadFailed'));
    expect(buttonWithText(host1, text('retry'))).toBeDefined();
    expect(s1.count('host')).toBe(1);
    expect(s1.count('branches')).toBe(0);
    unmount();

    // After a 503 error (loadFailed), retrying into 404 shows notLinked + retry and never calls branches
    const s2 = server({
      host: [
        { status: 503, body: {} },
        { status: 404, body: { error: 'notFound' } },
      ],
    });
    const host2 = mount(s2);
    await settle();
    expect(host2.textContent).toContain(text('loadFailed'));
    expect(host2.textContent).not.toContain(text('notLinked'));

    buttonWithText(host2, text('retry'))!.click();
    await settle();

    expect(host2.textContent).toContain(text('notLinked'));
    expect(host2.textContent).not.toContain(text('loadFailed'));
    expect(buttonWithText(host2, text('retry'))).toBeDefined();
    expect(s2.count('host')).toBe(2);
    expect(s2.count('branches')).toBe(0);
  });

  it('row 5: Branches route 500 -> loadFailed + retry', async () => {
    const s = server({
      host: [
        { status: 503, body: {} },
        { status: 200, body: { org: 'XC' } },
      ],
      branches: [
        { status: 500, body: {} },
        { status: 200, body: SAMPLE_BRANCHES, etag: '"b1-v1"' },
      ],
    });
    const host = mount(s);
    await settle();

    // Host 503 shows loadFailed (not notLinked)
    expect(host.textContent).toContain(text('loadFailed'));
    expect(host.textContent).not.toContain(text('notLinked'));

    // Retry resolves host 200 -> calls branches route (500) -> still shows loadFailed + retry
    buttonWithText(host, text('retry'))!.click();
    await settle();
    expect(s.count('host')).toBe(2);
    expect(s.count('branches')).toBe(1);
    expect(host.textContent).toContain(text('loadFailed'));
    expect(host.textContent).not.toContain(text('notLinked'));

    // Retry again re-fetches branches route (200) and displays branch cards
    buttonWithText(host, text('retry'))!.click();
    await settle();
    expect(s.count('host')).toBe(2);
    expect(s.count('branches')).toBe(2);
    expect(cards(host)).toHaveLength(2);
    expect(host.textContent).not.toContain(text('loadFailed'));
  });

  it('edge case 1: Host 200 whose body is not JSON (res.json() throws) -> error / loadFailed', async () => {
    const s = server({
      host: [{ status: 200, rawBody: '<html>502 Bad Gateway</html>' }],
    });
    const host = mount(s);
    await settle();

    expect(host.textContent).toContain(text('loadFailed'));
    expect(host.textContent).not.toContain(text('notLinked'));
    expect(buttonWithText(host, text('retry'))).toBeDefined();
    expect(s.count('host')).toBe(1);
    expect(s.count('branches')).toBe(0);
    expect(cards(host)).toHaveLength(0);
  });

  it('edge case 2: Retry while in error re-runs the host lookup', async () => {
    const s = server({
      host: [
        { status: 503, body: { error: 'unavailable' } },
        { status: 200, rawBody: 'not-json' },
        { status: 200, body: { org: 'XC' } },
      ],
    });
    const host = mount(s);
    await settle();

    expect(s.count('host')).toBe(1);
    expect(s.count('branches')).toBe(0);
    expect(host.textContent).toContain(text('loadFailed'));
    expect(host.textContent).not.toContain(text('notLinked'));

    // First retry while in error re-runs /api/public/host (which fails JSON parse -> stays in error)
    buttonWithText(host, text('retry'))!.click();
    await settle();
    expect(s.count('host')).toBe(2);
    expect(s.count('branches')).toBe(0);
    expect(host.textContent).toContain(text('loadFailed'));
    expect(host.textContent).not.toContain(text('notLinked'));

    // Second retry while in error re-runs /api/public/host (which succeeds -> loads branches)
    buttonWithText(host, text('retry'))!.click();
    await settle();
    expect(s.count('host')).toBe(3);
    expect(s.count('branches')).toBe(1);
    expect(cards(host)).toHaveLength(2);
    expect(host.textContent).not.toContain(text('loadFailed'));
  });
});
