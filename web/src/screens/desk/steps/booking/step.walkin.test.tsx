import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { th as sharedTh } from '../../../../shared/i18n/th';
import { makeDeskConfig, makeQueueTicketDto } from '../../../../shared/testing/fixtures';
import { OWNER_DESK } from '../../desk-path';
import { createDeskStore, type DeskEntryState, type DeskStore } from '../../desk-store';
import { step as bookingStep } from './step';
import * as texts from './i18n';

const ME = () => ({
  userId: 'u-1',
  username: 'ploy',
  name: 'Ploy',
  roles: [],
  branch: 'MAIN',
  branchId: 'MAIN',
  branchName: 'XC-B',
  branches: [],
});

type Call = {
  method: string;
  path: string;
  key: string | null;
  body: unknown;
};

type Answer = { status: number; body?: unknown; throws?: boolean };
type Route = (call: Call, nth: number) => Answer | Promise<Answer>;

function stubServer() {
  const calls: Call[] = [];
  const routes = new Map<string, Route>();
  const counts = new Map<string, number>();

  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = String(init?.method ?? 'GET').toUpperCase();
    const path = String(input).split('?')[0]!;
    const headers = new Headers(init?.headers);
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
    const call: Call = { method, path, key: headers.get('idempotency-key'), body };
    calls.push(call);
    const k = `${method} ${path}`;
    const n = counts.get(k) ?? 0;
    counts.set(k, n + 1);
    const route = routes.get(k);
    const a: Answer = route
      ? await route(call, n)
      : { status: 404, body: { error: { code: 'notFound', message: 'not found' } } };
    if (a.throws) throw new TypeError('Failed to fetch');
    return new Response(JSON.stringify(a.body ?? {}), {
      status: a.status,
      headers: { 'content-type': 'application/json' },
    });
  });

  routes.set('GET /api/v3/views/desk_config', () => ({
    status: 200,
    body: { version: 1, changed: true, data: makeDeskConfig({ shiftOpen: true }) },
  }));

  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    calls,
    serveCalls: () => calls.filter((c) => c.method === 'POST' && c.path === '/api/queue/tickets/serve'),
    onServe(route: Route) {
      routes.set('POST /api/queue/tickets/serve', route);
    },
  };
}

let disposers: (() => void)[] = [];
let host: HTMLDivElement;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  vi.stubGlobal('confirm', vi.fn(() => true));
});

