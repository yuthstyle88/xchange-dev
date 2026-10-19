// C03 up-front tests (feat/fe-customer skeleton): loadBranchNames names a booking's branch from the public
// branch list (branch code == branch id), and My Bookings shows that name. The implementer makes these pass
// and must not edit them.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import type { MyBookingsData } from '../../../shared/api/generated/my_bookings';
import { setLang } from '../../../shared/i18n';
import { BookingsScreen } from './Bookings';
import { loadBranchNames } from './branchNames';

type Reply = { status: number; body: unknown } | 'network';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const BRANCHES = {
  branches: [
    { code: 'bkk-1', name: 'Siam Square', address: null, openingHours: null, timezone: 'Asia/Bangkok', bookingEnabled: true, queueOpen: true },
    { code: 'bkk-2', name: 'Don Mueang', address: null, openingHours: null, timezone: 'Asia/Bangkok', bookingEnabled: false, queueOpen: false },
  ],
};

const BOOKINGS: MyBookingsData = {
  bookings: [
    {
      id: 'bk-1', bookingNo: 'B261015-0001', status: 'CONFIRMED', branchId: 'bkk-1', slotId: 's1',
      startAt: '2099-10-15T10:00:00Z', endAt: '2099-10-15T10:30:00Z', qrToken: null, quoteSummary: '',
      cancelCutoffAt: null, ticketNumber: null, ticketsAhead: null, estimateSeconds: null, estimateLowSeconds: null, estimateHighSeconds: null, estimateAsOf: null, readiness: 'unknown',
    },
    {
      id: 'bk-2', bookingNo: 'B261015-0002', status: 'CONFIRMED', branchId: 'zz-9', slotId: 's2',
      startAt: '2099-10-16T10:00:00Z', endAt: '2099-10-16T10:30:00Z', qrToken: null, quoteSummary: '',
      cancelCutoffAt: null, ticketNumber: null, ticketsAhead: null, estimateSeconds: null, estimateLowSeconds: null, estimateHighSeconds: null, estimateAsOf: null, readiness: 'unknown',
    },
  ],
};

function server(over: { host?: Reply; branches?: Reply } = {}) {
  const paths: string[] = [];
  const fetchFn = vi.fn(async (input: string) => {
    const path = new URL(input, 'http://localhost').pathname;
    paths.push(path);
    let r: Reply | undefined;
    if (path === '/api/public/host') r = over.host ?? { status: 200, body: { org: 'XC' } };
    else if (path === '/api/public/orgs/XC/branches') r = over.branches ?? { status: 200, body: BRANCHES };
    else if (path === '/api/v3/views/my_bookings') r = { status: 200, body: { version: 1, changed: true, data: BOOKINGS } };
    else return json(404, { error: { code: 'notFound', message: 'Not found' } });
    if (r === 'network') throw new TypeError('Failed to fetch');
    return json(r.status, r.body);
  });
  return { fetchFn, paths };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('loadBranchNames (C03)', () => {
  it('maps each public branch code to its name, reading the host org first', async () => {
    const s = server();
    const names = await loadBranchNames(s.fetchFn);
    expect(Object.fromEntries(names)).toEqual({ 'bkk-1': 'Siam Square', 'bkk-2': 'Don Mueang' });
    expect(s.paths).toEqual(['/api/public/host', '/api/public/orgs/XC/branches']);
  });

  it.each<[string, { host?: Reply; branches?: Reply }]>([
    ['the host fails', { host: { status: 500, body: {} } }],
    ['the host has no org', { host: { status: 200, body: {} } }],
    ['the branch list fails', { branches: { status: 503, body: {} } }],
    ['the network is down', { branches: 'network' }],
  ])('answers an empty map, never throws, when %s', async (_why, over) => {
    await expect(loadBranchNames(server(over).fetchFn)).resolves.toEqual(new Map());
  });
});

describe('My Bookings shows the branch name (C03)', () => {
  it('a known branch shows its name; an unknown one keeps its id', async () => {
    setLang('en');
    const s = server();
    host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => <BookingsScreen client={new ApiClient(s.fetchFn)} fetch={s.fetchFn} />, host);
    await flush();
    const branchOf = (id: string) =>
      host!.querySelector(`[data-booking-id="${id}"] .xc-booking-branch`)?.textContent;
    expect(branchOf('bk-1')).toBe('Siam Square');
    expect(branchOf('bk-2')).toBe('zz-9');
  });
});
