// W5 (RBAC design docs/design/blueprint/user-management-rbac.md §2 Q5 A, §3.7 W5): a session that must change its
// password sees the change page in place of the sign-in form, changes it, and signs in again.
// Up-front tests by the ticket's writer (claude/177aa7ef). The implementer makes them pass and must not edit, delete
// or weaken them.
//
// Contract these tests read (decided on xchange#564 by the BE controller, issuecomment-6245742828 and its
// amendment issuecomment-6245755335; scope by the FE controller, issuecomment-6245751076):
// - The login and session answers carry `user.mustChangePassword: boolean` (F11). A missing field reads as false.
// - POST /api/auth/password { currentPassword, newPassword } (F11). Success ends the session, because Identity
//   revokes every session, so the web signs in again with the new password. Refusals: 400 validationFailed (field
//   newPassword), 401 invalidCredentials.
// - The page is `web/src/screens/login/ChangePassword.tsx` (export ChangePassword), rendered by LoginScreen in place
//   of the sign-in form. Its root carries [data-change-password]. Inputs are named currentPassword
//   (autocomplete current-password), newPassword and confirmPassword (both autocomplete new-password). The submit
//   button reads `changeSubmit`. Its message line is [data-change-message].
// The whole path runs through the real authApi + ApiClient over a fake fetch, so the wiring is tested, not a fake.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import * as table from './i18n';

// The success answer of POST /api/auth/password: `200 {"saved": true}`, like logout, so the shared ApiClient (which
// reads JSON from every OK answer) stays untouched. Ruled by the BE controller, xchange#564 issuecomment-6245780522.
const CHANGE_OK = { status: 200, body: { saved: true } as unknown };

type Json = Record<string, unknown>;
interface Reply {
  status: number;
  body?: unknown;
}

const TELLER = { id: 'u-7', name: 'Nok', organizationId: 'org-1', roles: [{ role: 'TELLER', branchId: 'XC-B01' }] };
const TH = table.th as unknown as Record<string, string>;
const EN = table.en as unknown as Record<string, string>;

const screens = import.meta.glob<{ LoginScreen?: unknown }>('./LoginScreen.tsx', { eager: true });
const apis = import.meta.glob<{ authApi?: unknown }>('./auth-api.ts', { eager: true });
const pages = import.meta.glob<{ ChangePassword?: unknown }>('./ChangePassword.tsx', { eager: true });

let replies: Record<string, Reply | (() => Promise<Response>)>;
let posts: Array<{ path: string; body: Json }>;
let dispose: (() => void) | undefined;

function json(status: number, body?: unknown): Response {
  return body === undefined
    ? new Response(null, { status })
    : new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

beforeEach(() => {
  setLang('th');
  posts = [];
  replies = {
    '/api/auth/session': { status: 200, body: { user: null } },
    '/api/auth/password': CHANGE_OK,
  };
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = new URL(String(input), 'http://localhost').pathname;
      if ((init?.method ?? 'GET').toUpperCase() === 'POST') {
        posts.push({ path, body: typeof init?.body === 'string' ? (JSON.parse(init.body) as Json) : {} });
      }
      const r = replies[path];
      if (typeof r === 'function') return r();
      if (r) return json(r.status, r.body);
      return json(503, {});
    }),
  );
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

