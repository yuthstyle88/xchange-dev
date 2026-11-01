// FE-T9 #5857 (design docs/design/fe-triage-1101.md §3 W11): the Users screen gets a search box that sends
// `filter.q` to `admin_users` (the server already reads it: api/src/presentation/http/v3/views/admin_users.rs:92-100,
// on u.name and employee_code), and the password reset needs a second press.
// Up-front tests by the FE sub-controller claude/01TxaRJG. The implementer makes them pass and must not edit, delete
// or weaken them.
//
// Contract these tests read (the ticket §7 marks each [required]):
// - Search is submit-style, no debounce: `input[data-users-search]` (maxlength 100, the server's limit) sits in a
//   <form>, so Enter submits it, and `button[data-users-search-submit]` submits it too. A submit reads admin_users
//   once with `q` = the trimmed text (no `q` when it is empty) and no `before`, and replaces the list.
// - The applied `q` (the last submitted one, not the text being typed) stays on every later read: "more"
//   (`button[data-action="more"]`, adds `before`), the refresh button, and the reload after a row write (#4558).
// - Reset: `[data-users-action="reset"]` posts nothing. It swaps itself, in that row only, for
//   `[data-users-action="reset-confirm"]` (th "ยืนยันรีเซ็ตรหัสผ่าน") and `[data-users-action="reset-cancel"]`.
//   Confirm posts the reset; cancel posts nothing. One row at a time confirms.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en, th } from './i18n';
import { UsersScreen } from './UsersScreen';

type Row = {
  userId: string;
  name: string;
  status: string;
  version: number;
  employeeCode?: string | null;
  roles: Array<{ role: string; branch: string | null; validFrom: string; validTo: string | null }>;
};

const B01 = 'XC-B01';
const TOTAL = 120;
const role = { role: 'TELLER', branch: B01, validFrom: '2026-10-01T00:00:00Z', validTo: null };

let db: Row[] = [];
let reads: URL[] = [];
let writes: Array<{ path: string; body: any }> = [];
const disposers: Array<() => void> = [];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

/** 120 ACTIVE rows; even ones are named "Ann nnn", odd ones "Bob nnn". 60 match "ann". */
function seed(): Row[] {
  const rows: Row[] = [];
  for (let i = 0; i < TOTAL; i++) {
    const n = String(i).padStart(3, '0');
    rows.push({
      userId: `u-${n}`,
      name: `${i % 2 === 0 ? 'Ann' : 'Bob'} ${n}`,
      status: 'ACTIVE',
      version: 1,
      employeeCode: `E-${n}`,
      roles: [role],
    });
  }
  return rows;
}

