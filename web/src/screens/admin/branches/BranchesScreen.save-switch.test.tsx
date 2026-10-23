import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { BranchesScreen } from './BranchesScreen';
import * as table from './i18n';

// #2463: each admin_branches_profile read and each PUT /api/branches/profile is held
// until the test answers it, so the test can switch the branch picker while a save
// is in flight and control the order the answers arrive in.

interface Pending {
  branch: string;
  resolve: (res: Response) => void;
  reject: (err: unknown) => void;
}

interface PendingWrite extends Pending {
  key: string | null;
  rawBody: string;
  body: Record<string, unknown>;
}

let reads: Pending[] = [];
let writes: PendingWrite[] = [];
const disposers: Array<() => void> = [];

const A = 'XC-B01';
const B = 'XC-B02';

function profileFor(branch: string, overrides: Record<string, unknown> = {}): Record<string, unknown> {
  const isA = branch === A;
  return {
    branch,
    name: isA ? 'Silom Branch' : 'Asok Branch',
    code: isA ? 'SILOM' : 'ASOK',
    address: isA ? '1 Silom Rd' : '2 Asok Rd',
    timezone: isA ? 'Asia/Bangkok' : 'Asia/Tokyo',
    openingHours: {
      mon: [{ open: isA ? '09:00' : '10:00', close: isA ? '17:00' : '20:00' }],
      tue: [],
      wed: [],
      thu: [],
      fri: [],
      sat: isA ? [] : [{ open: '11:00', close: '15:00' }],
      sun: [],
    },
    bookingEnabled: isA,
    ...overrides,
  };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function profileAnswer(branch: string, overrides: Record<string, unknown> = {}): Response {
  return json(200, { version: 1, changed: true, data: profileFor(branch, overrides) });
}

function serverError(): Response {
  return json(500, { error: { code: 'serverError', message: 'Internal Server Error' } });
}

async function fakeFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const u = new URL(String(input), 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();

  if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches') {
    return json(200, {
      version: 1,
      changed: true,
      data: {
        branches: [
          { id: A, name: 'Silom Branch' },
          { id: B, name: 'Asok Branch' },
        ],
      },
    });
  }

  if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches_profile') {
    const branch = u.searchParams.get('branch') ?? '';
    return new Promise<Response>((resolve, reject) => {
      reads.push({ branch, resolve, reject });
    });
  }

  if (method === 'PUT' && u.pathname === '/api/branches/profile') {
    const rawBody = typeof init?.body === 'string' ? init.body : '{}';
    const body = JSON.parse(rawBody) as Record<string, unknown>;
    const key = new Headers(init?.headers).get('idempotency-key');
    return new Promise<Response>((resolve, reject) => {
      writes.push({ branch: String(body.branch), key, rawBody, body, resolve, reject });
    });
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
  const [branch] = createSignal(A);
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

function pickBranch(container: HTMLElement, id: string): void {
  const select = container.querySelector<HTMLSelectElement>('#branch-select');
  expect(select, 'branch select').not.toBeNull();
  select!.value = id;
  select!.dispatchEvent(new Event('change', { bubbles: true }));
}

function field(container: HTMLElement, name: string): HTMLInputElement {
  const input = container.querySelector<HTMLInputElement>(`[name="${name}"]`);
  expect(input, `${name} input`).not.toBeNull();
  return input!;
}

function heading(container: HTMLElement): string {
  return container.querySelector('.branches-heading')?.textContent ?? '';
}

function alertText(container: HTMLElement): string | null {
  return container.querySelector('[role="alert"]')?.textContent ?? null;
}

function statusText(container: HTMLElement): string | null {
  return container.querySelector('[role="status"]')?.textContent ?? null;
}

function submitProfileForm(container: HTMLElement): void {
  const form = container.querySelector<HTMLFormElement>('form.branches-form');
  expect(form, 'branches-form').not.toBeNull();
  form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
}

function readsFor(branch: string): Pending[] {
  return reads.filter((r) => r.branch === branch);
}

/** Mount on A, let A's profile load, press Save on A (the write stays in flight). */
async function saveOnAInFlight(): Promise<HTMLElement> {
  const container = mountScreen();
  await settle();
  readsFor(A)[0]!.resolve(profileAnswer(A));
  await settle();
  expect(heading(container)).toContain('Silom Branch');

  submitProfileForm(container);
  await settle();
  expect(writes.length).toBe(1);
  expect(writes[0]!.branch).toBe(A);
  return container;
}

function expectBForm(container: HTMLElement): void {
  expect(heading(container)).toContain('Asok Branch');
  expect(field(container, 'code').value).toBe('ASOK');
  expect(field(container, 'address').value).toBe('2 Asok Rd');
  expect(field(container, 'timezone').value).toBe('Asia/Tokyo');
  expect(field(container, 'bookingEnabled').checked).toBe(false);
}

beforeEach(() => {
  reads = [];
  writes = [];
  document.body.innerHTML = '';
  setLang('en');
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
});

afterEach(async () => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  await settle();
});