async function settle(): Promise<void> {
  for (let round = 0; round < 6; round++) {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

function tx(key: string, t: Record<string, string> = TH): string {
  expect(t[key], `i18n key ${key}`).toBeTypeOf('string');
  return t[key]!;
}

async function mount(): Promise<{ host: HTMLElement; navigate: ReturnType<typeof vi.fn> }> {
  const LoginScreen = screens['./LoginScreen.tsx']?.LoginScreen;
  const authApi = apis['./auth-api.ts']?.authApi;
  expect(LoginScreen).toBeTypeOf('function');
  expect(authApi).toBeTypeOf('function');
  const auth = (authApi as (api: ApiClient) => unknown)(new ApiClient());
  const host = document.createElement('div');
  document.body.appendChild(host);
  const navigate = vi.fn<(url: string) => void>();
  type Screen = (p: { auth: unknown; navigate: (url: string) => void }) => JSX.Element;
  dispose = render(() => (LoginScreen as Screen)({ auth, navigate }), host);
  await settle();
  return { host, navigate };
}

function input(host: ParentNode, name: string): HTMLInputElement {
  const el = host.querySelector<HTMLInputElement>(`input[name="${name}"]`);
  expect(el, `an input named ${name}`).not.toBeNull();
  return el!;
}

function type(el: HTMLInputElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

function button(host: ParentNode, text: string): HTMLButtonElement {
  const b = [...host.querySelectorAll('button')].find((x) => x.textContent?.trim() === text);
  expect(b, `a button reading ${JSON.stringify(text)}`).toBeTruthy();
  return b as HTMLButtonElement;
}

function page(host: HTMLElement): HTMLElement {
  const p = host.querySelector<HTMLElement>('[data-change-password]');
  expect(p, 'the change page [data-change-password]').not.toBeNull();
  return p!;
}

function changeMessage(host: HTMLElement): string {
  return (page(host).querySelector('[data-change-message]')?.textContent ?? '').trim();
}

async function signInMustChange(host: HTMLElement): Promise<void> {
  replies['/api/auth/login'] = { status: 200, body: { user: { ...TELLER, mustChangePassword: true } } };
  type(input(host, 'identifier'), 'nok');
  type(host.querySelector<HTMLInputElement>('input[autocomplete="current-password"]')!, 'Temp-Pass-16char');
  button(host, tx('submit')).click();
  await settle();
}

async function fillChange(host: HTMLElement, current: string, next: string, confirm = next): Promise<void> {
  const p = page(host);
  type(input(p, 'currentPassword'), current);
  type(input(p, 'newPassword'), next);
  type(input(p, 'confirmPassword'), confirm);
  button(p, tx('changeSubmit')).click();
  await settle();
}

describe('W5: the change page replaces the sign-in form (Q5 A)', () => {
  it('row 1: ChangePassword.tsx exports ChangePassword', () => {
    expect(pages['./ChangePassword.tsx']?.ChangePassword, 'web/src/screens/login/ChangePassword.tsx').toBeTypeOf('function');
  });

  it('row 2: a login answer with mustChangePassword shows the change page, not the home page', async () => {
    const { host, navigate } = await mount();
    await signInMustChange(host);
    expect(navigate).not.toHaveBeenCalled();
    const p = page(host);
    expect(input(p, 'currentPassword').getAttribute('autocomplete')).toBe('current-password');
    expect(input(p, 'newPassword').getAttribute('autocomplete')).toBe('new-password');
    expect(input(p, 'confirmPassword').getAttribute('autocomplete')).toBe('new-password');
    expect(host.querySelector('input[name="identifier"]'), 'the sign-in form is gone').toBeNull();
    expect(host.textContent).toContain(tx('changeTitle'));
  });

  it('row 3: a session answer with mustChangePassword on load shows the change page too', async () => {
    replies['/api/auth/session'] = { status: 200, body: { user: { ...TELLER, mustChangePassword: true } } };
    const { host, navigate } = await mount();
    expect(navigate).not.toHaveBeenCalled();
    page(host);
  });

  it('row 4: mustChangePassword false, or missing, goes home as before', async () => {
    for (const user of [{ ...TELLER, mustChangePassword: false }, TELLER]) {
      replies['/api/auth/session'] = { status: 200, body: { user } };
      const { navigate } = await mount();
      expect(navigate.mock.calls).toEqual([['/desk']]);
      dispose?.();
      dispose = undefined;
      document.body.innerHTML = '';
    }
  });
});

describe('W5: changing the password (F11 POST /api/auth/password)', () => {
  it('row 5: sends { currentPassword, newPassword } once, then the sign-in form returns with the done line', async () => {
    const { host, navigate } = await mount();
    await signInMustChange(host);
    await fillChange(host, 'Temp-Pass-16char', 'My-New-Pass-2026');
    const sent = posts.filter((x) => x.path === '/api/auth/password');
    expect(sent).toHaveLength(1);
    expect(sent[0]!.body).toEqual({ currentPassword: 'Temp-Pass-16char', newPassword: 'My-New-Pass-2026' });
    expect(navigate, 'the session ended: nothing to navigate to').not.toHaveBeenCalled();
    expect(host.querySelector('[data-change-password]'), 'the change page is gone').toBeNull();
    expect(input(host, 'identifier'), 'the sign-in form is back').toBeTruthy();
    expect(host.textContent).toContain(tx('changeDone'));
    expect(host.textContent).not.toContain('My-New-Pass-2026');
  });

  it('row 6: a confirmation that differs sends nothing and says so', async () => {
    const { host } = await mount();
    await signInMustChange(host);
    await fillChange(host, 'Temp-Pass-16char', 'My-New-Pass-2026', 'My-New-Pass-2027');
    expect(posts.filter((x) => x.path === '/api/auth/password')).toHaveLength(0);
    expect(changeMessage(host)).toBe(tx('changeErrMismatch'));
  });

  it.each([
    ['401 invalidCredentials', 401, { error: { code: 'invalidCredentials', message: 'SERVER-TEXT' } }, 'changeErr_invalidCredentials'],
    ['400 validationFailed', 400, { error: { code: 'validationFailed', field: 'newPassword', message: 'SERVER-TEXT' } }, 'changeErr_validationFailed'],
    ['an unknown code', 409, { error: { code: 'somethingElse', message: 'SERVER-TEXT' } }, 'errUnknown'],
  ])('row 7 (%s): shows the page\'s own sentence, stays on the page, never the server text', async (_n, status, body, key) => {
    replies['/api/auth/password'] = { status, body };
    const { host, navigate } = await mount();
    await signInMustChange(host);
    await fillChange(host, 'Temp-Pass-16char', 'My-New-Pass-2026');
    expect(changeMessage(host)).toBe(tx(key));
    expect(changeMessage(host)).not.toContain('SERVER-TEXT');
    expect(navigate).not.toHaveBeenCalled();
    page(host);
  });

  it('row 8: a network failure shows errNetwork and stays on the page', async () => {
    replies['/api/auth/password'] = () => Promise.reject(new TypeError('Failed to fetch'));
    const { host } = await mount();
    await signInMustChange(host);
    await fillChange(host, 'Temp-Pass-16char', 'My-New-Pass-2026');
    expect(changeMessage(host)).toBe(tx('errNetwork'));
    page(host);
  });

  it('row 9: one request while pending; the button is disabled', async () => {
    let release!: (r: Response) => void;
    replies['/api/auth/password'] = () => new Promise<Response>((res) => (release = res));
    const { host } = await mount();
    await signInMustChange(host);
    await fillChange(host, 'Temp-Pass-16char', 'My-New-Pass-2026');
    const b = button(page(host), tx('changeSubmit'));
    expect(b.disabled).toBe(true);
    b.click();
    await settle();
    expect(posts.filter((x) => x.path === '/api/auth/password')).toHaveLength(1);
    release(json(CHANGE_OK.status, CHANGE_OK.body));
    await settle();
    expect(host.textContent).toContain(tx('changeDone'));
  });
});

describe('W5 i18n', () => {
  it('row 10: every W5 key is worded in th and en, and en differs from th', () => {
    for (const key of [
      'changeTitle', 'changeIntro', 'currentPassword', 'newPassword', 'confirmPassword', 'changeSubmit', 'changeDone',
      'changeErrMismatch', 'changeErr_invalidCredentials', 'changeErr_validationFailed',
    ]) {
      expect(tx(key).trim(), `th.${key}`).not.toBe('');
      expect(tx(key, EN).trim(), `en.${key}`).not.toBe('');
      expect(EN[key], `en.${key} is English, not the Thai copy`).not.toBe(TH[key]);
    }
  });
});
