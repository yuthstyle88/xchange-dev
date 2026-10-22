// BT-F2 (kiosk-shared-phone §7.3 T-D11): a paired tablet opened at ?branch=OTHER&from=sign-in uses the tablet's
// branch (MAIN), never OTHER, while an unpaired page at ?branch=OTHER keeps OTHER (T12).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { QueueRequestScreen } from './QueueRequest';

interface Sent {
  method: string;
  path: string;
  search: string;
  body: any;
}

const RATES = {
  branchId: 'b-id-7',
  branchName: 'XC-B',
  open: true,
  rates: [{ code: 'USD', name: 'ดอลลาร์', nameEn: 'US Dollar', buy: 32.1, sell: 32.6 }],
  paymentMethods: [],
};

const PAIRED_MAIN = { paired: true, tabletId: 'bt-1', branchCode: 'MAIN', name: 'Front' };

let sent: Sent[];
let tabletMe: () => Response;
let profileStatus: number;
let assign: ReturnType<typeof vi.fn>;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const fetchFn = vi.fn(async (input: string, init?: RequestInit) => {
  const u = new URL(input, 'http://localhost');
  const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
  sent.push({ method: (init?.method ?? 'GET').toUpperCase(), path: u.pathname, search: u.search, body });
  if (u.pathname === '/api/branch-tablet/me') return tabletMe();
  if (u.pathname === '/api/public/host') return json(200, { org: 'ORG_1' });
  if (u.pathname === '/api/public/orgs/ORG_1/branches/MAIN/queue-rates') return json(200, RATES);
  if (u.pathname === '/api/public/orgs/ORG_1/branches/OTHER/queue-rates') return json(200, { ...RATES, branchId: 'b-id-9' });
  if (u.pathname === '/api/v3/views/my_profile') {
    return profileStatus === 200
      ? json(200, { version: 1, changed: true, data: { customerId: 'c-1', displayName: 'Somchai', emailCopy: { available: false, to: null } } })
      : json(profileStatus, { error: { code: 'signIn', message: 'sign in' } });
  }
  if (u.pathname === '/api/app/web-requests' && init?.method === 'POST') {
    return json(200, { requestId: 'k-1', requestQrUrl: '/q#tok-1', expiresAt: '2026-10-21T21:00:00Z' });
  }
  if (u.pathname === '/api/v3/views/my_web_request') {
    return json(200, { version: 5, changed: false });
  }
  return json(404, { error: { code: 'notFound', message: u.pathname } });
});

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(0);
}

function q<T extends Element>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

async function mount(search: string): Promise<void> {
  vi.stubGlobal('location', { ...window.location, assign, reload: vi.fn(), search, hash: '', pathname: '/app/queue' });
  dispose = render(() => <QueueRequestScreen fetch={fetchFn} search={search} intervalMs={3_000} />, host);
  await flush();
}

async function submit(): Promise<void> {
  const type = q<HTMLSelectElement>('select[name="type"]')!;
  type.value = 'BUY';
  type.dispatchEvent(new Event('change', { bubbles: true }));
  const cur = q<HTMLSelectElement>('select[name="currency"]')!;
  cur.value = 'USD';
  cur.dispatchEvent(new Event('change', { bubbles: true }));
  const amt = q<HTMLInputElement>('input[name="amount"]')!;
  amt.value = '100';
  amt.dispatchEvent(new Event('input', { bubbles: true }));
  q<HTMLButtonElement>('button[data-action="submit-request"]')!.click();
  await flush();
}

const creates = () => sent.filter((s) => s.path === '/api/app/web-requests' && s.method === 'POST');
const ratesReads = (code: string) => sent.filter((s) => s.path === `/api/public/orgs/ORG_1/branches/${code}/queue-rates`);

beforeEach(() => {
  vi.useFakeTimers();
  setAudience('customer');
  setLang('th');
  sent = [];
  tabletMe = () => json(200, PAIRED_MAIN);
  profileStatus = 200;
  assign = vi.fn();
  fetchFn.mockClear();
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('BT-F2 T-D11: withBranch at ?branch=OTHER&from=sign-in', () => {
  it('G1: paired tablet at ?branch=OTHER&from=sign-in reads rates for MAIN and never OTHER', async () => {
    await mount('?branch=OTHER&from=sign-in');
    expect(ratesReads('MAIN')).toHaveLength(1);
    expect(ratesReads('OTHER')).toHaveLength(0);
  });

  it('G2: paired tablet at ?branch=OTHER&from=sign-in submits webRequestCreate with MAIN branch id, never OTHER', async () => {
    await mount('?branch=OTHER&from=sign-in');
    await submit();
    expect(creates()).toHaveLength(1);
    expect(creates()[0]!.body.branch).toBe('b-id-7');
    expect(creates()[0]!.body.branch).not.toBe('b-id-9');
  });

  it('G3: not paired at ?branch=OTHER reads rates for OTHER (T12)', async () => {
    tabletMe = () => json(200, { paired: false });
    await mount('?branch=OTHER');
    expect(ratesReads('OTHER')).toHaveLength(1);
    expect(ratesReads('MAIN')).toHaveLength(0);
  });
});
