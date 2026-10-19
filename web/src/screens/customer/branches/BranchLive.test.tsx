// C08 up-front tests (feat/fe-customer skeleton): BranchLive shows one branch's waiting count, open counters
// and wait estimate from branch_service_status, and today's free slots from booking_slots (both public views,
// `{version, changed, data}` envelope). The implementer makes these pass and must not edit them.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { Fetch } from '../../../shared/api/client';
import type { BookingSlotsData } from '../../../shared/api/generated/booking_slots';
import type { BranchServiceStatusData } from '../../../shared/api/generated/branch_service_status';
import { setLang } from '../../../shared/i18n';
import { BranchLive } from './BranchLive';

type Reply = { status: number; body: unknown } | 'network';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}
const envelope = (data: unknown) => ({ status: 200, body: { version: 1, changed: true, data } });

const STATUS: BranchServiceStatusData = {
  branch: 'bkk-1',
  queueOpen: true,
  activeCounters: 2,
  waiting: { walkIn: 3, booking: 2 },
  estimateSeconds: 420,
  estimateLowSeconds: 240,
  estimateHighSeconds: 600,
  estimateAsOf: '2026-10-19T08:00:00Z',
};

const SLOTS: BookingSlotsData = {
  asOf: '2026-10-19T08:00:00Z',
  slots: [
    { id: 's-past', start: '2000-01-01T03:00:00Z', end: '2000-01-01T03:30:00Z', available: 4 },
    { id: 's-free', start: '2099-01-01T03:00:00Z', end: '2099-01-01T03:30:00Z', available: 3 },
    { id: 's-full', start: '2099-01-01T04:00:00Z', end: '2099-01-01T04:30:00Z', available: 0 },
    { id: 's-free2', start: '2099-01-01T05:00:00Z', end: '2099-01-01T05:30:00Z', available: 1 },
  ],
};

function server(over: { status?: Reply; slots?: Reply } = {}) {
  const urls: URL[] = [];
  const fetchFn: Fetch = vi.fn(async (input: string) => {
    const u = new URL(input, 'http://localhost');
    urls.push(u);
    let r: Reply;
    if (u.pathname === '/api/v3/views/branch_service_status') r = over.status ?? envelope(STATUS);
    else if (u.pathname === '/api/v3/views/booking_slots') r = over.slots ?? envelope(SLOTS);
    else return json(404, { error: { code: 'notFound', message: 'Not found' } });
    if (r === 'network') throw new TypeError('Failed to fetch');
    return json(r.status, r.body);
  });
  return { fetchFn, urls };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
function mount(fetchFn: Fetch, bookingEnabled = true): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <BranchLive branch="bkk-1" bookingEnabled={bookingEnabled} fetch={fetchFn} />, host);
}
const text = (id: string) => host.querySelector(`[data-testid="${id}"]`)?.textContent ?? null;

beforeEach(() => setLang('en'));
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('BranchLive (C08)', () => {
  it('reads branch_service_status and booking_slots for its own branch, with no date (the server uses the branch\'s today)', async () => {
    const s = server();
    mount(s.fetchFn);
    await flush();
    const status = s.urls.find((u) => u.pathname.endsWith('/branch_service_status'));
    const slots = s.urls.find((u) => u.pathname.endsWith('/booking_slots'));
    expect(status?.searchParams.get('branch')).toBe('bkk-1');
    expect(slots?.searchParams.get('branch')).toBe('bkk-1');
    expect(slots?.searchParams.has('date')).toBe(false);
  });

  it('shows walk-in plus booking as the waiting count, and the open counters', async () => {
    mount(server().fetchFn);
    await flush();
    expect(text('live-waiting')).toContain('5');
    expect(text('live-counters')).toContain('2');
  });

  it('shows the estimate range in whole minutes, rounded up', async () => {
    mount(server({ status: envelope({ ...STATUS, estimateLowSeconds: 241, estimateHighSeconds: 600 }) }).fetchFn);
    await flush();
    expect(text('live-estimate')).toMatch(/5\D+10/);
  });

  it('shows no estimate when the server has nothing to estimate from', async () => {
    mount(server({ status: envelope({ ...STATUS, estimateSeconds: null, estimateLowSeconds: null, estimateHighSeconds: null, estimateAsOf: null }) }).fetchFn);
    await flush();
    expect(text('live-waiting')).toContain('5');
    expect(host.querySelector('[data-testid="live-estimate"]')).toBeNull();
  });

  it('counts today\'s free slots: not yet started and with places left', async () => {
    mount(server().fetchFn);
    await flush();
    expect(text('live-free-slots')).toContain('2');
  });

  it('a branch that takes no bookings reads no slots and shows none', async () => {
    const s = server();
    mount(s.fetchFn, false);
    await flush();
    expect(s.urls.some((u) => u.pathname.endsWith('/booking_slots'))).toBe(false);
    expect(host.querySelector('[data-testid="live-free-slots"]')).toBeNull();
    expect(text('live-waiting')).toContain('5');
  });

  it('a failed status read hides the status parts but still shows the slots', async () => {
    mount(server({ status: { status: 500, body: {} } }).fetchFn);
    await flush();
    expect(host.querySelector('[data-testid="live-waiting"]')).toBeNull();
    expect(host.querySelector('[data-testid="live-counters"]')).toBeNull();
    expect(text('live-free-slots')).toContain('2');
  });

  it('a failed slots read hides the slots but still shows the status', async () => {
    mount(server({ slots: 'network' }).fetchFn);
    await flush();
    expect(host.querySelector('[data-testid="live-free-slots"]')).toBeNull();
    expect(text('live-waiting')).toContain('5');
  });
});
