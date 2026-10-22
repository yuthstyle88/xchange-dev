// Up-front tests for QW-2e (kiosk-shared-phone §5r.4a). Do not edit, delete or weaken; add tests in a file of your own.
//
// RequestStatus, the request's live status (§4.3a W4, W4a, W5; §5r F8). Rows: U3, U4 (the state's status shows at
// once) and the expiry. Only RequestStatus is under test; `my_web_request` comes from an injected fetch.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../../../shared/api/client';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from './i18n';
import type { WebRequestStatus } from './queue-api';
import { RequestStatus } from './RequestStatus';

const EXPIRES = '2026-10-21T21:00:00Z';
const BEFORE = new Date('2026-10-21T20:00:00Z');
const AFTER = new Date('2026-10-21T21:00:01Z');
const INTERVAL = 3_000;

let reads: string[];
let views: Array<() => Response>;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let onExpired: ReturnType<typeof vi.fn<() => void>>;
let setStatus: (s: WebRequestStatus | null) => void;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

/** A changed `my_web_request` answer; `version` is W4a's opaque string. */
function view(status: WebRequestStatus, displayNumber: string | null, version: string) {
  return () =>
    json(200, {
      version,
      changed: true,
      data: {
        requestId: 'k-1',
        status,
        expiresAt: status === 'QUOTED' ? EXPIRES : null,
        branch: { code: 'MAIN', name: 'XC-B' },
        type: 'BUY',
        currency: 'USD',
        amount: '100',
        displayNumber,
        ticketsAhead: displayNumber ? 2 : null,
      },
    });
}

const notFound = () => json(404, { error: { code: 'notFound', message: 'notFound' } });

const fetchFn = async (input: string): Promise<Response> => {
  const u = new URL(input, 'http://localhost');
  if (u.pathname !== '/api/v3/views/my_web_request') return notFound();
  reads.push(u.search);
  const next = views.shift();
  return next ? next() : json(200, { version: 'last', changed: false });
};

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(0);
}

async function tick(): Promise<void> {
  await vi.advanceTimersByTimeAsync(INTERVAL);
  await flush();
}

function q<T extends Element>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

const shownStatus = () => q('[data-request-status]')?.getAttribute('data-request-status') ?? null;

async function mount(): Promise<void> {
  const [status, set] = createSignal<WebRequestStatus | null>(null);
  setStatus = set;
  dispose = render(
    () => (
      <RequestStatus
        client={new ApiClient(fetchFn)}
        branchId="b-id-7"
        requestId="k-1"
        expiresAt={EXPIRES}
        intervalMs={INTERVAL}
        status={status()}
        onExpired={onExpired}
      />
    ),
    host,
  );
  await flush();
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(BEFORE);
  setAudience('customer');
  setLang('th');
  reads = [];
  views = [];
  onExpired = vi.fn();
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.useRealTimers();
});

describe('QW-2e: RequestStatus', () => {
  it('U3: one poller; the first read names the request and the branch, every later read carries since=<version>', async () => {
    views = [view('QUOTED', null, '5.00aa'), view('WAITING', 'K042', '6.00bb')];
    await mount();
    expect(reads).toHaveLength(1);
    expect(reads[0]).toContain('id=k-1');
    expect(reads[0]).toContain('branch=b-id-7');
    expect(reads[0]).not.toContain('since=');
    await tick();
    expect(reads).toHaveLength(2);
    expect(reads[1]).toMatch(/[?&]since=5\.00aa(&|$)/);
    await tick();
    expect(reads, 'one poller: one read per interval').toHaveLength(3);
    expect(reads[2]).toMatch(/[?&]since=6\.00bb(&|$)/);
  });

  it('U3: QUOTED shows no number; WAITING shows the number and the place in line', async () => {
    views = [view('QUOTED', null, '5.00aa'), view('WAITING', 'K042', '6.00bb')];
    await mount();
    expect(shownStatus()).toBe('QUOTED');
    expect(q('[data-display-number]')?.textContent?.trim() ?? '').toBe('');
    await tick();
    expect(shownStatus()).toBe('WAITING');
    expect(q('[data-display-number]')?.textContent).toContain('K042');
    expect(host.textContent).toContain(th.ahead.replace('{count}', '2'));
    expect(q('[data-request-status]')?.textContent).toContain(th.status_WAITING);
  });

  it('U4: the state status CANCELLED shows at once, with no poll in between', async () => {
    views = [view('QUOTED', null, '5.00aa')];
    await mount();
    expect(shownStatus()).toBe('QUOTED');
    const before = reads.length;
    setStatus('CANCELLED');
    await Promise.resolve();
    expect(shownStatus()).toBe('CANCELLED');
    expect(reads).toHaveLength(before);
  });

  it('expiry: a 404 after expiresAt calls onExpired, once', async () => {
    views = [view('QUOTED', null, '5.00aa'), notFound, notFound, notFound];
    await mount();
    vi.setSystemTime(AFTER);
    await tick();
    expect(onExpired).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(INTERVAL * 10);
    await flush();
    expect(onExpired).toHaveBeenCalledTimes(1);
  });

  it('expiry: a 404 before expiresAt is not an expiry', async () => {
    views = [view('QUOTED', null, '5.00aa'), notFound];
    await mount();
    await tick();
    expect(reads, 'the 404 was read').toHaveLength(2);
    expect(onExpired).not.toHaveBeenCalled();
  });

  it('unmount stops the poller', async () => {
    views = [view('QUOTED', null, '5.00aa')];
    await mount();
    const before = reads.length;
    expect(before, 'it polled while mounted').toBeGreaterThan(0);
    dispose?.();
    dispose = undefined;
    await vi.advanceTimersByTimeAsync(INTERVAL * 5);
    expect(reads).toHaveLength(before);
  });
});
