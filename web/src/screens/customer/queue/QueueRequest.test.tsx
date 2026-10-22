// QW-2 (kiosk-shared-phone §5r F7, F8; rows U1–U5, U9): the customer's /app/queue page. Fill the request, show its
// QR, follow it, cancel it, e-mail it. Written before the implementation by the sub-controller claude/36d9dc6a.
// Do not edit, delete or weaken these tests; add tests in a file of your own.
//
// Contract (§4.3a W1–W5, W11; slice c's routes named by claude/mbp-xc-ks on #564 issuecomment-6324684576):
// - rates: GET /api/public/host → {org}; GET /api/public/orgs/{org}/branches/{code}/queue-rates → today's kiosk
//   `rates` JSON {branchId, branchName, open, rates[{code,name,nameEn,buy,sell}], paymentMethods}. The page is opened
//   with the branch **code** (`?branch=`); every command and view takes the branch **id** = that answer's `branchId`.
// - POST /api/app/web-requests {branch, type, currency, amount, paymentMethod?} → {requestId, requestQrUrl, expiresAt};
//   `requestQrUrl` is relative (`/q#<token>`): the page draws `<location.origin>/q#<token>`.
// - POST /api/app/web-requests/{id}/qr {branch} → the same shape (a fresh token; the old one stops working).
// - POST /api/app/web-requests/{id}/cancel {branch} → {requestId, status: "CANCELLED"}.
// - GET /api/v3/views/my_web_request?branch=&id=&since= → {version, changed, data?: {status, displayNumber, ...}}.
// - 409 alreadyQueued names the open request's id in `error.message` (PR 4481, `web_request_create.rs` refusal).
// - A replayed create answers `requestQrUrl: null`: the page then calls `/{id}/qr` for a fresh one.
// - my_profile.emailCopy {available, to} (W11).
//
// Pinned (QW-2 implements them):
// - `web/src/screens/customer/queue/QueueRequest.tsx` exports
//     QueueRequestScreen(props?: { fetch?: Fetch; search?: string; intervalMs?: number })
//   (`fetch` injected as in BookingsScreen; `search` defaults to location.search).
// - The form: select[name="type"] (BUY|SELL), select[name="currency"], input[name="amount"],
//   button[data-action="submit-request"].
// - After submit: [data-request-qr][data-qr-url="<origin>/q#<token>"], button[data-action="cancel-web-request"],
//   button[data-action="done"], and button[data-action="email-copy"] only when emailCopy.available.
// - [data-request-status] holds the status; [data-display-number] holds the number once there is one.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { QueueRequestScreen } from './QueueRequest';

interface Sent {
  method: string;
  path: string;
  search: string;
  body: any;
  headers: Headers;
}

const RATES = {
  branchId: 'b-id-7',
  branchName: 'XC-B',
  open: true,
  rates: [{ code: 'USD', name: 'ดอลลาร์', nameEn: 'US Dollar', buy: 32.1, sell: 32.6 }],
  paymentMethods: [],
};

let sent: Sent[];
let profile: unknown;
let profileStatus: number;
let createAnswer: () => Response;
let views: Array<() => Response>;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const fetchFn = vi.fn(async (input: string, init?: RequestInit) => {
  const u = new URL(input, 'http://localhost');
  const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
  sent.push({ method: (init?.method ?? 'GET').toUpperCase(), path: u.pathname, search: u.search, body, headers: new Headers(init?.headers) });
  if (u.pathname === '/api/public/host') return json(200, { org: 'ORG_1' });
  if (u.pathname === '/api/public/orgs/ORG_1/branches/MAIN/queue-rates') return json(200, RATES);
  if (u.pathname === '/api/v3/views/my_profile') return json(profileStatus, profileStatus === 200 ? { version: 1, changed: true, data: profile } : { error: { code: 'signIn', message: 'sign in' } });
  if (u.pathname === '/api/app/web-requests' && init?.method === 'POST') return createAnswer();
  if (/^\/api\/app\/web-requests\/[^/]+\/qr$/.test(u.pathname)) {
    return json(200, { requestId: 'k-old', requestQrUrl: '/q#tok-fresh', expiresAt: '2026-10-21T21:00:00Z' });
  }
  if (/^\/api\/app\/web-requests\/[^/]+\/cancel$/.test(u.pathname)) return json(200, { requestId: 'k-1', status: 'CANCELLED' });
  if (u.pathname === '/api/v3/views/my_web_request') {
    const next = views.shift();
    return next ? next() : json(200, { version: 5, changed: false });
  }
  return json(404, { error: { code: 'notFound', message: u.pathname } });
});

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await vi.advanceTimersByTimeAsync(0);
}

function q<T extends Element>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

function view(status: string, displayNumber: string | null, version: number) {
  return () =>
    json(200, {
      version,
      changed: true,
      data: {
        requestId: 'k-1', status, expiresAt: '2026-10-21T21:00:00Z', branch: { code: 'MAIN', name: 'XC-B' },
        type: 'BUY', currency: 'USD', amount: '100', displayNumber, ticketsAhead: displayNumber ? 2 : null, counter: null,
      },
    });
}

