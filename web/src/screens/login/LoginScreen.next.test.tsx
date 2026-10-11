// S-9 #1223 (§7 `LoginScreen.tsx` [required], §9): both of LoginScreen's exits honour `next`.
// Design docs/design/xchange-v3-wave3-staff.md row S-9 (:216, :218) and its test row (:229): for the mount exit
// (an existing session) and the sign-in exit (a successful login), `/login?next=%2Fstaff` navigates to `/staff`;
// a refused `next` falls back to `homeOf(user)`; no `next` leaves `homeOf` unchanged; and the mount exit navigates
// exactly once and never back to `/login`. The refused values include the lane controller's ruling on #1223
// (issuecomment-6103413078): control characters between the slashes, `/login` and `/api/` paths.
// Written before the implementation by the test writer (claude/5d45b017). Do not edit, delete or weaken.
//
// The API this test pins, in `web/src/screens/login/LoginScreen.tsx`:
//
//   export function LoginScreen(props: {
//     auth: AuthApi;
//     navigate: (url: string) => void;
//     next?: string | null;          // what main.tsx reads from location.search, i.e. already decoded once
//   }): JSX.Element;
//
// `next` is handed in the way main.tsx will read it: new URLSearchParams(location.search).get('next').
// The rest of the screen is pinned by the existing LoginScreen.test.tsx, which this file does not repeat.
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

// homeOf sends this user to /desk (home.test.ts); so /desk below is "fell back to homeOf".
const SEAT_HOLDER: SessionUser = {
  id: 'u-7',
  name: 'Nok',
  organizationId: 'org-1',
  roles: [{ role: 'TELLER', branchId: 'XC-B01' }],
};
const HOME = '/desk';

/** The value main.tsx hands in for a raw query string, e.g. "?next=%2Fstaff" → "/staff". */
function nextFrom(search: string): string | null {
  return new URLSearchParams(search).get('next');
}

const REFUSED_RAW = [
  '%2F%2Fevil',
  '%2F%5Cevil.com',
  '%252F%252Fevil',
  'https%3A%2F%2Fevil',
  'javascript%3Aalert(1)',
  'staff',
  '',
  '%2F%09%2Fevil.com',
  '%2F%0A%2Fevil.com',
  '%2F%0D%2Fevil.com',
  '%2Flogin',
  '%2Flogin%3Fnext%3D%252Fstaff',
  '%2Fapi%2Fv3%2Fviews%2Fstaff_me',
  // rule 6, dot segments (ruling issuecomment-6103468024)
  '%2Fstaff%2F..%2Flogin',
  '%2Fstaff%2F%252e%252e%2Flogin',
  '%2Fstaff%2F%252E%252E%2Flogin',
  '%2Fx%2F..%2Fapi%2Fv3%2Fviews%2Fstaff_me',
  '%2Fstaff%2F.%2Ffloor',
];

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
  const LoginScreen = modules['./LoginScreen.tsx']?.LoginScreen;
  expect(LoginScreen, 'LoginScreen.tsx exports LoginScreen').toBeTypeOf('function');
  const host = document.createElement('div');
  document.body.appendChild(host);
  const navigate = vi.fn<(url: string) => void>();
  dispose = render(() => (LoginScreen as Screen)({ auth, navigate, next }), host);
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
  type(id, 'nok@example.com');
  type(pw, 'correct horse');
  const button = Array.from(host.querySelectorAll('button')).find((b) => b.textContent?.trim() === TH.submit);
  if (!button) throw new Error('no submit button');
  button.click();
  await settle();
}

const signedIn = () => fakeAuth({ session: async () => ({ user: SEAT_HOLDER }) });
const signsIn = () =>
  fakeAuth({ session: async () => ({ user: null }), login: async () => ({ user: SEAT_HOLDER }) });

describe('the mount exit (already signed in) honours next', () => {
  it('/login?next=%2Fstaff navigates to /staff', async () => {
    const { navigate } = mount(signedIn(), nextFrom('?next=%2Fstaff'));
    await settle();
    expect(navigate.mock.calls).toEqual([['/staff']]);
  });

  it.each(REFUSED_RAW)('a refused next=%j falls back to homeOf', async (raw) => {
    const { navigate } = mount(signedIn(), nextFrom(`?next=${raw}`));
    await settle();
    expect(navigate.mock.calls).toEqual([[HOME]]);
  });

  it('a query on an allowed path is kept: ?next=%2Fstaff%3Ftab%3Dfloor navigates to /staff?tab=floor', async () => {
    const { navigate } = mount(signedIn(), nextFrom('?next=%2Fstaff%3Ftab%3Dfloor'));
    await settle();
    expect(navigate.mock.calls).toEqual([['/staff?tab=floor']]);
  });

  it('navigates to exactly what safeNext returned: no second decode, no re-encode', async () => {
    // main.tsx's URLSearchParams decodes the query once (%252525 → %2525), safeNext once more (→ %25)
    const { navigate } = mount(signedIn(), nextFrom('?next=%2Fstaff%3Fq%3D%252525'));
    await settle();
    expect(navigate.mock.calls).toEqual([['/staff?q=%25']]);
  });

  it('no next leaves homeOf unchanged', async () => {
    const { navigate } = mount(signedIn(), nextFrom(''));
    await settle();
    expect(navigate.mock.calls).toEqual([[HOME]]);
  });

  it('no loop: it navigates exactly once and never to /login, even after more time passes', async () => {
    const { navigate } = mount(signedIn(), nextFrom('?next=%2Fstaff'));
    await settle();
    await settle();
    await new Promise((r) => setTimeout(r, 20));
    expect(navigate).toHaveBeenCalledTimes(1);
    for (const [url] of navigate.mock.calls) {
      expect(url.startsWith('/login')).toBe(false);
    }
  });
});

describe('the sign-in exit (a successful login) honours next', () => {
  it('/login?next=%2Fstaff navigates to /staff after signing in', async () => {
    const { host, navigate } = mount(signsIn(), nextFrom('?next=%2Fstaff'));
    await settle();
    expect(navigate).not.toHaveBeenCalled();
    await signIn(host);
    expect(navigate.mock.calls).toEqual([['/staff']]);
  });

  it.each(REFUSED_RAW)('a refused next=%j falls back to homeOf', async (raw) => {
    const { host, navigate } = mount(signsIn(), nextFrom(`?next=${raw}`));
    await settle();
    await signIn(host);
    expect(navigate.mock.calls).toEqual([[HOME]]);
  });

  it('a query on an allowed path is kept exactly after signing in: no second decode, no re-encode', async () => {
    const { host, navigate } = mount(signsIn(), nextFrom('?next=%2Fstaff%3Ftab%3Dfloor%26q%3D%252525'));
    await settle();
    await signIn(host);
    expect(navigate.mock.calls).toEqual([['/staff?tab=floor&q=%25']]);
  });

  it('no next leaves homeOf unchanged', async () => {
    const { host, navigate } = mount(signsIn());
    await settle();
    await signIn(host);
    expect(navigate.mock.calls).toEqual([[HOME]]);
  });
});
