import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { AdminUserRow } from '../../../shared/api/generated/admin_users';
import { UsersScreen } from './UsersScreen';

type UserRow = AdminUserRow & { employeeCode?: string | null };

const B01 = 'XC-B01';
const B02 = 'XC-B02';

interface FetchCall {
  url: string;
  method: string;
  body: unknown;
}

let fetchCalls: FetchCall[] = [];
let usersResponder: (u: URL) => Response;
let grantResponder: () => Response;
const disposers: Array<() => void> = [];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function makeUsers(count: number, prefix = 'u'): UserRow[] {
  return Array.from({ length: count }, (_, i) => ({
    userId: `${prefix}-${i + 1}`,
    name: `User ${prefix}-${i + 1}`,
    status: 'ACTIVE',
    version: 1,
    employeeCode: i === 0 ? 'E-0042' : null,
    roles: [
      {
        role: 'TELLER',
        branch: B01,
        validFrom: '2026-10-01T00:00:00Z',
        validTo: null,
      },
    ],
  }));
}

async function settle(): Promise<void> {
  await vi.runAllTimersAsync();
  await Promise.resolve();
  await Promise.resolve();
}

function mountScreen(initialBranch = B01): { container: HTMLElement; setBranch: Setter<string> } {
  const [branch, setBranch] = createSignal(initialBranch);
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown] = createSignal(true);
  const container = document.createElement('div');
  document.body.appendChild(container);
  const dispose = render(
    () => <UsersScreen branch={branch} period={period} shown={shown} />,
    container,
  );
  disposers.push(dispose);
  return { container, setBranch };
}

beforeEach(() => {
  vi.useFakeTimers({ now: new Date('2026-10-15T05:00:00Z') });
  fetchCalls = [];
  const defaultRows: UserRow[] = [
    {
      userId: 'u-1',
      name: 'Somchai Active',
      status: 'ACTIVE',
      version: 1,
      employeeCode: 'E-0042',
      roles: [
        {
          role: 'TELLER',
          branch: B01,
          validFrom: '2026-10-01T00:00:00Z',
          validTo: null,
        },
        {
          role: 'FINANCE',
          branch: B01,
          validFrom: '2026-09-01T00:00:00Z',
          validTo: '2026-10-05T00:00:00Z',
        },
      ],
    },
    {
      userId: 'u-2',
      name: 'Somsri Future',
      status: 'ACTIVE',
      version: 1,
      employeeCode: null,
      roles: [
        {
          role: 'RATE_MANAGER',
          branch: B01,
          validFrom: '2026-10-01T00:00:00Z',
          validTo: '2026-12-31T00:00:00Z',
        },
      ],
    },
  ];

  usersResponder = () => json(200, { version: 1, changed: true, data: structuredClone(defaultRows) });
  grantResponder = () => json(200, { userId: 'u-1', status: 'ACTIVE', version: 2 });

  document.body.innerHTML = '';
  setLang('en');

  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      fetchCalls.push({ url: u.pathname + u.search, method, body });

      if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches') {
        return json(200, {
          version: 1,
          changed: true,
          data: { branches: [{ id: B01, name: 'Silom' }, { id: B02, name: 'Sathorn' }] },
        });
      }
      if (method === 'GET' && u.pathname === '/api/v3/views/admin_users') {
        return usersResponder(u);
      }
      if (method === 'POST' && u.pathname === '/api/users/grant') {
        return grantResponder();
      }
      return json(404, { error: { code: 'notFound', message: 'Not Found' } });
    }),
  );
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  setLang('th');
});

