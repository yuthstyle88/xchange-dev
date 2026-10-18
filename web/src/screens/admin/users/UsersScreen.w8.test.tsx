// W8 (F9 #3155, owner Q5 A; the BE controller's F9 contract on xchange#564): userCreate no longer takes a typed
// password. The server answers `temporaryPassword` once; the screen shows it once with a copy button, never stores it,
// and clears it on Done or on a branch change.
// Up-front tests by the ticket's writer (claude/177aa7ef). The implementer makes them pass and must not edit, delete
// or weaken them.
//
// Contract these tests read (the ticket §7 marks each [required]):
// - `web/src/shared/ui/TempPasswordNotice.tsx` exports `TempPasswordNotice`, used by every screen that shows one.
// - The notice is `[data-temp-password]`; the password is in `[data-temp-password-value]`; the copy button is
//   `[data-temp-password-copy]` (navigator.clipboard.writeText); Done is `[data-temp-password-done]`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { UsersScreen } from './UsersScreen';
import * as table from './i18n';

const TEMP = 'Kq7mPz3RtW9xVb2N';
const B01 = 'XC-B01';
const B02 = 'XC-B02';
const INVITED = { userId: 'u-inv', name: 'Invited Person', status: 'INVITED', version: 1, roles: [] };

let posts: Array<{ path: string; body: Record<string, unknown> }> = [];
let createReply: { status: number; body: unknown } = { status: 200, body: {} };
let writeText: ReturnType<typeof vi.fn>;
let setItem: ReturnType<typeof vi.spyOn>;
const disposers: Array<() => void> = [];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

beforeEach(() => {
  setLang('th');
  posts = [];
  createReply = { status: 200, body: { userId: 'u-new', status: 'ACTIVE', version: 1, temporaryPassword: TEMP } };
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const u = new URL(String(input), 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (u.pathname === '/api/v3/views/admin_branches') {
        return json(200, { version: 1, changed: true, data: { branches: [{ id: B01, name: 'Silom' }, { id: B02, name: 'Siam' }] } });
      }
      if (u.pathname === '/api/v3/views/admin_users') {
        return json(200, { version: 1, changed: true, data: [structuredClone(INVITED)] });
      }
      if (method === 'POST' && u.pathname === '/api/users') {
        posts.push({ path: u.pathname, body: JSON.parse(String(init?.body ?? '{}')) as Record<string, unknown> });
        return json(createReply.status, createReply.body);
      }
      return json(200, { version: 1, changed: false });
    }),
  );
  writeText = vi.fn(async () => {});
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  setItem = vi.spyOn(Storage.prototype, 'setItem');
});

afterEach(() => {
  while (disposers.length) disposers.pop()!();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setItem.mockRestore();
  setLang('th');
});

