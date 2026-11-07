// OL-7 admin-users-more (verifier row 86: "rows with actions are still 149px tall, actions wrap to 3 lines"; sub-controller
// claude/1cc51615's decision, /tmp/ol7/ORDER-claude-19-b.md item 2). Measured by claude/94389a6b on xc-v3 build 45e8457 at
// 1280px: an ACTIVE row has 5 buttons, 104+97+103+94+70px ≈ 500px with gaps, in a 268px action column (users.css U-5), so
// they wrap to 3 lines; the visible table is 882px of 1021px. Decided: Profile stays on the row; reset, grant, suspend and
// disable move into a per-row "more" menu. Every moved button keeps its data-users-action value or class, so the screen's
// behaviour tests only open the menu first (restated with this file, by claude/94389a6b, on that decision).
//
// The menu, as this file pins it:
//   - toggle: <button data-users-menu="toggle" aria-haspopup="menu" aria-expanded="false|true" aria-label={moreActions}>;
//   - list:   <div data-users-menu="list" role="menu">, in the DOM only while open, its buttons role="menuitem";
//   - ACTIVE rows: Profile + toggle on the row; reset, grant, suspend, disable in the list;
//     SUSPENDED rows: Profile + Resume + toggle on the row; reset in the list;
//     INVITED rows: Activate only, no toggle; DISABLED rows: Enable only, no toggle;
//   - one list open at a time; Escape or a click outside closes it and puts focus back on its toggle;
//   - grant, suspend, disable close the list when clicked; reset shows reset-confirm / reset-cancel inside the list, and
//     either of those closes it.
// Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { makeAdminUserRole, makeAdminUserRow } from '../../../shared/testing/fixtures';
import { UsersScreen } from './UsersScreen';
import { en, th } from './i18n';

const B01 = 'XC-B01';
const role = makeAdminUserRole({ role: 'TELLER', branch: B01, validFrom: '2026-10-01T00:00:00Z', validTo: null });

const USERS = [
  makeAdminUserRow({ userId: 'u-active', name: 'Alpha Active', status: 'ACTIVE', version: 1, roles: [role] }),
  makeAdminUserRow({ userId: 'u-active2', name: 'Bravo Active', status: 'ACTIVE', version: 1, roles: [role] }),
  makeAdminUserRow({ userId: 'u-susp', name: 'Charlie Suspended', status: 'SUSPENDED', version: 1, roles: [role] }),
  makeAdminUserRow({ userId: 'u-inv', name: 'Delta Invited', status: 'INVITED', version: 1, roles: [] }),
  makeAdminUserRow({ userId: 'u-dis', name: 'Echo Disabled', status: 'DISABLED', version: 1, roles: [role] }),
];

