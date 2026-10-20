import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeAdminUserRole, makeAdminUserRow } from '../../../shared/testing/fixtures';
import { UsersScreen } from './UsersScreen';

// #2413: a second click on a row write (revoke × or Disable) while the first is
// in flight must not send a second request. The fake server below models the
// real idempotency store: a request whose key is still PENDING is answered 409
// idempotencyConflict (api/src/infrastructure/idempotency/pg_store.rs).

interface WriteCall {
  path: string;
  key: string | null;
  rawBody: string;
}

type Responder = () => Promise<Response> | Response;

let writes: WriteCall[] = [];
let queue: Responder[] = [];
let pendingKeys: Set<string> = new Set();
const disposers: Array<() => void> = [];

const B01 = 'XC-B01';
const SUCCESS_REVOKED_TH = 'ยกเลิกสิทธิ์เรียบร้อยแล้ว';

const SEED_USERS = [
  makeAdminUserRow({
    userId: 'u-1',
    name: 'Somchai Active',
    status: 'ACTIVE',
    version: 1,
    roles: [
      makeAdminUserRole({
        role: 'TELLER',
        branch: B01,
        validFrom: '2026-10-01T00:00:00Z',
        validTo: null,
      }),
    ],
  }),
];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function deferred(): { promise: Promise<Response>; resolve: (r: Response) => void; reject: (e: unknown) => void } {
  let resolve!: (r: Response) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<Response>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const ROW_WRITES = new Set(['/api/users/revoke', '/api/users/disable', '/api/users/grant']);

async function fakeFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const u = new URL(String(input), 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();

  if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches') {
    return json(200, { version: 1, changed: true, data: { branches: [{ id: B01, name: 'Silom Branch' }] } });
  }
  if (method === 'GET' && u.pathname === '/api/v3/views/admin_users') {
    return json(200, { version: 1, changed: true, data: structuredClone(SEED_USERS) });
  }
  if (method === 'POST' && ROW_WRITES.has(u.pathname)) {
    const key = new Headers(init?.headers).get('idempotency-key');
    writes.push({ path: u.pathname, key, rawBody: typeof init?.body === 'string' ? init.body : '' });
    if (key && pendingKeys.has(key)) {
      return json(409, { error: { code: 'idempotencyConflict', message: 'IDEMPOTENCY_CONFLICT' } });
    }
    const next = queue.shift();
    if (!next) throw new Error(`unexpected write ${u.pathname} with empty queue`);
    if (key) pendingKeys.add(key);
    try {
      return await next();
    } finally {
      if (key) pendingKeys.delete(key);
    }
  }
  return json(500, { error: { code: 'serverError', message: `unexpected ${method} ${u.pathname}` } });
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

function mountScreen(): HTMLElement {
  const [branch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown] = createSignal(true);
  const container = document.createElement('div');
  document.body.appendChild(container);
  const dispose = render(() => <UsersScreen branch={branch} period={period} shown={shown} />, container);
  disposers.push(dispose);
  return container;
}

function btn(container: HTMLElement, selector: string): HTMLButtonElement {
  const el = container.querySelector<HTMLButtonElement>(selector);
  expect(el, selector).not.toBeNull();
  return el!;
}

const alertText = (c: HTMLElement) => c.querySelector('.users-content [role="alert"]')?.textContent ?? null;
const statusText = (c: HTMLElement) => c.querySelector('.users-content [role="status"]')?.textContent ?? null;

beforeEach(() => {
  writes = [];
  queue = [];
  pendingKeys = new Set();
  document.body.innerHTML = '';
  setLang('th');
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
});

afterEach(async () => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  await settle();
});

