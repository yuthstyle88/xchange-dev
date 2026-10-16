import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { BranchesScreen } from './BranchesScreen';

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

const PROFILE_DATA = {
  branch: B01,
  name: 'Silom Branch',
  code: 'SILOM',
  address: '1 Silom Rd',
  timezone: 'Asia/Bangkok',
  openingHours: {
    mon: [{ open: '09:00', close: '17:00' }],
    tue: [{ open: '09:00', close: '17:00' }],
    wed: [{ open: '09:00', close: '17:00' }],
    thu: [{ open: '09:00', close: '17:00' }],
    fri: [{ open: '09:00', close: '17:00' }],
    sat: [],
    sun: [],
  },
  bookingEnabled: true,
};

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

  if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches_profile') {
    return json(200, {
      version: 1,
      changed: true,
      data: structuredClone(PROFILE_DATA),
    });
  }

  if (method === 'PUT' && u.pathname === '/api/branches/profile') {
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
    () => <BranchesScreen branch={branch} period={period} shown={shown} />,
    container,
  );
  disposers.push(dispose);
  return container;
}

function setCode(container: HTMLElement, value: string): void {
  const input = container.querySelector<HTMLInputElement>('input[name="code"]');
  expect(input, 'code input').not.toBeNull();
  input!.value = value;
  input!.dispatchEvent(new Event('input', { bubbles: true }));
  input!.dispatchEvent(new Event('change', { bubbles: true }));
}

function submitProfileForm(container: HTMLElement): void {
  const form = container.querySelector<HTMLFormElement>('form.branches-form');
  expect(form, 'branches-form').not.toBeNull();
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

describe('KW-2 (#2022): BranchesScreen main write idempotency key behaviour', () => {
  it('1. submit -> network TypeError -> submit again with the same input: 2 requests with the same key and a byte-equal body', async () => {
    writeQueue.push(
      () => {
        throw new TypeError('Failed to fetch');
      },
      () => json(200, structuredClone(PROFILE_DATA)),
    );

    const container = mountScreen();
    await settle();

    submitProfileForm(container);
    await settle();

    submitProfileForm(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });

  it('2. submit -> 500 -> submit again: same key', async () => {
    writeQueue.push(
      () => json(500, { error: { code: 'serverError', message: 'Internal Server Error' } }),
      () => json(200, structuredClone(PROFILE_DATA)),
    );

    const container = mountScreen();
    await settle();

    submitProfileForm(container);
    await settle();

    submitProfileForm(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });

  it('3. submit -> 4xx refusal -> submit again: a new key', async () => {
    writeQueue.push(
      () => json(409, { error: { code: 'conflict', message: 'Code conflict' } }),
      () => json(200, structuredClone(PROFILE_DATA)),
    );

    const container = mountScreen();
    await settle();

    submitProfileForm(container);
    await settle();

    submitProfileForm(container);
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
      () => json(200, { ...structuredClone(PROFILE_DATA), code: 'SILOM-2' }),
    );

    const container = mountScreen();
    await settle();

    submitProfileForm(container);
    await settle();

    setCode(container, 'SILOM-2');
    submitProfileForm(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).not.toBe(writes[0]!.rawBody);
  });

  it('5. submit -> 2xx -> submit the same input again: a new key', async () => {
    writeQueue.push(
      () => json(200, structuredClone(PROFILE_DATA)),
      () => json(200, structuredClone(PROFILE_DATA)),
    );

    const container = mountScreen();
    await settle();

    submitProfileForm(container);
    await settle();

    submitProfileForm(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);
  });
});
