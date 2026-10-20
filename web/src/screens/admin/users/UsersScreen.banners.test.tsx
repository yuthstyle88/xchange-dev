import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeAdminUserRole, makeAdminUserRow } from '../../../shared/testing/fixtures';
import { UsersScreen } from './UsersScreen';

// #2460: the page success banner is cleared when a later write starts, when a
// row error is shown and on a branch switch; a dialog write's late answer only
// closes / sets the error of the dialog it was sent from.

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

function deferred(): { promise: Promise<Response>; resolve: (r: Response) => void } {
  let resolve!: (r: Response) => void;
  const promise = new Promise<Response>((res) => {
    resolve = res;
  });
  return { promise, resolve };
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

function btn(container: ParentNode, selector: string): HTMLButtonElement {
  const el = container.querySelector<HTMLButtonElement>(selector);
  expect(el, selector).not.toBeNull();
  return el!;
}

function rowButton(c: HTMLElement, userName: string, selector: string): HTMLButtonElement {
  const row = Array.from(c.querySelectorAll('tbody tr')).find((tr) => tr.textContent?.includes(userName));
  expect(row, `row for ${userName}`).toBeTruthy();
  return btn(row!, selector);
}

const alertText = (c: HTMLElement) => c.querySelector('.users-content [role="alert"]')?.textContent ?? null;
const statusText = (c: HTMLElement) => c.querySelector('.users-content [role="status"]')?.textContent ?? null;
const dialog = (c: HTMLElement) => c.querySelector<HTMLElement>('[role="dialog"]');
const dialogAlert = (c: HTMLElement) => dialog(c)?.querySelector('[role="alert"]')?.textContent ?? null;

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

async function closedGrantA_thenOpenGrantB(c: HTMLElement): Promise<void> {
  rowButton(c, 'Alpha Active', '.users-btn-grant').click();
  await settle();
  expect(dialog(c)?.textContent).toContain('Alpha Active');
  btn(c, '.users-dialog-form button[type="submit"]').click();
  await settle();
  expect(writes.filter((w) => w.path === '/api/users/grant').length).toBe(1);

  btn(c, '.users-dialog-form .users-btn-secondary').click();
  await settle();
  expect(dialog(c)).toBeNull();

  rowButton(c, 'Bravo Active', '.users-btn-grant').click();
  await settle();
  expect(dialog(c)?.textContent).toContain('Bravo Active');
}

describe('#2460 UsersScreen banners and dialog answers', () => {
  it('row 1: Revoke ok, then Disable fails: only the Disable error shows', async () => {
    queue.push(
      () => json(200, { userId: 'u-a', status: 'ACTIVE', version: 2 }),
      () => json(409, { error: { code: 'staleVersion', message: 'STALE_VERSION' } }),
    );
    const { c } = mountScreen();
    await settle();

    rowButton(c, 'Alpha Active', '.users-btn-revoke').click();
    await settle();
    expect(statusText(c)).toBe(SUCCESS_REVOKED_TH);

    rowButton(c, 'Alpha Active', '.users-btn-disable').click();
    await settle();

    expect(alertText(c), 'the Disable error is shown').not.toBeNull();
    expect(statusText(c), 'the old success banner is gone').toBeNull();
  });

  it('row 2: Revoke ok, switch branch: no success banner', async () => {
    queue.push(() => json(200, { userId: 'u-a', status: 'ACTIVE', version: 2 }));
    const { c, setBranch } = mountScreen();
    await settle();

    rowButton(c, 'Alpha Active', '.users-btn-revoke').click();
    await settle();
    expect(statusText(c)).toBe(SUCCESS_REVOKED_TH);

    setBranch(B02);
    await settle();

    expect(statusText(c)).toBeNull();
  });

  it("row 3: Grant A, Close while pending, open Grant B, A fails: B's dialog shows no error", async () => {
    const a = deferred();
    queue.push(() => a.promise);
    const { c } = mountScreen();
    await settle();

    await closedGrantA_thenOpenGrantB(c);

    a.resolve(json(409, { error: { code: 'staleVersion', message: 'STALE_VERSION' } }));
    await settle();

    expect(dialog(c)?.textContent, "B's dialog is still open").toContain('Bravo Active');
    expect(dialogAlert(c), "A's error is not shown in B's dialog").toBeNull();
  });

  it("row 4: Grant A, Close while pending, open Grant B, A succeeds: B's dialog stays open", async () => {
    const a = deferred();
    queue.push(() => a.promise);
    const { c } = mountScreen();
    await settle();

    await closedGrantA_thenOpenGrantB(c);

    a.resolve(json(200, { userId: 'u-a', status: 'ACTIVE', version: 2 }));
    await settle();

    expect(dialog(c), "B's dialog stays open").not.toBeNull();
    expect(dialog(c)?.textContent).toContain('Bravo Active');
    expect(dialogAlert(c)).toBeNull();
  });

  it('edge: a success with the same dialog still open closes it as today', async () => {
    queue.push(() => json(200, { userId: 'u-a', status: 'ACTIVE', version: 2 }));
    const { c } = mountScreen();
    await settle();

    rowButton(c, 'Alpha Active', '.users-btn-grant').click();
    await settle();
    btn(c, '.users-dialog-form button[type="submit"]').click();
    await settle();

    expect(dialog(c)).toBeNull();
    expect(statusText(c)).not.toBeNull();
  });

  it('edge: a failure with the same dialog still open shows the error in it as today', async () => {
    queue.push(() => json(409, { error: { code: 'staleVersion', message: 'STALE_VERSION' } }));
    const { c } = mountScreen();
    await settle();

    rowButton(c, 'Alpha Active', '.users-btn-grant').click();
    await settle();
    btn(c, '.users-dialog-form button[type="submit"]').click();
    await settle();

    expect(dialog(c)?.textContent).toContain('Alpha Active');
    expect(dialogAlert(c)).not.toBeNull();
  });
});