describe('A05 users (#3329): grant period, ended roles, employeeCode column, paging', () => {
  it('row 1: grant with both dates empty sends validFrom: null, validTo: null', async () => {
    const { container } = mountScreen();
    await settle();

    container.querySelector<HTMLButtonElement>('.users-btn-grant')!.click();
    await settle();

    const form = container.querySelector<HTMLFormElement>('.users-dialog-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    const grants = fetchCalls.filter((c) => c.url === '/api/users/grant');
    expect(grants).toHaveLength(1);
    expect(grants[0]!.body).toEqual({
      userId: 'u-1',
      role: 'TELLER',
      branch: B01,
      validFrom: null,
      validTo: null,
    });
  });

  it('row 2: grant with 2026-11-01 -> 2026-12-31 sends Bangkok RFC 3339 start-of-day timestamps', async () => {
    const { container } = mountScreen();
    await settle();

    container.querySelector<HTMLButtonElement>('.users-btn-grant')!.click();
    await settle();

    const fromInput = container.querySelector<HTMLInputElement>('input[name="grantValidFrom"]')!;
    const toInput = container.querySelector<HTMLInputElement>('input[name="grantValidTo"]')!;
    fromInput.value = '2026-11-01';
    fromInput.dispatchEvent(new Event('input', { bubbles: true }));
    toInput.value = '2026-12-31';
    toInput.dispatchEvent(new Event('input', { bubbles: true }));

    const form = container.querySelector<HTMLFormElement>('.users-dialog-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    const grants = fetchCalls.filter((c) => c.url === '/api/users/grant');
    expect(grants).toHaveLength(1);
    expect(grants[0]!.body).toEqual({
      userId: 'u-1',
      role: 'TELLER',
      branch: B01,
      validFrom: '2026-11-01T00:00:00+07:00',
      validTo: '2026-12-31T00:00:00+07:00',
    });
  });

  it('row 3: grant refused 400 on validTo shows the worded error in the dialog and keeps it open', async () => {
    const msg = 'Valid-until date must be after valid-from date';
    grantResponder = () => json(400, { error: { code: 'invalidGrantPeriod', message: msg } });

    const { container } = mountScreen();
    await settle();

    container.querySelector<HTMLButtonElement>('.users-btn-grant')!.click();
    await settle();

    const form = container.querySelector<HTMLFormElement>('.users-dialog-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    const dialog = container.querySelector('[role="dialog"]');
    expect(dialog).not.toBeNull();
    expect(dialog!.querySelector('[role="alert"]')?.textContent).toContain(msg);
  });

  it('row 4: a user with one current and one ended role shows both; ended has data-role-ended and ended date; only current has revoke', async () => {
    const { container } = mountScreen();
    await settle();

    const row1 = container.querySelector('tr[data-user-id="u-1"]')!;
    const tags = row1.querySelectorAll('.users-role-tag');
    expect(tags.length).toBe(2);

    const currentTag = tags[0]!;
    const endedTag = tags[1]!;

    expect(currentTag.hasAttribute('data-role-ended')).toBe(false);
    expect(currentTag.querySelector('.users-btn-revoke')).not.toBeNull();

    expect(endedTag.hasAttribute('data-role-ended')).toBe(true);
    expect(endedTag.textContent).toContain('2026-10-05');
    expect(endedTag.querySelector('.users-btn-revoke')).toBeNull();
  });

  it('row 5: a role ending in the future is shown as current with "until <date>"', async () => {
    const { container } = mountScreen();
    await settle();

    const row2 = container.querySelector('tr[data-user-id="u-2"]')!;
    const tag = row2.querySelector('.users-role-tag')!;
    expect(tag.hasAttribute('data-role-ended')).toBe(false);
    expect(tag.textContent).toContain('until 2026-12-31');
    expect(tag.querySelector('.users-btn-revoke')).not.toBeNull();
  });

  it('row 6: a user with employeeCode "E-0042" / null shows E-0042 / — in the code column', async () => {
    const { container } = mountScreen();
    await settle();

    const row1Cells = container.querySelectorAll('tr[data-user-id="u-1"] td');
    const row2Cells = container.querySelectorAll('tr[data-user-id="u-2"] td');
    expect(row1Cells[0]?.textContent?.trim()).toBe('E-0042');
    expect(row2Cells[0]?.textContent?.trim()).toBe('—');
  });

  it('row 7: first read answers 50 rows -> shows 50 rows and "more" button; clicking reads before=<50th userId> and appends next page', async () => {
    const page1 = makeUsers(50, 'p1');
    const page2 = makeUsers(10, 'p2');
    usersResponder = (u) => {
      const before = u.searchParams.get('before');
      if (before === 'p1-50') {
        return json(200, { version: 1, changed: true, data: page2 });
      }
      return json(200, { version: 1, changed: true, data: page1 });
    };

    const { container } = mountScreen();
    await settle();

    expect(container.querySelectorAll('tbody tr').length).toBe(50);
    const moreBtn = container.querySelector<HTMLButtonElement>('button[data-action="more"]');
    expect(moreBtn).not.toBeNull();

    moreBtn!.click();
    await settle();

    expect(
      fetchCalls.some(
        (c) => c.url.includes('/api/v3/views/admin_users') && c.url.includes('before=p1-50') && c.url.includes('limit=50'),
      ),
    ).toBe(true);
    expect(container.querySelectorAll('tbody tr').length).toBe(60);
    expect(container.querySelector('button[data-action="more"]')).toBeNull();
  });

  it('row 8: a read answering < 50 rows hides the "more" button', async () => {
    const { container } = mountScreen();
    await settle();

    expect(container.querySelectorAll('tbody tr').length).toBe(2);
    expect(container.querySelector('button[data-action="more"]')).toBeNull();
  });

  it('row 9: switching branch after "more" restarts from page 1 of the new branch', async () => {
    const page1B1 = makeUsers(50, 'b1-p1');
    const page2B1 = makeUsers(50, 'b1-p2');
    const page1B2 = makeUsers(3, 'b2-p1');
    usersResponder = (u) => {
      const branch = u.searchParams.get('branch');
      const before = u.searchParams.get('before');
      if (branch === B02) {
        return json(200, { version: 1, changed: true, data: page1B2 });
      }
      if (before === 'b1-p1-50') {
        return json(200, { version: 1, changed: true, data: page2B1 });
      }
      return json(200, { version: 1, changed: true, data: page1B1 });
    };

    const { container, setBranch } = mountScreen(B01);
    await settle();

    container.querySelector<HTMLButtonElement>('button[data-action="more"]')!.click();
    await settle();
    expect(container.querySelectorAll('tbody tr').length).toBe(100);

    setBranch(B02);
    await settle();

    const rows = container.querySelectorAll('tbody tr');
    expect(rows.length).toBe(3);
    expect(rows[0]?.getAttribute('data-user-id')).toBe('b2-p1-1');
    expect(container.querySelector('button[data-action="more"]')).toBeNull();
  });
});
