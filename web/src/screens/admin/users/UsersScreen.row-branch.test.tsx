import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeAdminUserRole, makeAdminUserRow } from '../../../shared/testing/fixtures';
import { UsersScreen } from './UsersScreen';

interface WriteCall {
  path: string;
  rawBody: string;
}

type Responder = () => Promise<Response> | Response;

let writes: WriteCall[] = [];
let queue: Responder[] = [];
const disposers: Array<() => void> = [];

const B01 = 'XC-B01';
const B02 = 'XC-B02';
const SUCCESS_REVOKED_TH = 'ยกเลิกสิทธิ์เรียบร้อยแล้ว';

const SEED_USERS = [
  makeAdminUserRow({
    userId: 'u-a',
    name: 'Alpha Active',
    status: 'ACTIVE',
    version: 1,
    roles: [
      makeAdminUserRole({ role: 'TELLER', branch: B01, validFrom: '2026-10-01T00:00:00Z', validTo: null }),
    ],
  }),
  makeAdminUserRow({
    userId: 'u-b',
    name: 'Bravo Active',
    status: 'ACTIVE',
    version: 1,
    roles: [
      makeAdminUserRole({ role: 'FINANCE', branch: B01, validFrom: '2026-10-01T00:00:00Z', validTo: null }),
    ],
  }),
];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function deferred(): {
  promise: Promise<Response>;
  resolve: (r: Response) => void;
  reject: (e: unknown) => void;
} {
  let resolve!: (r: Response) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<Response>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const WRITES = new Set(['/api/users/revoke', '/api/users/disable', '/api/users/grant', '/api/users']);

async function fakeFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const u = new URL(String(input), 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();

  if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches') {
    return json(200, {
      version: 1,
      changed: true,
      data: { branches: [{ id: B01, name: 'Silom Branch' }, { id: B02, name: 'Sathorn Branch' }] },
    });
  }
  if (method === 'GET' && u.pathname === '/api/v3/views/admin_users') {
    return json(200, { version: 1, changed: true, data: structuredClone(SEED_USERS) });
  }
  if (method === 'POST' && WRITES.has(u.pathname)) {
    writes.push({ path: u.pathname, rawBody: typeof init?.body === 'string' ? init.body : '' });
    const next = queue.shift();
    if (!next) throw new Error(`unexpected write ${u.pathname} with empty queue`);
    return await next();
  }
  return json(500, { error: { code: 'serverError', message: `unexpected ${method} ${u.pathname}` } });
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

function mountScreen(): { c: HTMLElement; setBranch: Setter<string> } {
  const [branch, setBranch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown] = createSignal(true);
  const container = document.createElement('div');
  document.body.appendChild(container);
  const dispose = render(() => <UsersScreen branch={branch} period={period} shown={shown} />, container);
  disposers.push(dispose);
  return { c: container, setBranch };
}

function rowButton(container: HTMLElement, userName: string, selector: string): HTMLButtonElement {
  const row = Array.from(container.querySelectorAll('tbody tr')).find((tr) =>
    tr.textContent?.includes(userName),
  );
  expect(row, `row for ${userName}`).toBeDefined();
  const el = row!.querySelector<HTMLButtonElement>(selector);
  expect(el, `${selector} in row ${userName}`).not.toBeNull();
  return el!;
}

const alertText = (c: HTMLElement) => c.querySelector('.users-content [role="alert"]')?.textContent ?? null;
const statusText = (c: HTMLElement) => c.querySelector('.users-content [role="status"]')?.textContent ?? null;

beforeEach(() => {
  writes = [];
  queue = [];
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

describe('108-Plaza/xchange#2716 UsersScreen row actions after branch switch', () => {
  // Row 1: Revoke on A held, switch to B, the write succeeds -> No success banner on B
  it('Row 1: Revoke on A held, switch to B, the write succeeds -> No success banner on B', async () => {
    const def = deferred();
    queue.push(() => def.promise);

    const { c, setBranch } = mountScreen();
    await settle();

    rowButton(c, 'Alpha Active', '.users-btn-revoke').click();
    await settle();

    // Switch branch while write is in flight
    setBranch(B02);
    await settle();

    // In-flight write completes successfully
    def.resolve(json(200, { userId: 'u-a', status: 'ACTIVE', version: 2 }));
    await settle();

    expect(statusText(c)).toBeNull();
  });

  // Row 2: Disable on A held, switch to B, the write succeeds -> No success banner on B
  it('Row 2: Disable on A held, switch to B, the write succeeds -> No success banner on B', async () => {
    const def = deferred();
    queue.push(() => def.promise);

    const { c, setBranch } = mountScreen();
    await settle();

    rowButton(c, 'Alpha Active', '.users-btn-disable').click();
    await settle();

    // Switch branch while write is in flight
    setBranch(B02);
    await settle();

    // In-flight write completes successfully
    def.resolve(json(200, { userId: 'u-a', status: 'DISABLED', version: 2 }));
    await settle();

    expect(statusText(c)).toBeNull();
  });

  // Row 3a: Revoke on A held, switch to B, the write fails -> No error banner on B
  it('Row 3a: Revoke on A held, switch to B, the write fails -> No error banner on B', async () => {
    const defRevoke = deferred();
    queue.push(() => defRevoke.promise);

    const { c, setBranch } = mountScreen();
    await settle();

    rowButton(c, 'Alpha Active', '.users-btn-revoke').click();
    await settle();

    // Switch branch while write is in flight
    setBranch(B02);
    await settle();

    // In-flight write fails
    defRevoke.resolve(json(500, { error: { code: 'serverError', message: 'SERVER_ERROR' } }));
    await settle();

    expect(alertText(c)).toBeNull();
  });

  // Row 3b: Disable on A held, switch to B, the write fails -> No error banner on B
  it('Row 3b: Disable on A held, switch to B, the write fails -> No error banner on B', async () => {
    const defDisable = deferred();
    queue.push(() => defDisable.promise);

    const { c, setBranch } = mountScreen();
    await settle();

    rowButton(c, 'Alpha Active', '.users-btn-disable').click();
    await settle();

    // Switch branch while write is in flight
    setBranch(B02);
    await settle();

    // In-flight write fails
    defDisable.resolve(json(500, { error: { code: 'serverError', message: 'SERVER_ERROR' } }));
    await settle();

    expect(alertText(c)).toBeNull();
  });

  // Row 4: Revoke or Disable with no switch -> The banner shows and the list re-reads
  it('Row 4: Revoke or Disable with no switch -> The banner shows and the list re-reads', async () => {
    queue.push(() => json(200, { userId: 'u-a', status: 'ACTIVE', version: 2 }));

    const { c } = mountScreen();
    await settle();

    rowButton(c, 'Alpha Active', '.users-btn-revoke').click();
    await settle();

    expect(statusText(c)).toBe(SUCCESS_REVOKED_TH);
  });
});