const disposers: Array<() => void> = [];
let writes: string[] = [];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function fakeFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const u = new URL(String(input), 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  if (method === 'GET' && u.pathname === '/api/v3/views/admin_branches') {
    return json(200, { version: 1, changed: true, data: { branches: [{ id: B01, name: 'Silom Branch' }] } });
  }
  if (method === 'GET' && u.pathname === '/api/v3/views/admin_users') {
    return json(200, { version: 1, changed: true, data: structuredClone(USERS) });
  }
  if (method === 'POST') {
    writes.push(u.pathname);
    return json(200, {});
  }
  return json(500, { error: { code: 'serverError', message: `unexpected ${method} ${u.pathname}` } });
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
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

function row(c: HTMLElement, id: string): HTMLElement {
  const r = c.querySelector<HTMLElement>(`tr[data-user-id="${id}"]`);
  expect(r, `row ${id}`).not.toBeNull();
  return r!;
}

/** Buttons on the row itself, outside any open menu list. */
function rowButtons(r: HTMLElement): HTMLButtonElement[] {
  return [...r.querySelectorAll<HTMLButtonElement>('.users-row-actions button')].filter(
    (b) => !b.closest('[data-users-menu="list"]'),
  );
}

const toggle = (r: HTMLElement) => r.querySelector<HTMLButtonElement>('[data-users-menu="toggle"]');
const list = (r: HTMLElement) => r.querySelector<HTMLElement>('[data-users-menu="list"]');

async function open(r: HTMLElement): Promise<HTMLElement> {
  toggle(r)!.click();
  await settle();
  const l = list(r);
  expect(l, 'menu list open').not.toBeNull();
  return l!;
}

beforeEach(() => {
  setLang('th');
  writes = [];
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
});

afterEach(() => {
  while (disposers.length) disposers.pop()!();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('users row "more" menu', () => {
  it('UM-1 ACTIVE row: Profile and the menu toggle on the row; reset, grant, suspend, disable are not on the row', async () => {
    const c = await mount();
    const r = row(c, 'u-active');
    const on = rowButtons(r);
    expect(on.map((b) => b.getAttribute('data-users-action') ?? b.getAttribute('data-users-menu'))).toEqual(['profile', 'toggle']);
    expect(r.querySelector('[data-users-action="reset"]')).toBeNull();
    expect(r.querySelector('.users-btn-grant')).toBeNull();
    expect(r.querySelector('[data-users-action="suspend"]')).toBeNull();
    expect(r.querySelector('.users-btn-disable')).toBeNull();
  });

  it('UM-2 the toggle: aria-haspopup menu, aria-expanded false, labelled moreActions (TH and EN)', async () => {
    expect(th.moreActions).toBe('การจัดการเพิ่มเติม');
    expect(en.moreActions).toBe('More actions');
    const c = await mount();
    const t = toggle(row(c, 'u-active'))!;
    expect(t.getAttribute('aria-haspopup')).toBe('menu');
    expect(t.getAttribute('aria-expanded')).toBe('false');
    expect(t.getAttribute('aria-label')).toBe(th.moreActions);
    expect(t.getAttribute('type')).toBe('button');
  });

  it('UM-3 opening: role="menu" list with reset, grant, suspend, disable as menuitems, in that order; aria-expanded true', async () => {
    const c = await mount();
    const r = row(c, 'u-active');
    const l = await open(r);
    expect(l.getAttribute('role')).toBe('menu');
    expect(toggle(r)!.getAttribute('aria-expanded')).toBe('true');
    const items = [...l.querySelectorAll<HTMLButtonElement>('button')];
    expect(items.map((b) => b.textContent?.trim())).toEqual([th.resetPassword, th.grant, th.suspend, th.disable]);
    expect(items.every((b) => b.getAttribute('role') === 'menuitem')).toBe(true);
    expect(l.querySelector('[data-users-action="reset"]')).not.toBeNull();
    expect(l.querySelector('.users-btn-grant')).not.toBeNull();
    expect(l.querySelector('[data-users-action="suspend"]')).not.toBeNull();
    expect(l.querySelector('.users-btn-disable')).not.toBeNull();
  });

  it('UM-4 SUSPENDED row: Profile, Resume and the toggle on the row; reset alone in the list', async () => {
    const c = await mount();
    const r = row(c, 'u-susp');
    expect(rowButtons(r).map((b) => b.getAttribute('data-users-action') ?? b.getAttribute('data-users-menu'))).toEqual([
      'profile',
      'resume',
      'toggle',
    ]);
    const l = await open(r);
    expect([...l.querySelectorAll('button')].map((b) => b.getAttribute('data-users-action'))).toEqual(['reset']);
  });

  it('UM-5 INVITED and DISABLED rows: one button, no toggle', async () => {
    const c = await mount();
    expect(toggle(row(c, 'u-inv'))).toBeNull();
    expect(rowButtons(row(c, 'u-inv')).map((b) => b.textContent?.trim())).toEqual([th.activate]);
    expect(toggle(row(c, 'u-dis'))).toBeNull();
    expect(rowButtons(row(c, 'u-dis')).map((b) => b.getAttribute('data-users-action'))).toEqual(['enable']);
  });

  it('UM-6 one list at a time: opening another row closes the first', async () => {
    const c = await mount();
    await open(row(c, 'u-active'));
    await open(row(c, 'u-active2'));
    expect(list(row(c, 'u-active'))).toBeNull();
    expect(toggle(row(c, 'u-active'))!.getAttribute('aria-expanded')).toBe('false');
    expect(c.querySelectorAll('[data-users-menu="list"]')).toHaveLength(1);
  });

  it('UM-7 the toggle again, Escape, or a click outside closes the list; Escape puts focus back on the toggle', async () => {
    const c = await mount();
    const r = row(c, 'u-active');
    await open(r);
    toggle(r)!.click();
    await settle();
    expect(list(r)).toBeNull();

    const l = await open(r);
    l.querySelector<HTMLButtonElement>('button')!.focus();
    l.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await settle();
    expect(list(r)).toBeNull();
    expect(document.activeElement).toBe(toggle(r));

    await open(r);
    document.body.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
    document.body.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await settle();
    expect(list(r)).toBeNull();
  });

  it('UM-8 a click inside the list does not count as outside', async () => {
    const c = await mount();
    const r = row(c, 'u-active');
    const l = await open(r);
    l.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
    l.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await settle();
    expect(list(r)).not.toBeNull();
  });

  it('UM-9 reset keeps the list open with confirm and cancel inside it; cancel closes it, nothing is sent', async () => {
    const c = await mount();
    const r = row(c, 'u-active');
    const l = await open(r);
    l.querySelector<HTMLButtonElement>('[data-users-action="reset"]')!.click();
    await settle();
    const l2 = list(r);
    expect(l2, 'still open').not.toBeNull();
    expect(l2!.querySelector('[data-users-action="reset-confirm"]')).not.toBeNull();
    l2!.querySelector<HTMLButtonElement>('[data-users-action="reset-cancel"]')!.click();
    await settle();
    expect(list(r)).toBeNull();
    expect(writes).toEqual([]);
  });

  it('UM-10 suspend from the list sends the suspend write and closes the list', async () => {
    const c = await mount();
    const r = row(c, 'u-active');
    const l = await open(r);
    l.querySelector<HTMLButtonElement>('[data-users-action="suspend"]')!.click();
    await settle();
    expect(list(r)).toBeNull();
    expect(writes.length).toBe(1);
  });

  it('UM-11 grant from the list opens the grant dialog and closes the list', async () => {
    const c = await mount();
    const r = row(c, 'u-active');
    const l = await open(r);
    l.querySelector<HTMLButtonElement>('.users-btn-grant')!.click();
    await settle();
    expect(list(r)).toBeNull();
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  });
});
