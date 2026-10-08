import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import type { AuthApi, AuthUser } from './auth-api';
import { th } from './i18n';
import { LoginScreen } from './LoginScreen';

type StatusAnswer = { user: AuthUser | null };
type LoginAnswer = { user: AuthUser; mustChange?: boolean };
type Creds = { username: string; password: string };

const FRONT: AuthUser = { id: 'employee-7', username: 'nok', name: 'Nok', role: 'FRONT', branchId: 'XC-B01' };

let dispose: (() => void) | undefined;
let fetched: string[];

beforeEach(() => {
  fetched = [];
  // AppShell reads /health/ready; nothing else on this screen may call fetch.
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string) => {
      fetched.push(String(url));
      return new Response('{}', { status: 503, headers: { 'Content-Type': 'application/json' } });
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

/** A fake AuthApi: `status` and `login` answer through the functions given, and every call is recorded. */
function fakeAuth(
  status: () => Promise<StatusAnswer>,
  login: (p: Creds) => Promise<LoginAnswer> = async () => {
    throw new Error('login was not expected in this test');
  },
) {
  const calls = { status: 0, login: [] as Creds[] };
  const auth: AuthApi = {
    status: () => {
      calls.status += 1;
      return status();
    },
    login: (p: Creds) => {
      calls.login.push({ ...p });
      return login(p);
    },
  };
  return { auth, calls };
}

function mount(auth: AuthApi) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const navigate = vi.fn<(url: string) => void>();
  dispose = render(() => <LoginScreen auth={auth} navigate={navigate} />, host);
  return { host, navigate };
}

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

function usernameInput(host: HTMLElement): HTMLInputElement {
  const el = host.querySelector<HTMLInputElement>('input[autocomplete="username"]');
  if (!el) throw new Error('no username input (autocomplete="username")');
  return el;
}

function passwordInput(host: HTMLElement): HTMLInputElement {
  const el = host.querySelector<HTMLInputElement>('input[autocomplete="current-password"]');
  if (!el) throw new Error('no password input (autocomplete="current-password")');
  return el;
}

function submitButton(host: HTMLElement, text: string = th.submit): HTMLButtonElement {
  const el = Array.from(host.querySelectorAll('button')).find((b) => b.textContent?.trim() === text);
  if (!el) throw new Error(`no button with text ${JSON.stringify(text)}`);
  return el;
}

function type(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('LoginScreen — mustChange with FRONT role (#480)', () => {
  it('a FRONT user with mustChange true goes to /staff', async () => {
    const fake = fakeAuth(
      async () => ({ user: null }),
      async () => ({ user: FRONT, mustChange: true }),
    );
    const { host, navigate } = mount(fake.auth);
    await settle();

    type(usernameInput(host), 'nok');
    type(passwordInput(host), 'secret-pw-1');
    submitButton(host).click();
    await settle();

    expect(fake.calls.login).toEqual([{ username: 'nok', password: 'secret-pw-1' }]);
    expect(navigate.mock.calls).toEqual([['/staff']]);
  });
});