afterEach(() => {
  for (const d of disposers) d();
  disposers = [];
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

function readyStore(s: ReturnType<typeof stubServer>, customerId = ''): DeskStore {
  let store!: DeskStore;
  createRoot((dispose) => {
    disposers.push(dispose);
    store = createDeskStore({ api: s.api, path: OWNER_DESK, me: ME as never });
  });
  store.setField('name', 'JOHN DOE', 'user');
  store.setValueDate('2026-10-28');
  if (customerId !== '') store.setCustomerId(customerId);
  store.go(2);
  disposers.push(render(() => bookingStep.component({ store }), host));
  return store;
}

const submitBtn = () => host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
const ticketRow = () => host.querySelector('[data-row="walkin-ticket"]');
const alertEl = () => host.querySelector('[role="alert"]');

describe('steps/booking walk-in queue ticket (#5352)', () => {
  it('1. sale with bookingId (or kioskRequestId): no request to /api/queue/tickets/serve; go(3) as today', async () => {
    const cases: Record<string, string | boolean>[] = [{ bookingId: 'bk-1' }, { kioskRequestId: 'kr-1' }];
    for (const extras of cases) {
      const s = stubServer();
      const store = readyStore(s);
      store.setEntry({
        kind: 'kiosk',
        id: 'e-1',
        banner: () => '',
        locked: true,
        identityConfirmed: () => true,
        setIdentityConfirmed: () => {},
        payloadExtras: () => extras,
        refusal: () => undefined,
      } satisfies DeskEntryState);

      submitBtn().click();
      expect(s.serveCalls()).toHaveLength(0);
      expect(store.step()).toBe(3);

      for (const d of disposers) d();
      disposers = [];
      host.innerHTML = '';
    }
  });

  it('2. walk-in, server 200 {displayNumber:"W012", status:"SERVING"}: one POST with {customerId:null} and an Idempotency-Key; "คิว W012" shown; step 3', async () => {
    const s = stubServer();
    s.onServe(() => ({
      status: 200,
      body: makeQueueTicketDto({ displayNumber: 'W012', status: 'SERVING' }),
    }));
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));

    const calls = s.serveCalls();
    expect(calls).toHaveLength(1);
    expect(calls[0]!.body).toEqual({ customerId: null });
    expect(calls[0]!.key).toBeTruthy();
    expect(store.walkInTicket()?.displayNumber).toBe('W012');
    expect(ticketRow()?.textContent).toBe('คิว W012');
  });

  it('3. walk-in with a customer id: body {customerId:"<id>"}', async () => {
    const s = stubServer();
    s.onServe(() => ({
      status: 200,
      body: makeQueueTicketDto({ displayNumber: 'W013', status: 'SERVING' }),
    }));
    const store = readyStore(s, 'CX-00000001');

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));

    const calls = s.serveCalls();
    expect(calls).toHaveLength(1);
    expect(calls[0]!.body).toEqual({ customerId: 'CX-00000001' });
  });

  it('4. 409 queueClosed: Thai refusal on role="alert"; stays on step 2', async () => {
    const s = stubServer();
    s.onServe(() => ({
      status: 409,
      body: { error: { code: 'queueClosed', message: 'queueClosed' } },
    }));
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(alertEl()).not.toBeNull());

    expect(alertEl()!.textContent).toBe((texts.th as Record<string, string>)['error.queueClosed']);
    expect(alertEl()!.textContent).toMatch(/[\u0E00-\u0E7F]/);
    expect(store.step()).toBe(2);
  });

  it('5. 409 outsideServiceHours: its Thai text; stays on step 2', async () => {
    const s = stubServer();
    s.onServe(() => ({
      status: 409,
      body: { error: { code: 'outsideServiceHours', message: 'outsideServiceHours' } },
    }));
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(alertEl()).not.toBeNull());

    expect(alertEl()!.textContent).toBe((texts.th as Record<string, string>)['error.outsideServiceHours']);
    expect(alertEl()!.textContent).toMatch(/[\u0E00-\u0E7F]/);
    expect(store.step()).toBe(2);
  });

  it('6. refusal, then retry: the same Idempotency-Key both times', async () => {
    const s = stubServer();
    s.onServe((_call, n) =>
      n === 0
        ? { status: 409, body: { error: { code: 'queueClosed', message: 'queueClosed' } } }
        : { status: 200, body: makeQueueTicketDto({ displayNumber: 'W014', status: 'SERVING' }) },
    );
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(alertEl()).not.toBeNull());
    expect(store.step()).toBe(2);

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));

    const calls = s.serveCalls();
    expect(calls).toHaveLength(2);
    expect(calls[0]!.key).toBeTruthy();
    expect(calls[1]!.key).toBe(calls[0]!.key);
  });

  it('7. startNew(), then a new walk-in: a different key; no old ticket shown', async () => {
    const s = stubServer();
    s.onServe((_call, n) => ({
      status: 200,
      body: makeQueueTicketDto({ id: `t-${n}`, displayNumber: n === 0 ? 'W015' : 'W016', status: 'SERVING' }),
    }));
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));
    expect(ticketRow()?.textContent).toBe('คิว W015');

    store.startNew();
    expect(store.walkInTicket()).toBeUndefined();
    expect(ticketRow()).toBeNull();

    store.setField('name', 'JANE DOE', 'user');
    store.setValueDate('2026-10-28');
    store.go(2);
    expect(ticketRow()).toBeNull();

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));

    const calls = s.serveCalls();
    expect(calls).toHaveLength(2);
    expect(calls[1]!.key).toBeTruthy();
    expect(calls[1]!.key).not.toBe(calls[0]!.key);
    expect(ticketRow()?.textContent).toBe('คิว W016');
  });

  it('8. back-to-edit after success: no new request; the same number shown', async () => {
    const s = stubServer();
    s.onServe(() => ({
      status: 200,
      body: makeQueueTicketDto({ displayNumber: 'W017', status: 'SERVING' }),
    }));
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));
    expect(s.serveCalls()).toHaveLength(1);

    store.backToEdit();
    expect(store.step()).toBe(0);
    expect(store.walkInTicket()?.displayNumber).toBe('W017');

    store.go(2);
    expect(ticketRow()?.textContent).toBe('คิว W017');

    submitBtn().click();
    expect(s.serveCalls()).toHaveLength(1);
    expect(store.step()).toBe(3);
    expect(ticketRow()?.textContent).toBe('คิว W017');
  });

  it('9. network TypeError: the shared network text; stays on step 2; the same key on retry', async () => {
    const s = stubServer();
    s.onServe((_call, n) =>
      n === 0
        ? { status: 0, throws: true }
        : { status: 200, body: makeQueueTicketDto({ displayNumber: 'W018', status: 'SERVING' }) },
    );
    const store = readyStore(s);

    submitBtn().click();
    await vi.waitFor(() => expect(alertEl()).not.toBeNull());
    expect(alertEl()!.textContent).toBe(sharedTh['error.network']);
    expect(store.step()).toBe(2);

    submitBtn().click();
    await vi.waitFor(() => expect(store.step()).toBe(3));

    const calls = s.serveCalls();
    expect(calls).toHaveLength(2);
    expect(calls[0]!.key).toBeTruthy();
    expect(calls[1]!.key).toBe(calls[0]!.key);
  });
});