beforeEach(() => {
  vi.useFakeTimers();
  setAudience('customer');
  setLang('th');
  sent = [];
  profileStatus = 200;
  profile = { customerId: 'c-1', displayName: 'Somchai', emailCopy: { available: false, to: null } };
  createAnswer = () => json(200, { requestId: 'k-1', requestQrUrl: '/q#tok-1', expiresAt: '2026-10-21T21:00:00Z' });
  views = [view('QUOTED', null, 5)];
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

async function mount(search = '?branch=MAIN'): Promise<void> {
  dispose = render(() => <QueueRequestScreen fetch={fetchFn} search={search} intervalMs={3_000} />, host);
  await flush();
}

async function submit(amount = '100'): Promise<void> {
  const type = q<HTMLSelectElement>('select[name="type"]')!;
  type.value = 'BUY';
  type.dispatchEvent(new Event('change', { bubbles: true }));
  const cur = q<HTMLSelectElement>('select[name="currency"]')!;
  cur.value = 'USD';
  cur.dispatchEvent(new Event('change', { bubbles: true }));
  const amt = q<HTMLInputElement>('input[name="amount"]')!;
  amt.value = amount;
  amt.dispatchEvent(new Event('input', { bubbles: true }));
  q<HTMLButtonElement>('button[data-action="submit-request"]')!.click();
  await flush();
}

const creates = () => sent.filter((s) => s.path === '/api/app/web-requests' && s.method === 'POST');

describe('QW-2: /app/queue', () => {
  it('U1: signed out -> the sign-in link, and no request can be sent', async () => {
    profileStatus = 401;
    await mount();
    expect(q('a[href^="/app/sign-in"]'), 'link to /app/sign-in').not.toBeNull();
    expect(q('button[data-action="submit-request"]')).toBeNull();
  });

  it('U2: submit sends the request with the branch id; the QR of <origin>/q#<token> shows; no number yet', async () => {
    await mount();
    await submit();
    expect(creates()).toHaveLength(1);
    expect(creates()[0]!.body).toEqual({ branch: 'b-id-7', type: 'BUY', currency: 'USD', amount: '100' });
    expect(creates()[0]!.headers.get('Idempotency-Key'), 'Idempotency-Key').toBeTruthy();
    const qr = q<HTMLElement>('[data-request-qr]');
    expect(qr).not.toBeNull();
    expect(qr!.getAttribute('data-qr-url')).toBe(`${window.location.origin}/q#tok-1`);
    expect(q('[data-display-number]')?.textContent?.trim() ?? '').toBe('');
  });

  it('U3: the status follows my_web_request with since; WAITING shows the number', async () => {
    views = [view('QUOTED', null, 5), view('WAITING', 'K042', 6)];
    await mount();
    await submit();
    await vi.advanceTimersByTimeAsync(3_000);
    await flush();
    const reads = sent.filter((s) => s.path === '/api/v3/views/my_web_request');
    expect(reads.length).toBeGreaterThanOrEqual(2);
    expect(reads[0]!.search).toContain('id=k-1');
    expect(reads[0]!.search).toContain('branch=b-id-7');
    expect(reads[1]!.search).toMatch(/[?&]since=5(&|$)/);
    expect(q('[data-display-number]')?.textContent).toContain('K042');
  });

  it('U4: ยกเลิกคิว is one press with no dialog: one cancel for this request and branch; status CANCELLED', async () => {
    await mount();
    await submit();
    q<HTMLButtonElement>('button[data-action="cancel-web-request"]')!.click();
    await flush();
    const cancels = sent.filter((s) => /\/api\/app\/web-requests\/k-1\/cancel$/.test(s.path));
    expect(cancels).toHaveLength(1);
    expect(cancels[0]!.body).toEqual({ branch: 'b-id-7' });
    expect(document.querySelector('dialog[open]')).toBeNull();
    expect(q('[data-request-status]')?.getAttribute('data-request-status')).toBe('CANCELLED');
  });

  it('U5: no e-mail button without a verified e-mail; a button when emailCopy.available', async () => {
    await mount();
    await submit();
    expect(q('button[data-action="email-copy"]')).toBeNull();
    dispose?.();
    host.innerHTML = '';
    profile = { customerId: 'c-1', displayName: 'Somchai', emailCopy: { available: true, to: 'j***@example.com' } };
    await mount();
    await submit();
    expect(q('button[data-action="email-copy"]')).not.toBeNull();
    expect(host.textContent).toContain('j***@example.com');
  });

  it('U9: 409 alreadyQueued opens the existing request (a fresh QR for it, then its status)', async () => {
    createAnswer = () => json(409, { error: { code: 'alreadyQueued', message: 'k-old' } });
    await mount();
    await submit();
    expect(sent.some((s) => /\/api\/app\/web-requests\/k-old\/qr$/.test(s.path)), 'reissue for the open request').toBe(true);
    expect(q<HTMLElement>('[data-request-qr]')?.getAttribute('data-qr-url')).toBe(`${window.location.origin}/q#tok-fresh`);
  });
});
