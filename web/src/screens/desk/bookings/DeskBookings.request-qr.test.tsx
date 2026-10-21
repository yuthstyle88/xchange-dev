// QW-1 (kiosk-shared-phone §5r F13, rows D1, D2): the desk's existing QR box also takes a customer's web-request QR.
// A scanned `<origin>/q#<token>` calls `findByToken` (§4.3a W8) and opens the request at the desk; a booking QR works
// as today. Written before the implementation by the sub-controller claude/36d9dc6a. Do not edit, delete or weaken
// these tests; add tests in a file of your own.
//
// Pinned:
// - D1: typing (or a keyboard-wedge scanner entering) `https://h/q#tok` in input[name="qrToken"] and pressing
//   button[data-action="lookup-qr"] sends exactly one `POST /api/kiosk` with
//   `{action: "findByToken", payload: {branchId: <the desk's branch>, token: "tok"}}` and **no** desk_bookings
//   `qr` read; on `{requestId}` it calls `window.location.assign('/desk?branch=<branch>&kiosk=<requestId>')`.
// - A `404 notFound` shows a refusal in role="alert" and does not navigate.
// - D2: a booking token (not `/q#…`) does today's `desk_bookings` read with `qr` and never calls `/api/kiosk`.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { makeDeskBookingsData, makeMe, makeMeRole } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { DeskBookingsScreen } from './DeskBookingsScreen';

const BRANCH_ID = 'XC-B01';

const STAFF_TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: BRANCH_ID })],
  branch: BRANCH_ID,
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
});

interface Sent {
  method: string;
  path: string;
  search: string;
  body: any;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let sent: Sent[];
let findAnswer: () => Response;
let assign: ReturnType<typeof vi.fn>;

beforeAll(() => warmModules(() => import('./DeskBookingsScreen')), 60_000);

beforeEach(() => {
  setAudience('staff');
  setLang('en');
  document.body.innerHTML = '';
  host = document.createElement('div');
  document.body.appendChild(host);
  sent = [];
  assign = vi.fn();
  findAnswer = () => json(200, { requestId: 'k-9' });
  vi.stubGlobal('location', { ...window.location, assign, search: `?branch=${BRANCH_ID}`, pathname: '/desk/bookings' });
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      sent.push({ method: (init?.method ?? 'GET').toUpperCase(), path: u.pathname, search: u.search, body });
      if (u.pathname === '/api/v3/views/me') return json(200, { version: 1, changed: true, data: STAFF_TELLER });
      if (u.pathname === '/api/v3/views/desk_bookings') {
        return json(200, { version: 1, changed: true, data: makeDeskBookingsData({ items: [] }) });
      }
      if (u.pathname === '/api/kiosk' && body?.action === 'findByToken') return findAnswer();
      return json(404, { error: { code: 'notFound', message: u.pathname } });
    }),
  );
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

async function scan(value: string): Promise<void> {
  dispose = render(() => <DeskBookingsScreen />, host);
  await flush();
  const input = host.querySelector<HTMLInputElement>('input[name="qrToken"]')!;
  expect(input, 'input[name=qrToken] (DeskBookingsScreen.tsx:620)').not.toBeNull();
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  host.querySelector<HTMLButtonElement>('button[data-action="lookup-qr"]')!.click();
  await flush();
}

const kioskCalls = () => sent.filter((s) => s.path === '/api/kiosk');
const qrReads = () => sent.filter((s) => s.path === '/api/v3/views/desk_bookings' && /(^|[?&])qr=/.test(s.search));

describe('QW-1: the desk QR box takes a web-request QR', () => {
  it('D1: /q#tok -> one findByToken for this branch, then opens /desk?branch=…&kiosk=<id>', async () => {
    await scan('https://xc-v3.spacing.cloud/q#tok');
    expect(kioskCalls()).toHaveLength(1);
    expect(kioskCalls()[0]!.method).toBe('POST');
    expect(kioskCalls()[0]!.body).toEqual({ action: 'findByToken', payload: { branchId: BRANCH_ID, token: 'tok' } });
    expect(qrReads(), 'a request QR is not a booking lookup').toHaveLength(0);
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign).toHaveBeenCalledWith(`/desk?branch=${BRANCH_ID}&kiosk=k-9`);
  });

  it('D1: 404 notFound -> a refusal shows and the page stays', async () => {
    findAnswer = () => json(404, { error: { code: 'notFound', message: 'not found' } });
    await scan('https://h/q#unknown');
    expect(host.querySelector('[role="alert"]'), 'refusal in role=alert').not.toBeNull();
    expect(assign).not.toHaveBeenCalled();
  });

  it('D2: a booking token -> today\'s desk_bookings qr read, never /api/kiosk', async () => {
    await scan('test-qr-token-xyz');
    expect(kioskCalls()).toHaveLength(0);
    expect(qrReads().length).toBeGreaterThanOrEqual(1);
    expect(assign).not.toHaveBeenCalled();
  });
});