async function settle(): Promise<void> {
  for (let round = 0; round < 6; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

async function mount(): Promise<{ c: HTMLElement; setBranch: (b: string) => void }> {
  const [branch, setBranch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown] = createSignal(true);
  const c = document.createElement('div');
  document.body.appendChild(c);
  disposers.push(render(() => <UsersScreen branch={branch} period={period} shown={shown} />, c));
  await settle();
  return { c, setBranch };
}

function typeInto(c: HTMLElement, selector: string, value: string): void {
  const el = c.querySelector<HTMLInputElement>(selector);
  expect(el, `an input ${selector}`).not.toBeNull();
  el!.value = value;
  el!.dispatchEvent(new Event('input', { bubbles: true }));
}

async function createOne(c: HTMLElement): Promise<void> {
  c.querySelector<HTMLButtonElement>('.users-btn-primary')!.click();
  await settle();
  typeInto(c, 'input[name="name"]', 'New Staff');
  typeInto(c, 'input[name="phone"]', '0812345678');
  c.querySelector<HTMLFormElement>('.users-dialog-form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  await settle();
}

const notice = (c: HTMLElement) => c.querySelector<HTMLElement>('[data-temp-password]');

describe('W8: the temporary password from userCreate (Users screen)', () => {
  it('row 1: web/src/shared/ui/TempPasswordNotice.tsx exports TempPasswordNotice', () => {
    const mods = import.meta.glob<{ TempPasswordNotice?: unknown }>('../../../shared/ui/TempPasswordNotice.tsx', { eager: true });
    expect(mods['../../../shared/ui/TempPasswordNotice.tsx']?.TempPasswordNotice).toBeTypeOf('function');
  });

  it('row 2: create shows the temporary password once, with copy and Done; nothing typed is sent as a password', async () => {
    const { c } = await mount();
    await createOne(c);
    expect(posts).toHaveLength(1);
    expect('password' in posts[0]!.body, 'no password in the userCreate body').toBe(false);
    expect(notice(c), 'the notice').not.toBeNull();
    expect(notice(c)!.querySelector('[data-temp-password-value]')?.textContent).toContain(TEMP);
    expect(notice(c)!.querySelector('[data-temp-password-copy]'), 'a copy button').not.toBeNull();
    expect(notice(c)!.querySelector('[data-temp-password-done]'), 'a Done button').not.toBeNull();
    expect(c.textContent!.split(TEMP).length - 1, 'shown exactly once').toBe(1);
  });

  it('row 3: copy writes exactly the temporary password to the clipboard', async () => {
    const { c } = await mount();
    await createOne(c);
    notice(c)!.querySelector<HTMLButtonElement>('[data-temp-password-copy]')!.click();
    await settle();
    expect(writeText.mock.calls).toEqual([[TEMP]]);
  });

  it('row 4: Done removes the notice and the password from the page', async () => {
    const { c } = await mount();
    await createOne(c);
    notice(c)!.querySelector<HTMLButtonElement>('[data-temp-password-done]')!.click();
    await settle();
    expect(notice(c)).toBeNull();
    expect(c.textContent).not.toContain(TEMP);
  });

  it('row 5: a branch change clears the notice', async () => {
    const { c, setBranch } = await mount();
    await createOne(c);
    expect(notice(c)).not.toBeNull();
    setBranch(B02);
    await settle();
    expect(notice(c)).toBeNull();
    expect(c.textContent).not.toContain(TEMP);
  });

  it('row 6: activate (the retry path) also sends no password and shows the temporary password', async () => {
    const { c } = await mount();
    c.querySelector<HTMLButtonElement>('.users-btn-activate')!.click();
    await settle();
    expect(c.querySelector('[role="dialog"] input[type="password"]'), 'no password field').toBeNull();
    typeInto(c, 'input[name="activatePhone"]', '0899999999');
    c.querySelector<HTMLFormElement>('.users-dialog-form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    expect(posts).toHaveLength(1);
    expect(posts[0]!.body.userId).toBe('u-inv');
    expect('password' in posts[0]!.body).toBe(false);
    expect(notice(c)!.querySelector('[data-temp-password-value]')?.textContent).toContain(TEMP);
  });

  it('row 7: the password is never put in localStorage or sessionStorage', async () => {
    const { c } = await mount();
    await createOne(c);
    notice(c)!.querySelector<HTMLButtonElement>('[data-temp-password-copy]')!.click();
    await settle();
    const stored = setItem.mock.calls.filter((call: unknown[]) => JSON.stringify(call).includes(TEMP));
    expect(stored).toEqual([]);
  });

  it('row 8: an answer without temporaryPassword shows no notice (a server before F9)', async () => {
    createReply = { status: 200, body: { userId: 'u-new', status: 'ACTIVE', version: 1 } };
    const { c } = await mount();
    await createOne(c);
    expect(notice(c)).toBeNull();
  });

  it('row 9: the copy text exists in th and en, worded', () => {
    for (const lang of ['th', 'en'] as const) {
      const t = (table as unknown as Record<string, Record<string, unknown>>)[lang];
      expect(typeof t.tempPasswordCopy === 'string' && (t.tempPasswordCopy as string).trim() !== '', `${lang}.tempPasswordCopy`).toBe(true);
    }
  });
});
