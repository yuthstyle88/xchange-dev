// Amended by W8 (claude/177aa7ef): userCreate no longer takes a typed password (F9 #3155, owner Q5 A); the server
// answers temporaryPassword once. xchange#564, the BE controller's F9 contract.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { AdminUserRow } from '../../../shared/api/generated/admin_users';
import { UsersScreen } from './UsersScreen';

// #2472: each dialog opening is its own generation. A write's late answer only
// closes / sets the error of the same opening it was sent from, not a dialog of
// the same kind and user that was closed and reopened meanwhile.

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

const SEED_USERS: AdminUserRow[] = [
  {
    userId: 'u-a',
    name: 'Alpha Active',
    status: 'ACTIVE',
    version: 1,
    roles: [
      { role: 'TELLER', branch: B01, validFrom: '2026-10-01T00:00:00Z', validTo: null },
    ],
  },
  {
    userId: 'u-b',
    name: 'Bravo Active',
    status: 'ACTIVE',
    version: 1,
    roles: [
      { role: 'FINANCE', branch: B01, validFrom: '2026-10-01T00:00:00Z', validTo: null },
    ],
  },
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

function type(c: HTMLElement, selector: string, value: string): void {
  const input = c.querySelector<HTMLInputElement>(selector);
  expect(input, selector).not.toBeNull();
  input!.value = value;
  input!.dispatchEvent(new Event('input', { bubbles: true }));
}

function submitDialog(c: HTMLElement): void {
  const form = c.querySelector<HTMLFormElement>('.users-dialog-form');
  expect(form, 'dialog form').not.toBeNull();
  form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
}

const dialog = (c: HTMLElement) => c.querySelector<HTMLElement>('[role="dialog"]');
const dialogAlert = (c: HTMLElement) => dialog(c)?.querySelector('[role="alert"]')?.textContent ?? null;
const closeDialog = (c: HTMLElement) => btn(c, '.users-dialog-form .users-btn-secondary').click();
const createButton = (c: HTMLElement) => btn(c, '.users-actions .users-btn-primary');

// Create, Close while pending, reopen Create and type into the new form.
async function closedCreate_thenReopenAndType(c: HTMLElement): Promise<void> {
  createButton(c).click();
  await settle();
  type(c, 'input[name="name"]', 'First Person');
  type(c, 'input[name="phone"]', '0811111111');
  submitDialog(c);
  await settle();
  expect(writes.filter((w) => w.path === '/api/users').length).toBe(1);

  closeDialog(c);
  await settle();
  expect(dialog(c)).toBeNull();

  createButton(c).click();
  await settle();
  expect(dialog(c), 'the reopened Create form is open').not.toBeNull();
  type(c, 'input[name="name"]', 'Second Person');
  type(c, 'input[name="phone"]', '0822222222');
}

// Grant A, Close while pending, reopen Grant A.
async function closedGrantA_thenReopenGrantA(c: HTMLElement): Promise<void> {
  rowButton(c, 'Alpha Active', '.users-btn-grant').click();
  await settle();
  expect(dialog(c)?.textContent).toContain('Alpha Active');
  submitDialog(c);
  await settle();
  expect(writes.filter((w) => w.path === '/api/users/grant').length).toBe(1);

  closeDialog(c);
  await settle();
  expect(dialog(c)).toBeNull();

  rowButton(c, 'Alpha Active', '.users-btn-grant').click();
  await settle();
  expect(dialog(c)?.textContent).toContain('Alpha Active');
}

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

describe('#2472 UsersScreen: a closed dialog\'s late answer leaves a reopened dialog alone', () => {
  it('row 1: Create, Close, reopen Create, type, first create succeeds: the reopened form stays open with its input', async () => {
    const first = deferred();
    queue.push(() => first.promise);
    const c = mountScreen();
    await settle();

    await closedCreate_thenReopenAndType(c);

    first.resolve(json(200, { userId: 'u-new', status: 'ACTIVE', version: 1 }));
    await settle();

    expect(dialog(c), 'the reopened Create form stays open').not.toBeNull();
    expect(c.querySelector<HTMLInputElement>('input[name="name"]')?.value).toBe('Second Person');
    expect(c.querySelector<HTMLInputElement>('input[name="phone"]')?.value).toBe('0822222222');
    expect(dialogAlert(c)).toBeNull();
  });

  it('row 2: Create, Close, reopen Create, type, first create fails: no error in the reopened form', async () => {
    const first = deferred();
    queue.push(() => first.promise);
    const c = mountScreen();
    await settle();

    await closedCreate_thenReopenAndType(c);

    first.resolve(json(409, { error: { code: 'staleVersion', message: 'STALE_VERSION' } }));
    await settle();

    expect(dialog(c), 'the reopened Create form stays open').not.toBeNull();
    expect(dialogAlert(c), 'the first create error is not shown in the reopened form').toBeNull();
    expect(c.querySelector<HTMLInputElement>('input[name="name"]')?.value).toBe('Second Person');
  });

  it('row 3a: Grant A, Close, reopen Grant A, first grant succeeds: the reopened dialog stays open', async () => {
    const first = deferred();
    queue.push(() => first.promise);
    const c = mountScreen();
    await settle();

    await closedGrantA_thenReopenGrantA(c);

    first.resolve(json(200, { userId: 'u-a', status: 'ACTIVE', version: 2 }));
    await settle();

    expect(dialog(c), 'the reopened Grant A dialog stays open').not.toBeNull();
    expect(dialog(c)?.textContent).toContain('Alpha Active');
    expect(dialogAlert(c)).toBeNull();
  });

  it('row 3b: Grant A, Close, reopen Grant A, first grant fails: no error in the reopened dialog', async () => {
    const first = deferred();
    queue.push(() => first.promise);
    const c = mountScreen();
    await settle();

    await closedGrantA_thenReopenGrantA(c);

    first.resolve(json(409, { error: { code: 'staleVersion', message: 'STALE_VERSION' } }));
    await settle();

    expect(dialog(c)?.textContent, 'the reopened Grant A dialog stays open').toContain('Alpha Active');
    expect(dialogAlert(c), 'the first grant error is not shown in the reopened dialog').toBeNull();
  });
});
