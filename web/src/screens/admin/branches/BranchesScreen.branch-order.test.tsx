import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { BranchesScreen } from './BranchesScreen';
import * as table from './i18n';

// #2247: each admin_branches_profile read is held until the test answers it,
// so the test controls the order the answers arrive in.

interface PendingRead {
  branch: string;
  resolve: (res: Response) => void;
  reject: (err: unknown) => void;
}

interface WriteCall {
  body: Record<string, unknown>;
}

let reads: PendingRead[] = [];
let writes: WriteCall[] = [];
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
    const body = JSON.parse(typeof init?.body === 'string' ? init.body : '{}') as Record<string, unknown>;
    writes.push({ body });
    return json(200, profileFor(String(body.branch), body));
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

function submitProfileForm(container: HTMLElement): void {
  const form = container.querySelector<HTMLFormElement>('form.branches-form');
  expect(form, 'branches-form').not.toBeNull();
  form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
}

function readsFor(branch: string): PendingRead[] {
  return reads.filter((r) => r.branch === branch);
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

describe('#2247 BranchesScreen: profile answers that arrive out of branch order', () => {
  it("row 1: switch A→B, A's answer arrives after B's: form shows B and Save writes B's fields to B", async () => {
    const container = mountScreen();
    await settle();
    expect(readsFor(A).length).toBe(1);

    pickBranch(container, B);
    await settle();
    expect(readsFor(B).length).toBe(1);

    readsFor(B)[0]!.resolve(profileAnswer(B));
    await settle();
    readsFor(A)[0]!.resolve(profileAnswer(A));
    await settle();

    expect(heading(container)).toContain('Asok Branch');
    expect(field(container, 'code').value).toBe('ASOK');
    expect(field(container, 'address').value).toBe('2 Asok Rd');
    expect(field(container, 'timezone').value).toBe('Asia/Tokyo');
    expect(field(container, 'bookingEnabled').checked).toBe(false);

    submitProfileForm(container);
    await settle();

    expect(writes.length).toBe(1);
    const body = writes[0]!.body;
    expect(body.branch).toBe(B);
    expect(body.code).toBe('ASOK');
    expect(body.address).toBe('2 Asok Rd');
    expect(body.timezone).toBe('Asia/Tokyo');
    expect(body.bookingEnabled).toBe(false);
    expect(body.openingHours).toEqual(profileFor(B).openingHours);
  });

  it("row 2: switch A→B, A's read fails after B's answer: B's profile stays and A's error is not shown", async () => {
    const container = mountScreen();
    await settle();

    pickBranch(container, B);
    await settle();

    readsFor(B)[0]!.resolve(profileAnswer(B));
    await settle();
    readsFor(A)[0]!.resolve(
      json(500, { error: { code: 'serverError', message: 'Internal Server Error' } }),
    );
    await settle();

    expect(alertText(container)).toBeNull();
    expect(heading(container)).toContain('Asok Branch');
    expect(field(container, 'code').value).toBe('ASOK');
    expect(field(container, 'timezone').value).toBe('Asia/Tokyo');
  });

  it('row 3 (kept): single branch load shows the profile', async () => {
    const container = mountScreen();
    await settle();

    readsFor(A)[0]!.resolve(profileAnswer(A));
    await settle();

    expect(heading(container)).toContain('Silom Branch');
    expect(field(container, 'code').value).toBe('SILOM');
    expect(field(container, 'address').value).toBe('1 Silom Rd');
    expect(field(container, 'timezone').value).toBe('Asia/Bangkok');
    expect(field(container, 'bookingEnabled').checked).toBe(true);
    expect(alertText(container)).toBeNull();
  });

  it('row 4 (kept): empty timezone on save shows timezoneRequired and writes nothing', async () => {
    const container = mountScreen();
    await settle();

    readsFor(A)[0]!.resolve(profileAnswer(A, { timezone: '' }));
    await settle();

    submitProfileForm(container);
    await settle();

    expect(writes.length).toBe(0);
    expect(alertText(container)).toBe(table.en.timezoneRequired);
  });

  it("edge: switch A→B→A before either answers: A's newest answer wins", async () => {
    const container = mountScreen();
    await settle();

    pickBranch(container, B);
    await settle();
    pickBranch(container, A);
    await settle();

    const aReads = readsFor(A);
    expect(aReads.length).toBe(2);
    expect(readsFor(B).length).toBe(1);

    // Newest A answer first, then B, then the older A answer.
    aReads[1]!.resolve(profileAnswer(A, { code: 'SILOM-NEW', address: 'New Silom Rd' }));
    await settle();
    readsFor(B)[0]!.resolve(profileAnswer(B));
    await settle();
    aReads[0]!.resolve(profileAnswer(A, { code: 'SILOM-OLD', address: 'Old Silom Rd' }));
    await settle();

    expect(heading(container)).toContain('Silom Branch');
    expect(field(container, 'code').value).toBe('SILOM-NEW');
    expect(field(container, 'address').value).toBe('New Silom Rd');
    expect(field(container, 'timezone').value).toBe('Asia/Bangkok');
  });
});
