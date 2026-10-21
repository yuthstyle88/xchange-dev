// Written before the fix by the ticket author claude/studio-claude-4 (sub-controller under XC FE claude/e92db6),
// from the QA-F11 trace on #4305. Do not edit, delete or weaken; add tests in a file of your own.
//
// The loop: a signed-in customer reaches a staff page; the server answers 303 `/login?next=<page>`
// (api/src/presentation/pages.rs `to_login`). The staff login's mount exit (LoginScreen.tsx:42-48) treats any session
// as "signed in" and navigates to `safeNext(next) ?? homeOf(user)`: `next` is the staff page, and `homeOf` returns `/`
// for a user without staff roles (home.ts). The server sends the customer straight back to `/login`, which loops.
// Customers do sign in on this screen (deploy/server/seed-test-booking.sh:350, `/login?next=/app/book`), so a customer
// target under `/app` stays as it is today. A session with no staff role (roles empty, or only CUSTOMER) goes only to
// a customer path:
//   - [required] mount exit, customer session, next=/staff or next=/ or no next → navigate('/app'), exactly once;
//   - [required] mount exit, customer session (roles []), next=/desk → navigate('/app');
//   - [required] mount exit, customer session, next=/app/book → navigate('/app/book');
//   - [required] sign-in exit, a customer signs in with next=/staff → navigate('/app'); with next=/app/book → '/app/book';
//   - [required] a staff session (TELLER) with next=/staff still goes to /staff (LoginScreen.next.test.tsx pins the rest).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import * as table from './i18n';

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
type Screen = (props: {
  auth: AuthApiShape;
  navigate: (url: string) => void;
  next?: string | null;
}) => JSX.Element;

const modules = import.meta.glob<{ LoginScreen?: unknown }>('./LoginScreen.tsx', { eager: true });
const TH = table.th as unknown as Record<string, string>;

const CUSTOMER: SessionUser = {
  id: 'u-c1',
  name: 'Test Customer',
  organizationId: 'org-1',
  roles: [{ role: 'CUSTOMER', branchId: null }],
};
const ROLELESS: SessionUser = { id: 'u-c2', name: 'Roleless', organizationId: 'org-1', roles: [] };
const TELLER: SessionUser = {
  id: 'u-7',
  name: 'Nok',
  organizationId: 'org-1',
  roles: [{ role: 'TELLER', branchId: 'XC-B01' }],
};

let dispose: (() => void) | undefined;

beforeEach(() => {
  // AuthLayout's footer reads /health/ready; nothing else on this screen may call fetch.
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

function auth(session: SessionUser | null, loginAs?: SessionUser): AuthApiShape {
  return {
    session: async () => ({ user: session }),
    login: async () => {
      if (!loginAs) throw new Error('login was not expected in this test');
      return { user: loginAs };
    },
  };
}

function mount(a: AuthApiShape, next: string | null) {
  const LoginScreen = modules['./LoginScreen.tsx']?.LoginScreen;
  expect(LoginScreen, 'LoginScreen.tsx exports LoginScreen').toBeTypeOf('function');
  const host = document.createElement('div');
  document.body.appendChild(host);
  const navigate = vi.fn<(url: string) => void>();
  dispose = render(() => (LoginScreen as Screen)({ auth: a, navigate, next }), host);
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

async function signIn(host: HTMLElement) {
  const id = host.querySelector<HTMLInputElement>('input[name="identifier"]');
  const pw = host.querySelector<HTMLInputElement>('input[autocomplete="current-password"]');
  if (!id || !pw) throw new Error('no sign-in form');
  type(id, '0812345678');
  type(pw, 'correct horse');
  const button = Array.from(host.querySelectorAll('button')).find((b) => b.textContent?.trim() === TH.submit);
  if (!button) throw new Error('no submit button');
  button.click();
  await settle();
}

describe('staff login: a customer session is never sent to a staff page (#4305 loop)', () => {
  it.each([['/staff'], ['/'], [null]])('[required] mount exit, customer, next=%j → /app, once', async (next) => {
    const { navigate } = mount(auth(CUSTOMER), next);
    await settle();
    expect(navigate.mock.calls).toEqual([['/app']]);
  });

  it('[required] mount exit, a session with no roles at all, next=/desk → /app', async () => {
    const { navigate } = mount(auth(ROLELESS), '/desk');
    await settle();
    expect(navigate.mock.calls).toEqual([['/app']]);
  });

  it('[required] mount exit, customer, next=/app/book → /app/book (the seeded sign-in path)', async () => {
    const { navigate } = mount(auth(CUSTOMER), '/app/book');
    await settle();
    expect(navigate.mock.calls).toEqual([['/app/book']]);
  });

  it('[required] sign-in exit, customer, next=/staff → /app', async () => {
    const { host, navigate } = mount(auth(null, CUSTOMER), '/staff');
    await settle();
    await signIn(host);
    expect(navigate.mock.calls).toEqual([['/app']]);
  });

  it('[required] sign-in exit, customer, next=/app/book → /app/book', async () => {
    const { host, navigate } = mount(auth(null, CUSTOMER), '/app/book');
    await settle();
    await signIn(host);
    expect(navigate.mock.calls).toEqual([['/app/book']]);
  });

  it('[required] a staff session keeps its staff target: TELLER, next=/staff → /staff', async () => {
    const { navigate } = mount(auth(TELLER), '/staff');
    await settle();
    expect(navigate.mock.calls).toEqual([['/staff']]);
  });
});