beforeEach(() => {
  setLang('th');
  db = seed();
  reads = [];
  writes = [];
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
        // Like the server: q matches the name or the employee code, case-insensitively.
        const q = (u.searchParams.get('q') ?? '').toLowerCase();
        const matching = q
          ? db.filter((r) => r.name.toLowerCase().includes(q) || (r.employeeCode ?? '').toLowerCase().includes(q))
          : db;
        const before = u.searchParams.get('before');
        const limit = Number(u.searchParams.get('limit') ?? '50');
        const start = before ? matching.findIndex((r) => r.userId === before) + 1 : 0;
        return json(200, { version: 1, changed: true, data: structuredClone(matching.slice(start, start + limit)) });
      }
      if (method === 'POST' && u.pathname.startsWith('/api/users/')) {
        const body = typeof init?.body === 'string' ? JSON.parse(init.body) : {};
        writes.push({ path: u.pathname, body });
        if (u.pathname === '/api/users/reset-password') {
          return json(200, { userId: body.userId, temporaryPassword: 'Kq7mPz3RtW9xVb2N', mustChangePassword: true });
        }
        const r = db.find((x) => x.userId === body.userId);
        if (r && u.pathname === '/api/users/suspend') {
          r.status = 'SUSPENDED';
          r.version += 1;
        }
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

function names(c: HTMLElement): string[] {
  return [...c.querySelectorAll<HTMLElement>('tr[data-user-id] strong')].map((s) => s.textContent ?? '');
}

function row(c: HTMLElement, userId: string): HTMLElement {
  const r = c.querySelector<HTMLElement>(`tr[data-user-id="${userId}"]`);
  expect(r, `a row tr[data-user-id="${userId}"]`).not.toBeNull();
  return r!;
}

function actions(r: HTMLElement): string[] {
  return [...r.querySelectorAll<HTMLElement>('[data-users-action]')].map((b) => b.dataset.usersAction!).sort();
}

function click(el: ParentNode, selector: string): void {
  const b = el.querySelector<HTMLElement>(selector);
  expect(b, `a control ${selector}`).not.toBeNull();
  b!.click();
}

function searchBox(c: HTMLElement): HTMLInputElement {
  const box = c.querySelector<HTMLInputElement>('input[data-users-search]');
  expect(box, 'the search box input[data-users-search]').not.toBeNull();
  return box!;
}

function typeInto(box: HTMLInputElement, text: string): void {
  box.value = text;
  box.dispatchEvent(new Event('input', { bubbles: true }));
  box.dispatchEvent(new Event('change', { bubbles: true }));
}

/** Types `text` and presses the search button. */
async function search(c: HTMLElement, text: string): Promise<void> {
  typeInto(searchBox(c), text);
  click(c, 'button[data-users-search-submit]');
  await settle();
}

async function more(c: HTMLElement): Promise<void> {
  click(c, 'button[data-action="more"]');
  await settle();
}

describe('W11 search (filter.q)', () => {
  it('S1: searching "ann" makes one read with q=ann and no before, and replaces the list', async () => {
    const c = await mount();
    expect(names(c).some((n) => n.startsWith('Bob')), 'the first page has both names').toBe(true);
    expect(searchBox(c).maxLength, 'the server refuses q over 100 characters').toBe(100);

    const before = reads.length;
    await search(c, '  ann ');
    const sent = reads.slice(before);
    expect(sent).toHaveLength(1);
    expect(sent[0]!.searchParams.get('q')).toBe('ann');
    expect(sent[0]!.searchParams.has('before')).toBe(false);
    expect(sent[0]!.searchParams.get('limit')).toBe('50');
    expect(names(c)).toHaveLength(50);
    expect(names(c).every((n) => n.startsWith('Ann')), 'only the matching rows are shown').toBe(true);
  });

  it('S1b: Enter submits it too (the box sits in a form)', async () => {
    const c = await mount();
    const box = searchBox(c);
    expect(box.form, 'the search box is inside a <form>').not.toBeNull();
    typeInto(box, 'ann');
    const before = reads.length;
    box.form!.requestSubmit();
    await settle();
    const sent = reads.slice(before);
    expect(sent).toHaveLength(1);
    expect(sent[0]!.searchParams.get('q')).toBe('ann');
  });

  it('S1c: typing alone reads nothing (no debounce timer)', async () => {
    const c = await mount();
    const before = reads.length;
    typeInto(searchBox(c), 'ann');
    await settle();
    await new Promise((r) => setTimeout(r, 600));
    await settle();
    expect(reads.slice(before)).toHaveLength(0);
  });

  it('S2: "more" keeps the applied q and adds before; text typed but not submitted is not sent', async () => {
    const c = await mount();
    await search(c, 'ann');
    typeInto(searchBox(c), 'bob');
    const before = reads.length;
    await more(c);
    const sent = reads.slice(before);
    expect(sent).toHaveLength(1);
    expect(sent[0]!.searchParams.get('q')).toBe('ann');
    expect(sent[0]!.searchParams.get('before')).toBe('u-098');
    expect(names(c)).toHaveLength(60);
    expect(names(c).every((n) => n.startsWith('Ann'))).toBe(true);
    expect(c.querySelector('button[data-action="more"]'), 'all 60 matches are shown').toBeNull();
  });

  it('S3: clearing the box and submitting reads without q, and the full list is back', async () => {
    const c = await mount();
    await search(c, 'ann');
    const before = reads.length;
    await search(c, '');
    const sent = reads.slice(before);
    expect(sent).toHaveLength(1);
    expect(sent[0]!.searchParams.has('q')).toBe(false);
    expect(sent[0]!.searchParams.has('before')).toBe(false);
    expect(names(c).some((n) => n.startsWith('Bob'))).toBe(true);

    await search(c, 'ann');
    const again = reads.length;
    await search(c, '   ');
    expect(reads.slice(again).map((u) => u.searchParams.has('q'))).toEqual([false]);
  });

  it('S4: the reload after a row write keeps q (the #4558 paging reload)', async () => {
    const c = await mount();
    await search(c, 'ann');
    await more(c);
    expect(names(c)).toHaveLength(60);

    const before = reads.length;
    click(row(c, 'u-100'), '[data-users-action="suspend"]');
    await settle();
    const confirm = document.querySelector<HTMLElement>('[data-users-action="confirm"], [data-action="confirm"]');
    if (confirm) {
      confirm.click();
      await settle();
    }
    expect(writes.map((w) => w.path)).toEqual(['/api/users/suspend']);
    const sent = reads.slice(before);
    expect(sent.length, 'the list is read again').toBeGreaterThan(0);
    expect(sent.map((u) => u.searchParams.get('q'))).toEqual(sent.map(() => 'ann'));
    expect(names(c)).toHaveLength(60);
    expect(names(c).every((n) => n.startsWith('Ann'))).toBe(true);
    expect(actions(row(c, 'u-100'))).toContain('resume');
  });

  it('S5: the refresh button keeps q', async () => {
    const c = await mount();
    await search(c, 'ann');
    const refresh = [...c.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === (th as Record<string, string>).refresh,
    );
    expect(refresh, 'the refresh button').toBeTruthy();
    const before = reads.length;
    refresh!.click();
    await settle();
    expect(reads.slice(before).map((u) => u.searchParams.get('q'))).toEqual(['ann']);
  });
});

describe('W11 reset needs a second press', () => {
  it('RC1: reset alone posts nothing; the row swaps it for reset-confirm and reset-cancel', async () => {
    const c = await mount();
    expect(actions(row(c, 'u-000'))).toEqual(['profile', 'reset', 'suspend']);
    click(row(c, 'u-000'), '[data-users-action="reset"]');
    await settle();
    expect(writes).toEqual([]);
    expect(actions(row(c, 'u-000'))).toEqual(['profile', 'reset-cancel', 'reset-confirm', 'suspend']);
    expect(row(c, 'u-000').querySelector('[data-users-action="reset-confirm"]')!.textContent!.trim()).toBe(
      'ยืนยันรีเซ็ตรหัสผ่าน',
    );
    expect(actions(row(c, 'u-001')), 'other rows are unchanged').toEqual(['profile', 'reset', 'suspend']);
  });

  it('RC2: cancel posts nothing and the row offers reset again', async () => {
    const c = await mount();
    click(row(c, 'u-000'), '[data-users-action="reset"]');
    await settle();
    click(row(c, 'u-000'), '[data-users-action="reset-cancel"]');
    await settle();
    expect(writes).toEqual([]);
    expect(actions(row(c, 'u-000'))).toEqual(['profile', 'reset', 'suspend']);
  });

  it('RC3: confirm posts one reset; afterwards the row offers reset again, with no confirm left', async () => {
    const c = await mount();
    click(row(c, 'u-000'), '[data-users-action="reset"]');
    await settle();
    click(row(c, 'u-000'), '[data-users-action="reset-confirm"]');
    await settle();
    expect(writes).toEqual([{ path: '/api/users/reset-password', body: { userId: 'u-000' } }]);
    expect(c.querySelector('[data-users-action="reset-confirm"]')).toBeNull();
    expect(actions(row(c, 'u-000'))).toEqual(['profile', 'reset', 'suspend']);
  });

  it('RC4: one row confirms at a time: pressing reset on another row moves the confirm there', async () => {
    const c = await mount();
    click(row(c, 'u-000'), '[data-users-action="reset"]');
    await settle();
    click(row(c, 'u-001'), '[data-users-action="reset"]');
    await settle();
    expect(writes).toEqual([]);
    expect(c.querySelectorAll('[data-users-action="reset-confirm"]')).toHaveLength(1);
    expect(actions(row(c, 'u-000'))).toEqual(['profile', 'reset', 'suspend']);
    expect(actions(row(c, 'u-001'))).toEqual(['profile', 'reset-cancel', 'reset-confirm', 'suspend']);
  });
});

describe('W11 i18n', () => {
  it('I1: the new keys are worded in th and en, and en differs from th', () => {
    const TH = th as unknown as Record<string, string>;
    const EN = en as unknown as Record<string, string>;
    expect(TH.resetConfirm).toBe('ยืนยันรีเซ็ตรหัสผ่าน');
    for (const key of ['resetConfirm', 'resetCancel', 'searchLabel', 'searchButton']) {
      expect(typeof TH[key] === 'string' && TH[key]!.trim() !== '', `th.${key}`).toBe(true);
      expect(typeof EN[key] === 'string' && EN[key]!.trim() !== '', `en.${key}`).toBe(true);
      expect(EN[key], `en.${key} is English`).not.toBe(TH[key]);
    }
  });
});
