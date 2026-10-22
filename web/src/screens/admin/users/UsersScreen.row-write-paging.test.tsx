// #4558 (docs/design/blueprint/user-management-rbac.md §12, rows UR1–UR3): a row write keeps the rows the admin has
// already paged. Today every write handler calls loadUsers(), which reads only the first 50 rows, so a row written
// on the second or third page disappears.
// Up-front tests by the XC FE controller claude/e8020026. The implementer makes them pass and must not edit, delete
// or weaken them.
//
// Contract these tests read:
// - admin_users is keyset-paged by name: `before` = the last shown row's userId, `limit` = the page size (50).
//   The stub below answers any `limit`, so a reload in one larger read and a reload page by page both pass.
// - Rows are `tr[data-user-id]`; actions `[data-users-action]` by status (ACTIVE: profile, suspend;
//   SUSPENDED: profile, resume; DISABLED: enable). "more" is `button[data-action="more"]`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { UsersScreen } from './UsersScreen';

type Row = {
  userId: string;
  name: string;
  status: string;
  version: number;
  roles: Array<{ role: string; branch: string | null; validFrom: string; validTo: string | null }>;
};

const B01 = 'XC-B01';
const TOTAL = 120;
const role = { role: 'TELLER', branch: B01, validFrom: '2026-10-01T00:00:00Z', validTo: null };

let db: Row[] = [];
let reads: URL[] = [];
const disposers: Array<() => void> = [];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function seed(): Row[] {
  const rows: Row[] = [];
  for (let i = 0; i < TOTAL; i++) {
    const n = String(i).padStart(3, '0');
    rows.push({
      userId: `u-${n}`,
      name: `User ${n}`,
      status: i === 70 ? 'DISABLED' : 'ACTIVE',
      version: 1,
      roles: i === 70 ? [] : [role],
    });
  }
  return rows;
}

function setStatus(userId: string, status: string): void {
  const r = db.find((x) => x.userId === userId)!;
  r.status = status;
  r.version += 1;
}

beforeEach(() => {
  setLang('th');
  db = seed();
  reads = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const u = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches') {
        return json(200, { version: 1, changed: true, data: { branches: [{ id: B01, name: 'Silom' }] } });
      }
      if (method === 'GET' && u.pathname === '/api/v3/views/admin_users') {
        reads.push(u);
        const before = u.searchParams.get('before');
        const limit = Number(u.searchParams.get('limit') ?? '50');
        const start = before ? db.findIndex((r) => r.userId === before) + 1 : 0;
        return json(200, { version: 1, changed: true, data: structuredClone(db.slice(start, start + limit)) });
      }
      if (method === 'POST' && u.pathname.startsWith('/api/users/')) {
        const body = typeof init?.body === 'string' ? JSON.parse(init.body) : {};
        const next: Record<string, string> = {
          '/api/users/suspend': 'SUSPENDED',
          '/api/users/resume': 'ACTIVE',
          '/api/users/enable': 'ACTIVE',
          '/api/users/disable': 'DISABLED',
        };
        if (next[u.pathname]) setStatus(body.userId, next[u.pathname]!);
        const r = db.find((x) => x.userId === body.userId);
        return json(200, { userId: body.userId, status: r?.status ?? 'ACTIVE', version: r?.version ?? 1 });
      }
      return json(200, { version: 1, changed: false });
    }),
  );
});

afterEach(() => {
  while (disposers.length) disposers.pop()!();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

async function settle(): Promise<void> {
  for (let round = 0; round < 8; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

async function mount(): Promise<HTMLElement> {
  const [branch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown] = createSignal(true);
  const c = document.createElement('div');
  document.body.appendChild(c);
  disposers.push(render(() => <UsersScreen branch={branch} period={period} shown={shown} />, c));
  await settle();
  return c;
}

function rows(c: HTMLElement): number {
  return c.querySelectorAll('tr[data-user-id]').length;
}

function row(c: HTMLElement, userId: string): HTMLElement | null {
  return c.querySelector<HTMLElement>(`tr[data-user-id="${userId}"]`);
}

function actions(r: HTMLElement): string[] {
  return [...r.querySelectorAll<HTMLElement>('[data-users-action]')].map((b) => b.dataset.usersAction!).sort();
}

async function more(c: HTMLElement): Promise<void> {
  const b = c.querySelector<HTMLButtonElement>('button[data-action="more"]');
  expect(b, 'a "more" button').not.toBeNull();
  b!.click();
  await settle();
}

async function act(c: HTMLElement, userId: string, action: string): Promise<void> {
  const r = row(c, userId);
  expect(r, `row ${userId} is shown before the write`).not.toBeNull();
  const b = r!.querySelector<HTMLElement>(`[data-users-action="${action}"]`);
  expect(b, `row ${userId} offers ${action}`).not.toBeNull();
  b!.click();
  await settle();
  // A status write may ask to confirm first; press the confirm control when one is shown.
  const confirm = document.querySelector<HTMLElement>('[data-users-action="confirm"], [data-action="confirm"]');
  if (confirm) {
    confirm.click();
    await settle();
  }
}

describe('#4558: a row write keeps the rows already paged (§12)', () => {
  it('UR1: 120 shown, suspend a second-page row ⇒ the row stays, now offers resume; 120 shown; no "more"', async () => {
    const c = await mount();
    await more(c);
    await more(c);
    expect(rows(c)).toBe(TOTAL);
    await act(c, 'u-075', 'suspend');
    const r = row(c, 'u-075');
    expect(r, 'the written row is still on screen').not.toBeNull();
    expect(actions(r!)).toEqual(['profile', 'resume']);
    expect(rows(c)).toBe(TOTAL);
    expect(c.querySelector('button[data-action="more"]')).toBeNull();
  });

  it('UR2: 100 shown, enable a DISABLED second-page row ⇒ the row stays, now offers suspend; "more" still offered', async () => {
    const c = await mount();
    await more(c);
    expect(rows(c)).toBe(100);
    await act(c, 'u-070', 'enable');
    const r = row(c, 'u-070');
    expect(r, 'the written row is still on screen').not.toBeNull();
    expect(actions(r!)).toEqual(['profile', 'suspend']);
    expect(rows(c)).toBeGreaterThanOrEqual(100);
    expect(c.querySelector('button[data-action="more"]'), '"more" is still offered').not.toBeNull();
  });

  it('UR3: first page only, suspend a row on it ⇒ one read after the write, with no before; 50 shown; "more" offered', async () => {
    const c = await mount();
    expect(rows(c)).toBe(50);
    const readsBefore = reads.length;
    await act(c, 'u-010', 'suspend');
    const after = reads.slice(readsBefore);
    expect(after).toHaveLength(1);
    expect(after[0]!.searchParams.get('before')).toBeNull();
    expect(rows(c)).toBe(50);
    expect(actions(row(c, 'u-010')!)).toEqual(['profile', 'resume']);
    expect(c.querySelector('button[data-action="more"]')).not.toBeNull();
  });
});
