import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { BranchesScreen } from './BranchesScreen';

const disposers: Array<() => void> = [];

const A = 'XC-B01';
const X = 'XC-B02';

function profileFor(branch: string): Record<string, unknown> {
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
      sat: [],
      sun: [],
    },
    bookingEnabled: isA,
  };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
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
          { id: X, name: 'Asok Branch' },
        ],
      },
    });
  }

  if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches_profile') {
    const branch = u.searchParams.get('branch') ?? A;
    return json(200, {
      version: 1,
      changed: true,
      data: profileFor(branch),
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

function mountScreen(initialBranch = A) {
  const [branch] = createSignal(initialBranch);
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown, setShown] = createSignal(true);
  const container = document.createElement('div');
  document.body.appendChild(container);
  const dispose = render(
    () => <BranchesScreen branch={branch} period={period} shown={shown} />,
    container,
  );
  disposers.push(dispose);
  return { container, setShown };
}

function branchSelect(container: HTMLElement): HTMLSelectElement {
  const select = container.querySelector<HTMLSelectElement>('#branch-select');
  expect(select, '#branch-select').not.toBeNull();
  return select!;
}

function userPickBranch(container: HTMLElement, id: string): void {
  const select = branchSelect(container);
  if (select.value === id) return;
  select.value = id;
  select.dispatchEvent(new Event('change', { bubbles: true }));
}

function clickRefresh(container: HTMLElement): void {
  const btn = container.querySelector<HTMLButtonElement>('.branches-toolbar button.branches-btn');
  expect(btn, 'Refresh button').not.toBeNull();
  btn!.click();
}

function heading(container: HTMLElement): string {
  return container.querySelector('.branches-heading')?.textContent ?? '';
}

function codeValue(container: HTMLElement): string {
  const input = container.querySelector<HTMLInputElement>('input[name="code"]');
  expect(input, 'code input').not.toBeNull();
  return input!.value;
}

beforeEach(() => {
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

describe('#2549 BranchesScreen: branch picker stays in sync after Refresh or re-opening the menu', () => {
  it('row 1 & edge case: first load selects the shell branch, and after picking X (second) and pressing Refresh, #branch-select value is X', async () => {
    const { container } = mountScreen(A);
    await settle();

    // Edge case (§9): first load selects the shell's branch as today
    expect(branchSelect(container).value).toBe(A);
    expect(heading(container)).toContain('Silom Branch');

    userPickBranch(container, X);
    await settle();
    expect(branchSelect(container).value).toBe(X);
    expect(heading(container)).toContain('Asok Branch');
    expect(codeValue(container)).toBe('ASOK');

    clickRefresh(container);
    await settle();

    expect(branchSelect(container).value).toBe(X);
    expect(heading(container)).toContain('Asok Branch');
    expect(codeValue(container)).toBe('ASOK');
  });

  it('row 2: Pick X, hide and re-show the menu -> #branch-select value is X', async () => {
    const { container, setShown } = mountScreen(A);
    await settle();

    userPickBranch(container, X);
    await settle();
    expect(branchSelect(container).value).toBe(X);

    setShown(false);
    await settle();
    setShown(true);
    await settle();

    expect(branchSelect(container).value).toBe(X);
    expect(heading(container)).toContain('Asok Branch');
    expect(codeValue(container)).toBe('ASOK');
  });

  it("row 3: Pick X, press Refresh, then pick A -> A's profile loads", async () => {
    const { container } = mountScreen(A);
    await settle();

    userPickBranch(container, X);
    await settle();
    expect(heading(container)).toContain('Asok Branch');

    clickRefresh(container);
    await settle();

    userPickBranch(container, A);
    await settle();

    expect(branchSelect(container).value).toBe(A);
    expect(heading(container)).toContain('Silom Branch');
    expect(codeValue(container)).toBe('SILOM');
  });
});