describe('#2463 BranchesScreen: switching the branch picker while a profile save is in flight', () => {
  it("row 1: save A, switch to B, A's save answers ok after B's read: B's form and no Saved banner on B", async () => {
    const container = await saveOnAInFlight();

    pickBranch(container, B);
    await settle();
    readsFor(B)[0]!.resolve(profileAnswer(B));
    await settle();

    writes[0]!.resolve(json(200, profileFor(A, writes[0]!.body)));
    await settle();

    expectBForm(container);
    expect(statusText(container)).toBeNull();
    expect(alertText(container)).toBeNull();
  });

  it("row 1b: save A, switch to B, A's save answers ok before B's read: B's form and no Saved banner on B", async () => {
    const container = await saveOnAInFlight();

    pickBranch(container, B);
    await settle();

    writes[0]!.resolve(json(200, profileFor(A, writes[0]!.body)));
    await settle();
    expect(heading(container)).not.toContain('Silom Branch');

    readsFor(B)[0]!.resolve(profileAnswer(B));
    await settle();

    expectBForm(container);
    expect(statusText(container)).toBeNull();
    expect(alertText(container)).toBeNull();
  });

  it("row 2: save A, switch to B, A's save fails: no A error on B", async () => {
    const container = await saveOnAInFlight();

    pickBranch(container, B);
    await settle();
    readsFor(B)[0]!.resolve(profileAnswer(B));
    await settle();

    writes[0]!.resolve(serverError());
    await settle();

    expect(alertText(container)).toBeNull();
    expect(statusText(container)).toBeNull();
    expectBForm(container);
  });

  it("row 3: save A, switch to B, B's read fails, then A's save answers: form does not show A's fields under B", async () => {
    const container = await saveOnAInFlight();

    pickBranch(container, B);
    await settle();
    readsFor(B)[0]!.resolve(serverError());
    await settle();
    const bReadError = alertText(container);
    expect(bReadError).not.toBeNull();

    writes[0]!.resolve(json(200, profileFor(A, writes[0]!.body)));
    await settle();

    expect(heading(container)).not.toContain('Silom Branch');
    expect(container.querySelector('form.branches-form')).toBeNull();
    for (const name of ['code', 'address', 'timezone']) {
      const input = container.querySelector<HTMLInputElement>(`input[name="${name}"]`);
      expect(input?.value ?? null, `${name} must not carry A's value under B`).not.toBe(
        String(profileFor(A)[name]),
      );
    }
    expect(statusText(container)).toBeNull();
    // B's own read error stays; A's save does not replace it.
    expect(alertText(container)).toBe(bReadError);
  });

  it("row 4: save A, switch to B (B loads), back to A, A's re-read fails, then A's save answers: no B fields under A and no Saved", async () => {
    const container = await saveOnAInFlight();

    pickBranch(container, B);
    await settle();
    readsFor(B)[0]!.resolve(profileAnswer(B));
    await settle();
    expectBForm(container);

    pickBranch(container, A);
    await settle();
    readsFor(A)[1]!.resolve(serverError());
    await settle();
    const aReadError = alertText(container);
    expect(aReadError).not.toBeNull();

    writes[0]!.resolve(json(200, profileFor(A, writes[0]!.body)));
    await settle();

    for (const name of ['code', 'address', 'timezone']) {
      const input = container.querySelector<HTMLInputElement>(`input[name="${name}"]`);
      expect(input?.value ?? null, `${name} must not carry B's value under A`).not.toBe(
        String(profileFor(B)[name]),
      );
    }
    expect(statusText(container)).not.toBe(table.en.saved);
    expect(statusText(container)).toBeNull();
    // A's own re-read error stays; the older save answer does not replace it.
    expect(alertText(container)).toBe(aReadError);
  });

  it('edge (kept guard): codeConflict on the same branch still shows its message', async () => {
    const container = await saveOnAInFlight();

    writes[0]!.resolve(json(409, { error: { code: 'conflict', message: 'Code conflict' } }));
    await settle();

    expect(alertText(container)).toBe(table.en.codeConflict);
    expect(statusText(container)).toBeNull();
    expect(heading(container)).toContain('Silom Branch');
  });

  it('edge (kept guard): A save 500 while on B, back to A, Save the same input: same idempotency key and byte-equal body', async () => {
    const container = await saveOnAInFlight();

    pickBranch(container, B);
    await settle();
    readsFor(B)[0]!.resolve(profileAnswer(B));
    await settle();
    writes[0]!.resolve(serverError());
    await settle();

    pickBranch(container, A);
    await settle();
    readsFor(A)[1]!.resolve(profileAnswer(A));
    await settle();

    submitProfileForm(container);
    await settle();

    expect(writes.length).toBe(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.rawBody).toBe(writes[0]!.rawBody);

    writes[1]!.resolve(json(200, profileFor(A, writes[1]!.body)));
    await settle();
    expect(statusText(container)).toBe(table.en.saved);
  });
});
