import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import * as table from './i18n';
import { LoginScreen } from './LoginScreen';

interface SessionUser {
  id: string;
  name: string;
  organizationId: string;
  roles: Array<{ role: string; branchId: string | null }>;
}

interface Creds {
  identifier: string;
  password: string;
}

interface AuthApiShape {
  session(): Promise<{ user: SessionUser | null }>;
  login(p: Creds): Promise<{ user: SessionUser }>;
}

const TH = table.th as unknown as Record<string, string>;

const TELLER: SessionUser = {
  id: 'u-7',
  name: 'Nok',
  organizationId: 'org-1',
  roles: [{ role: 'TELLER', branchId: 'XC-B01' }],
};

const OWNER: SessionUser = {
  id: 'u-1',
  name: 'Somchai',
  organizationId: 'org-1',
  roles: [
    { role: 'EXECUTIVE', branchId: null },
    { role: 'ADMIN', branchId: null },
  ],
};

let dispose: (() => void) | undefined;

beforeEach(() => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => new Response('{}', { status: 503, headers: { 'Content-Type': 'application/json' } })),
  );
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (err: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function fakeAuth(over: {
  session?: () => Promise<{ user: SessionUser | null }>;
  login?: (p: Creds) => Promise<{ user: SessionUser }>;
}): AuthApiShape {
  return {
    session: over.session ?? (async () => ({ user: null })),
    login:
      over.login ??
      (async () => {
        throw new Error('login was not expected in this test');
      }),
  };
}

function mount(auth: AuthApiShape, next?: string | null) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const navigate = vi.fn<(url: string) => void>();
  dispose = render(() => <LoginScreen auth={auth} navigate={navigate} next={next} />, host);
  return { host, navigate };
}

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

function type(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function identifierInput(host: HTMLElement): HTMLInputElement {
  const el = host.querySelector<HTMLInputElement>('input[name="identifier"]');
  if (!el) throw new Error('no identifier input');
  return el;
}

function passwordInput(host: HTMLElement): HTMLInputElement {
  const el = host.querySelector<HTMLInputElement>('input[autocomplete="current-password"]');
  if (!el) throw new Error('no password input');
  return el;
}

function submitButton(host: HTMLElement): HTMLButtonElement {
  const el = Array.from(host.querySelectorAll('button')).find((b) => b.textContent?.trim() === TH.submit);
  if (!el) throw new Error('no submit button');
  return el;
}

function messageLine(host: HTMLElement): string {
  const el = host.querySelector('#message');
  if (!el) throw new Error('no #message element');
  return (el.textContent ?? '').trim();
}

describe('LoginScreen session race conditions (#2887)', () => {
  it('1: submit while session() is in flight; session() rejects, login() resolves TELLER', async () => {
    const sessionDeferred = deferred<{ user: SessionUser | null }>();
    const loginDeferred = deferred<{ user: SessionUser }>();
    const auth = fakeAuth({
      session: () => sessionDeferred.promise,
      login: () => loginDeferred.promise,
    });
    const { host, navigate } = mount(auth);

    type(identifierInput(host), 'nok@example.com');
    type(passwordInput(host), 'password123');
    submitButton(host).click();
    await settle();

    sessionDeferred.reject(new TypeError('Failed to fetch'));
    await settle();
    expect(messageLine(host)).toBe('');

    loginDeferred.resolve({ user: TELLER });
    await settle();
    expect(messageLine(host)).toBe('');
    expect(navigate.mock.calls).toEqual([['/desk']]);
  });

  it('2: submit while session() is in flight; session() resolves late with OWNER; login() resolves TELLER', async () => {
    const sessionDeferred = deferred<{ user: SessionUser | null }>();
    const loginDeferred = deferred<{ user: SessionUser }>();
    const auth = fakeAuth({
      session: () => sessionDeferred.promise,
      login: () => loginDeferred.promise,
    });
    const { host, navigate } = mount(auth);

    type(identifierInput(host), 'nok@example.com');
    type(passwordInput(host), 'password123');
    submitButton(host).click();
    await settle();

    sessionDeferred.resolve({ user: OWNER });
    await settle();
    expect(navigate.mock.calls).toEqual([]);

    loginDeferred.resolve({ user: TELLER });
    await settle();
    expect(navigate.mock.calls).toEqual([['/desk']]);
  });

  it('3: submit while session() is in flight; login() rejects invalidCredentials, then session() rejects TypeError', async () => {
    const sessionDeferred = deferred<{ user: SessionUser | null }>();
    const loginDeferred = deferred<{ user: SessionUser }>();
    const auth = fakeAuth({
      session: () => sessionDeferred.promise,
      login: () => loginDeferred.promise,
    });
    const { host } = mount(auth);

    type(identifierInput(host), 'nok@example.com');
    type(passwordInput(host), 'wrong-pass');
    submitButton(host).click();
    await settle();

    loginDeferred.reject(new ApiError({ code: 'invalidCredentials', status: 401, message: 'invalid credentials' }));
    await settle();
    expect(messageLine(host)).toBe(TH.err_invalidCredentials);

    sessionDeferred.reject(new TypeError('Failed to fetch'));
    await settle();
    expect(messageLine(host)).toBe(TH.err_invalidCredentials);
  });

  it('4 (kept): no submit; session() resolves user', async () => {
    const auth = fakeAuth({
      session: async () => ({ user: TELLER }),
    });
    const { navigate } = mount(auth);
    await settle();
    expect(navigate.mock.calls).toEqual([['/desk']]);
  });

  it('5 (kept): no submit; session() rejects', async () => {
    const auth = fakeAuth({
      session: async () => {
        throw new TypeError('Failed to fetch');
      },
    });
    const { host } = mount(auth);
    await settle();
    expect(messageLine(host)).toBe(TH.errNetwork);
  });
});
