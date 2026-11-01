// FE-T9 #5857 (design docs/design/fe-triage-1101.md §3 W12): after a password change the sign-in page shows
// `changeDone` as a success notice, not in the red `#message` alert (`LoginScreen.tsx:106,130`, `auth.css:134`).
// Up-front tests by the FE sub-controller claude/01TxaRJG. The implementer makes them pass and must not edit, delete
// or weaken them.
//
// Contract these tests read (the ticket §7 marks each [required]):
// - The notice is an element with role="status" and the class `xc-auth-success`, inside the sign-in form's card.
//   `#message` (role="alert") stays empty while it shows.
// - The next sign-in submit clears the notice; a refusal then shows in `#message` as before.
// - auth.css styles `.xc-auth-success` with a colour of its own, not the alert's #a5003c. jsdom computes no colours,
//   so the rule is read from the file (as LoginScreen.layout.test.tsx does), not from the page.
// The whole path runs through the real authApi + ApiClient over a fake fetch, like LoginScreen.w5.test.tsx.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { authApi } from './auth-api';
import * as table from './i18n';
import { LoginScreen } from './LoginScreen';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Json = Record<string, unknown>;
interface Reply {
  status: number;
  body?: unknown;
}

const TELLER = { id: 'u-7', name: 'Nok', organizationId: 'org-1', roles: [{ role: 'TELLER', branchId: 'XC-B01' }] };
const TH = table.th as unknown as Record<string, string>;

let authCss = '';
let replies: Record<string, Reply>;
let posts: Array<{ path: string; body: Json }>;
let dispose: (() => void) | undefined;

function json(status: number, body?: unknown): Response {
  return body === undefined
    ? new Response(null, { status })
    : new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const dir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  authCss = fs.readFileSync(dir + '../../shared/ui/auth.css', 'utf8');
});

beforeEach(() => {
  setLang('th');
  posts = [];
  replies = {
    '/api/auth/session': { status: 200, body: { user: null } },
    '/api/auth/password': { status: 200, body: { saved: true } },
  };
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = new URL(String(input), 'http://localhost').pathname;
      if ((init?.method ?? 'GET').toUpperCase() === 'POST') {
        posts.push({ path, body: typeof init?.body === 'string' ? (JSON.parse(init.body) as Json) : {} });
      }
      const r = replies[path];
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

async function mount(): Promise<HTMLElement> {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const auth = authApi(new ApiClient());
  dispose = render(() => <LoginScreen auth={auth} navigate={vi.fn()} />, host);
  await settle();
  return host;
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

async function signIn(host: HTMLElement, password: string): Promise<void> {
  type(input(host, 'identifier'), 'nok');
  type(host.querySelector<HTMLInputElement>('input[autocomplete="current-password"]')!, password);
  button(host, TH.submit!).click();
  await settle();
}

/** Signs in with a temporary password, changes it, and lands back on the sign-in form. */
async function changePassword(host: HTMLElement): Promise<void> {
  replies['/api/auth/login'] = { status: 200, body: { user: { ...TELLER, mustChangePassword: true } } };
  await signIn(host, 'Temp-Pass-16char');
  const page = host.querySelector<HTMLElement>('[data-change-password]');
  expect(page, 'the change page').not.toBeNull();
  type(input(page!, 'currentPassword'), 'Temp-Pass-16char');
  type(input(page!, 'newPassword'), 'My-New-Pass-2026');
  type(input(page!, 'confirmPassword'), 'My-New-Pass-2026');
  button(page!, TH.changeSubmit!).click();
  await settle();
  expect(posts.filter((p) => p.path === '/api/auth/password')).toHaveLength(1);
  expect(host.querySelector('input[name="identifier"]'), 'the sign-in form is back').not.toBeNull();
}

function alertText(host: HTMLElement): string {
  const el = host.querySelector('#message');
  expect(el, 'the #message line').not.toBeNull();
  expect(el!.getAttribute('role')).toBe('alert');
  return (el!.textContent ?? '').trim();
}

function notices(host: HTMLElement): HTMLElement[] {
  return [...host.querySelectorAll<HTMLElement>('[role="status"]')].filter((n) => (n.textContent ?? '').trim() !== '');
}

describe('W12: the password-change success is a notice, not an alert', () => {
  it('row 1: after a change, changeDone shows in a role="status" element with the class xc-auth-success; #message stays empty', async () => {
    const host = await mount();
    await changePassword(host);
    const shown = notices(host);
    expect(shown, 'one non-empty role="status" element').toHaveLength(1);
    expect(shown[0]!.textContent!.trim()).toBe(TH.changeDone);
    expect(shown[0]!.classList.contains('xc-auth-success')).toBe(true);
    expect(alertText(host)).toBe('');
  });

  it('row 2: the next sign-in clears the notice; a refusal then shows in #message as before', async () => {
    const host = await mount();
    await changePassword(host);
    replies['/api/auth/login'] = { status: 401, body: { error: { code: 'invalidCredentials', message: 'x' } } };
    await signIn(host, 'wrong-password');
    expect(notices(host)).toHaveLength(0);
    expect(alertText(host)).not.toBe('');
    expect(alertText(host)).not.toBe(TH.changeDone);
  });

  it('row 3: a plain sign-in page shows no notice', async () => {
    const host = await mount();
    expect(notices(host)).toHaveLength(0);
    expect(alertText(host)).toBe('');
  });

  it('row 4: auth.css colours .xc-auth-success, and not with the alert red #a5003c', () => {
    const rule = /\.xc-auth-success\s*\{([^}]*)\}/.exec(authCss);
    expect(rule, 'a .xc-auth-success rule in shared/ui/auth.css').not.toBeNull();
    const colour = /(?:^|;|\s)color\s*:\s*([^;]+);?/.exec(rule![1]!);
    expect(colour, 'the rule sets color').not.toBeNull();
    expect(colour![1]!.trim().toLowerCase()).not.toBe('#a5003c');
  });
});
