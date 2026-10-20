import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeAdminUserRole, makeAdminUserRow } from '../../../shared/testing/fixtures';
import { UsersScreen } from './UsersScreen';

interface WriteCall {
  url: string;
  method: string;
  key: string | null;
  rawBody: string;
}

type WriteResponder = () => Promise<Response> | Response;

let writes: WriteCall[] = [];
let writeQueue: WriteResponder[] = [];
const disposers: Array<() => void> = [];

const B01 = 'XC-B01';

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

async function fakeFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = String(input);
  const u = new URL(url, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();

  if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches') {
    return json(200, {
      version: 1,
      changed: true,
      data: { branches: [{ id: B01, name: 'Silom Branch' }] },
    });
  }

  if (method === 'GET' && u.pathname === '/api/v3/views/admin_users') {
    return json(200, {
      version: 1,
      changed: true,
      data: structuredClone(SEED_USERS),
    });
  }

  if (method === 'POST' && u.pathname === '/api/users/grant') {
    const headers = new Headers(init?.headers);
    writes.push({
      url: u.pathname,
      method,
      key: headers.get('idempotency-key'),
      rawBody: typeof init?.body === 'string' ? init.body : '',
    });
    const next = writeQueue.shift();
    if (!next) {
      throw new Error('unexpected write with empty writeQueue');
    }
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

function mountScreen(): HTMLElement {
  const [branch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown] = createSignal(true);
  const container = document.createElement('div');
  document.body.appendChild(container);
  const dispose = render(
    () => <UsersScreen branch={branch} period={period} shown={shown} />,
    container,
  );
  disposers.push(dispose);
  return container;
}

function openGrantDialog(container: HTMLElement): void {
  const btn = container.querySelector<HTMLButtonElement>('.users-btn-grant');
  expect(btn, 'grant button').not.toBeNull();
  btn!.click();
}

function selectGrantRole(container: HTMLElement, role: string): void {
  const select = container.querySelector<HTMLSelectElement>('select[name="grantRole"]');
  expect(select, 'grantRole select').not.toBeNull();
  select!.value = role;
  select!.dispatchEvent(new Event('input', { bubbles: true }));
  select!.dispatchEvent(new Event('change', { bubbles: true }));
}

function submitGrantDialog(container: HTMLElement): void {
  const form = container.querySelector<HTMLFormElement>('.users-dialog-form');
  expect(form, 'grant dialog form').not.toBeNull();
  form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
}

beforeEach(() => {
  writes = [];
  writeQueue = [];
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

describe('KW-2 (#2022): UsersScreen main write idempotency key behaviour', () => {
  it('1. submit -> network TypeError -> submit again with the same input: 2 requests with the same key and a byte-equal body', async () => {
    writeQueue.push(
      () => {
        throw new TypeError('Failed to fetch');
      },
      () => json(200, { userId: 'u-1', status: 'ACTIVE', version: 2 }),
    );

    const container = mountScreen();
    await settle();

    openGrantDialog(container);
    await settle();
    selectGrantRole(container, 'FINANCE');
    submitGrantDialog(container);
    await settle();

    submitGrantDialog(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });

  it('2. submit -> 500 -> submit again: same key', async () => {
    writeQueue.push(
      () => json(500, { error: { code: 'serverError', message: 'Internal Server Error' } }),
      () => json(200, { userId: 'u-1', status: 'ACTIVE', version: 2 }),
    );

    const container = mountScreen();
    await settle();

    openGrantDialog(container);
    await settle();
    selectGrantRole(container, 'FINANCE');
    submitGrantDialog(container);
    await settle();

    submitGrantDialog(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });

  it('3. submit -> 4xx refusal -> submit again: a new key', async () => {
    writeQueue.push(
      () => json(409, { error: { code: 'roleAlreadyGranted', message: 'Role already granted' } }),
      () => json(200, { userId: 'u-1', status: 'ACTIVE', version: 2 }),
    );

    const container = mountScreen();
    await settle();

    openGrantDialog(container);
    await settle();
    selectGrantRole(container, 'FINANCE');
    submitGrantDialog(container);
    await settle();

    submitGrantDialog(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('4. submit -> network error -> edit the input -> submit: a new key', async () => {
    writeQueue.push(
      () => {
        throw new TypeError('Failed to fetch');
      },
      () => json(200, { userId: 'u-1', status: 'ACTIVE', version: 2 }),
    );

    const container = mountScreen();
    await settle();

    openGrantDialog(container);
    await settle();
    selectGrantRole(container, 'FINANCE');
    submitGrantDialog(container);
    await settle();

    selectGrantRole(container, 'ADMIN');
    submitGrantDialog(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).not.toBe(writes[0]!.rawBody);
  });

  it('5. submit -> 2xx -> submit the same input again: a new key', async () => {
    writeQueue.push(
      () => json(200, { userId: 'u-1', status: 'ACTIVE', version: 2 }),
      () => json(200, { userId: 'u-1', status: 'ACTIVE', version: 3 }),
    );

    const container = mountScreen();
    await settle();

    openGrantDialog(container);
    await settle();
    selectGrantRole(container, 'FINANCE');
    submitGrantDialog(container);
    await settle();

    openGrantDialog(container);
    await settle();
    selectGrantRole(container, 'FINANCE');
    submitGrantDialog(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });
});