describe('#2413 UsersScreen row writes in flight', () => {
  it('row 1: double click Disable sends one POST /api/users/disable', async () => {
    const first = deferred();
    queue.push(() => first.promise);

    const c = mountScreen();
    await settle();

    btn(c, '.users-btn-disable').click();
    await settle();
    btn(c, '.users-btn-disable').click();
    await settle();

    expect(writes.filter((w) => w.path === '/api/users/disable').length).toBe(1);
    expect(btn(c, '.users-btn-disable').disabled).toBe(true);

    first.resolve(json(200, { userId: 'u-1', status: 'DISABLED', version: 2 }));
    await settle();

    expect(writes.filter((w) => w.path === '/api/users/disable').length).toBe(1);
    expect(alertText(c)).toBeNull();
  });

  it('row 2: click × while a revoke is in flight: no page error; the success text shows', async () => {
    const first = deferred();
    queue.push(() => first.promise);

    const c = mountScreen();
    await settle();

    btn(c, '.users-btn-revoke').click();
    await settle();
    btn(c, '.users-btn-revoke').click();
    await settle();

    expect(alertText(c), 'no page error while the first revoke is in flight').toBeNull();
    expect(btn(c, '.users-btn-revoke').disabled).toBe(true);

    first.resolve(json(200, { userId: 'u-1', status: 'ACTIVE', version: 2 }));
    await settle();

    expect(alertText(c)).toBeNull();
    expect(statusText(c)).toBe(SUCCESS_REVOKED_TH);
    expect(writes.filter((w) => w.path === '/api/users/revoke').length).toBe(1);
  });

  it('row 3: network error on Disable (after a double click), then retry: the retry goes out with the same key', async () => {
    const first = deferred();
    queue.push(
      () => first.promise,
      () => json(200, { userId: 'u-1', status: 'DISABLED', version: 2 }),
    );

    const c = mountScreen();
    await settle();

    btn(c, '.users-btn-disable').click();
    await settle();
    btn(c, '.users-btn-disable').click();
    await settle();

    first.reject(new TypeError('Failed to fetch'));
    await settle();

    expect(btn(c, '.users-btn-disable').disabled, 'button usable again after the network error').toBe(false);
    btn(c, '.users-btn-disable').click();
    await settle();

    const disables = writes.filter((w) => w.path === '/api/users/disable');
    expect(disables.length).toBe(2);
    expect(disables[0]!.key).toBeTruthy();
    expect(disables[1]!.key).toBe(disables[0]!.key);
    expect(disables[1]!.rawBody).toBe(disables[0]!.rawBody);
  });

  it('edge: rowWriting is cleared after a refusal; the next click sends again', async () => {
    const first = deferred();
    queue.push(
      () => first.promise,
      () => json(200, { userId: 'u-1', status: 'DISABLED', version: 2 }),
    );

    const c = mountScreen();
    await settle();

    btn(c, '.users-btn-disable').click();
    await settle();
    btn(c, '.users-btn-disable').click();
    await settle();

    first.resolve(json(409, { error: { code: 'staleVersion', message: 'STALE_VERSION' } }));
    await settle();

    expect(alertText(c), 'the refusal is shown').not.toBeNull();
    expect(btn(c, '.users-btn-disable').disabled).toBe(false);
    expect(btn(c, '.users-btn-revoke').disabled).toBe(false);

    btn(c, '.users-btn-disable').click();
    await settle();

    expect(writes.filter((w) => w.path === '/api/users/disable').length).toBe(2);
    expect(alertText(c)).toBeNull();
  });

  it('edge: the grant dialog keeps its own dialogSubmitting guard (unchanged)', async () => {
    const first = deferred();
    queue.push(() => first.promise);

    const c = mountScreen();
    await settle();

    btn(c, '.users-btn-grant').click();
    await settle();
    const submit = btn(c, '.users-dialog-form button[type="submit"]');
    submit.click();
    await settle();
    expect(submit.disabled).toBe(true);
    submit.click();
    await settle();

    expect(writes.filter((w) => w.path === '/api/users/grant').length).toBe(1);

    first.resolve(json(200, { userId: 'u-1', status: 'ACTIVE', version: 2 }));
    await settle();
    expect(writes.filter((w) => w.path === '/api/users/grant').length).toBe(1);
  });
});
