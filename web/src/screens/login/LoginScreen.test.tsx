// OR-3 web follow-up (#762 §7 `LoginScreen.tsx` [required], §9 edge cases): the sign-in screen signs in
// through the Identity BFF that OR-3 (#752, merged 8234e77) added (design docs/design/blueprint/org-roles-branches.md
// §5.1).
// Replaces the #396/#480 pins of the old `/api/auth {action}` flow (status, login({username}), mustChange → /staff).
// Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken.
// Amended by #1157 (claude/08f7f46f; owner: "ไม่มีระบบเชิญสำหรับพนักงาน"): the one-time-code step is removed with
// its route, so E5/E6 and the code-step assertions go; E2 now pins one submit and no code-step button or text.
//
// The API this test pins, in `web/src/screens/login/LoginScreen.tsx`:
//
//   export function LoginScreen(props: { auth: AuthApi; navigate: (url: string) => void }): JSX.Element;
//
// with `AuthApi` as pinned in ./auth-api.test.ts (session, login).
//
// What it pins, row by row (the ticket's §9 numbers):
// - E1 on load `session()` once; a user → navigate(homeOf(user)) (TELLER → /desk, owner → /, else /).
// - E2 signed out: an identifier field (name identifier, autocomplete username, type text, required, maxlength 254,
//   NO pattern), the password field as before, and the submit button: the form's only button.
// - E3 a failed `session()` shows the screen's own sentence for its code; a network failure shows errNetwork.
// - E4 submit → `login({identifier, password})` exactly once; e-mail and phone identifiers are not blocked.
// - E7 every refusal shows the sentence for its CODE from ./i18n, never err.message and never by HTTP status;
//   an unknown code → errUnknown; a non-ApiError → errNetwork.
// - E8 one request while pending; the password is concealed again after a refusal.
// - E9 a `mustChange` in an answer changes nothing (Identity owns passwords, design §5 B2).
// - E10 texts and error sentences follow setLang.
// The texts are asserted through ./i18n (whose Thai is pinned in i18n.test.ts).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError } from '../../shared/api/client';
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
interface NewAuthApi {
  session(): Promise<{ user: SessionUser | null }>;
  login(p: Creds): Promise<{ user: SessionUser }>;
}
type Screen = (props: { auth: NewAuthApi; navigate: (url: string) => void }) => JSX.Element;

const modules = import.meta.glob<{ LoginScreen?: unknown }>('./LoginScreen.tsx', { eager: true });
const TH = table.th as unknown as Record<string, string>;
const EN = table.en as unknown as Record<string, string>;

/** A text from the screen's table; fails (instead of comparing undefined) when the key is missing. */
function tx(key: string, t: Record<string, string> = TH): string {
  expect(t[key], `i18n key ${key}`).toBeTypeOf('string');
  return t[key]!;
}

const TELLER: SessionUser = { id: 'u-7', name: 'Nok', organizationId: 'org-1', roles: [{ role: 'TELLER', branchId: 'XC-B01' }] };
const RATES: SessionUser = { id: 'u-9', name: 'Ploy', organizationId: 'org-1', roles: [{ role: 'RATE_MANAGER', branchId: 'XC-B01' }] };
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
let fetched: string[];

beforeEach(() => {
  fetched = [];
  // AuthLayout's footer reads /health/ready; nothing else on this screen may call fetch.
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

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

const notExpected = (name: string) => async () => {
  throw new Error(`${name} was not expected in this test`);
};

/** A fake AuthApi: each call is recorded and answered by the function given. */
function fakeAuth(over: {
  session?: () => Promise<{ user: SessionUser | null }>;
  login?: (p: Creds) => Promise<{ user: SessionUser }>;
}) {
  const calls = { session: 0, login: [] as Creds[] };
  const session = over.session ?? (async () => ({ user: null }));
  const login = over.login ?? notExpected('login');
  const auth: NewAuthApi = {
    session: () => {
      calls.session += 1;
      return session();
    },
    login: (p) => {
      calls.login.push({ ...p });
      return login(p);
    },
  };
  return { auth, calls };
}

function mount(auth: NewAuthApi) {
  const LoginScreen = modules['./LoginScreen.tsx']?.LoginScreen;
  expect(LoginScreen, 'LoginScreen.tsx exports LoginScreen').toBeTypeOf('function');
  const host = document.createElement('div');
  document.body.appendChild(host);
  const navigate = vi.fn<(url: string) => void>();
  dispose = render(() => (LoginScreen as Screen)({ auth, navigate }), host);
  return { host, navigate };
}

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

function identifierInput(host: HTMLElement): HTMLInputElement {
  const el = host.querySelector<HTMLInputElement>('input[name="identifier"]');
  if (!el) throw new Error('no identifier input (name="identifier")');
  return el;
}

function passwordInput(host: HTMLElement): HTMLInputElement {
  const el = host.querySelector<HTMLInputElement>('input[autocomplete="current-password"]');
  if (!el) throw new Error('no password input (autocomplete="current-password")');
  return el;
}

function buttonWithText(host: HTMLElement, text: string): HTMLButtonElement {
  const el = Array.from(host.querySelectorAll('button')).find((b) => b.textContent?.trim() === text);
  if (!el) throw new Error(`no button with text ${JSON.stringify(text)}`);
  return el;
}

function submitButton(host: HTMLElement, t: Record<string, string> = TH): HTMLButtonElement {
  return buttonWithText(host, tx('submit', t));
}

function labelOf(host: HTMLElement, input: HTMLInputElement): string | undefined {
  return host.querySelector(`label[for="${input.id}"]`)?.textContent?.trim();
}

function messageLine(host: HTMLElement): string {
  const el = host.querySelector('#message');
  if (!el) throw new Error('no #message line');
  return (el.textContent ?? '').trim();
}

function type(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

async function signedOutScreen(over: Parameters<typeof fakeAuth>[0] = {}) {
  const fake = fakeAuth({ session: async () => ({ user: null }), ...over });
  const m = mount(fake.auth);
  await settle();
  return { ...fake, ...m };
}

async function fillAndSubmit(host: HTMLElement, identifier: string, password: string) {
  type(identifierInput(host), identifier);
  type(passwordInput(host), password);
  submitButton(host).click();
  await settle();
}

function refusal(code: string, status: number, message = 'SERVER-SENTENCE-MUST-NOT-SHOW') {
  return new ApiError({ code, message, status });
}

describe('LoginScreen — on load (E1, E2, E3)', () => {
  it.each([
    ['a TELLER', TELLER, '/desk'],
    ['the owner', OWNER, '/'],
    ['any other role', RATES, '/'],
  ] as Array<[string, SessionUser, string]>)('%s from session() goes to its home', async (_n, user, to) => {
    const { auth, calls } = fakeAuth({ session: async () => ({ user }) });
    const { navigate } = mount(auth);
    await settle();
    expect(calls.session).toBe(1);
    expect(navigate.mock.calls).toEqual([[to]]);
  });

  it('nobody signed in: the identifier form is shown and nothing navigates (E2)', async () => {
    const { host, navigate, calls } = await signedOutScreen();
    expect(calls.session).toBe(1);
    expect(navigate).not.toHaveBeenCalled();
    expect(host.querySelector('header h1')?.textContent?.trim()).toBe(tx('title'));

    const id = identifierInput(host);
    expect(id.type).toBe('text');
    expect(id.getAttribute('autocomplete')).toBe('username');
    expect(id.required).toBe(true);
    expect(id.hasAttribute('pattern'), 'Identity accepts e-mail, phone or username: no v2 username pattern').toBe(false);
    expect(id.getAttribute('maxlength')).toBe('254');
    expect(labelOf(host, id)).toBe(tx('identifier'));

    const pw = passwordInput(host);
    expect(pw.type).toBe('password');
    expect(pw.required).toBe(true);
    expect(pw.getAttribute('maxlength')).toBe('128');
    expect(labelOf(host, pw)).toBe(tx('password'));

    const button = submitButton(host);
    expect(button.type).toBe('submit');
    expect(button.disabled).toBe(false);
    expect(id.form).toBe(button.form);
    expect(pw.form).toBe(button.form);

    const submits = Array.from(host.querySelectorAll('form button[type="submit"]'));
    expect(submits, 'the form has one submit').toEqual([button]);
    const others = Array.from(host.querySelectorAll('form button:not([type="submit"])'));
    expect(
      others.every((b) => b.closest('label, .field, div')?.querySelector('input[type="password"], input[type="text"]')),
      'every other button belongs to a field (the password show/hide), none opens a code step',
    ).toBe(true);
    expect(host.textContent ?? '', 'no code step text').not.toMatch(/รหัสเชิญ|invite/i);
  });

  it('a refused session() shows the sentence for its code and the form stays usable (E3)', async () => {
    const { host, navigate, calls } = await signedOutScreen({
      session: async () => {
        throw refusal('identityUnavailable', 503);
      },
      login: async () => ({ user: TELLER }),
    });
    expect(messageLine(host)).toBe(tx('err_identityUnavailable'));
    expect(navigate).not.toHaveBeenCalled();
    expect(submitButton(host).disabled).toBe(false);

    await fillAndSubmit(host, 'nok', 'secret-pw-1');
    expect(calls.login).toEqual([{ identifier: 'nok', password: 'secret-pw-1' }]);
    expect(navigate.mock.calls).toEqual([['/desk']]);
  });

  it('a network failure of session() shows errNetwork, not the browser text (E3)', async () => {
    const { host, navigate } = await signedOutScreen({
      session: async () => {
        throw new TypeError('Failed to fetch');
      },
    });
    expect(messageLine(host)).toBe(tx('errNetwork'));
    expect(host.textContent).not.toContain('Failed to fetch');
    expect(navigate).not.toHaveBeenCalled();
  });
});

describe('LoginScreen — signing in (E4, E8, E9)', () => {
  it.each(['nok', 'nok@example.com', '+66812345678', '0812345678'])(
    'sends exactly {identifier: %j, password} once',
    async (identifier) => {
      const { host, navigate, calls } = await signedOutScreen({ login: async () => ({ user: TELLER }) });
      await fillAndSubmit(host, identifier, 'secret-pw-1');
      expect(calls.login).toEqual([{ identifier, password: 'secret-pw-1' }]);
      expect(navigate.mock.calls).toEqual([['/desk']]);
    },
  );

  it.each([
    ['a TELLER', TELLER, '/desk'],
    ['the owner', OWNER, '/'],
    ['a RATE_MANAGER', RATES, '/'],
  ] as Array<[string, SessionUser, string]>)('%s goes to its home after login', async (_n, user, to) => {
    const { host, navigate } = await signedOutScreen({ login: async () => ({ user }) });
    await fillAndSubmit(host, 'someone', 'secret-pw-1');
    expect(navigate.mock.calls).toEqual([[to]]);
  });

  it('a mustChange in the answer changes nothing: no /staff detour (E9)', async () => {
    const { host, navigate } = await signedOutScreen({
      login: async () => ({ user: TELLER, mustChange: true }) as unknown as { user: SessionUser },
    });
    await fillAndSubmit(host, 'nok', 'secret-pw-1');
    expect(navigate.mock.calls).toEqual([['/desk']]);
  });

  it('the button is disabled while a login is pending, and a second press sends nothing (E8)', async () => {
    const pending = deferred<{ user: SessionUser }>();
    const { host, navigate, calls } = await signedOutScreen({ login: () => pending.promise });
    await fillAndSubmit(host, 'nok', 'secret-pw-1');
    const button = submitButton(host);
    expect(button.disabled).toBe(true);
    button.click();
    await settle();
    expect(calls.login).toHaveLength(1);
    pending.resolve({ user: TELLER });
    await settle();
    expect(calls.login).toHaveLength(1);
    expect(navigate.mock.calls).toEqual([['/desk']]);
  });

  it('after a refusal the button is enabled again, the password concealed, nothing navigates (E8)', async () => {
    const { host, navigate, calls } = await signedOutScreen({
      login: async () => {
        throw refusal('invalidCredentials', 401);
      },
    });
    const pw = passwordInput(host);
    type(identifierInput(host), 'nok');
    type(pw, 'wrong-pw-1');
    pw.parentElement!.querySelector('button')!.click();
    expect(pw.type).toBe('text');
    submitButton(host).click();
    await settle();
    expect(calls.login).toHaveLength(1);
    expect(submitButton(host).disabled).toBe(false);
    expect(pw.type).toBe('password');
    expect(navigate).not.toHaveBeenCalled();
  });

  it('an empty identifier sends nothing; the field is marked and focused', async () => {
    const { host, calls } = await signedOutScreen({ login: async () => ({ user: TELLER }) });
    const id = identifierInput(host);
    type(passwordInput(host), 'secret-pw-1');
    submitButton(host).form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    expect(calls.login).toHaveLength(0);
    expect(id.getAttribute('aria-invalid')).toBe('true');
    expect(document.activeElement).toBe(id);
  });

  it('makes no request of its own: no fetch outside /health/ready and the shell me read', async () => {
    const { host } = await signedOutScreen({ login: async () => ({ user: TELLER }) });
    await fillAndSubmit(host, 'nok', 'secret-pw-1');
    const screenUrls = fetched.filter((url) => !/\/api\/v3\/views\/me(\?|$)/.test(url));
    for (const url of screenUrls) expect(url).toMatch(/\/health\/ready$/);
  });
});

describe('LoginScreen — refusals by code (E7)', () => {
  it.each([
    ['invalidCredentials', 401],
    ['accountLocked', 423],
    ['tooManyAttempts', 429],
    ['identityDisabled', 403],
    ['userDisabled', 403],
    ['identityUnavailable', 503],
    ['invalidRequest', 400],
  ])('code %s (HTTP %i) shows its own sentence, never the server message', async (code, status) => {
    const { host, navigate } = await signedOutScreen({
      login: async () => {
        throw refusal(code, status);
      },
    });
    await fillAndSubmit(host, 'nok', 'secret-pw-1');
    expect(messageLine(host)).toBe(tx(`err_${code}`));
    expect(host.textContent).not.toContain('SERVER-SENTENCE-MUST-NOT-SHOW');
    expect(navigate).not.toHaveBeenCalled();
  });

  it('chooses by code, never by HTTP status: same status, different codes → different sentences', async () => {
    const shown: string[] = [];
    for (const [code, status] of [
      ['invalidCredentials', 503],
      ['identityUnavailable', 401],
      ['userDisabled', 401],
    ] as Array<[string, number]>) {
      const { host } = await signedOutScreen({
        login: async () => {
          throw refusal(code, status);
        },
      });
      await fillAndSubmit(host, 'nok', 'secret-pw-1');
      shown.push(messageLine(host));
      dispose?.();
      dispose = undefined;
      document.body.innerHTML = '';
    }
    expect(shown).toEqual([tx('err_invalidCredentials'), tx('err_identityUnavailable'), tx('err_userDisabled')]);
  });

  it('an unknown code shows errUnknown, not the server message', async () => {
    const { host } = await signedOutScreen({
      login: async () => {
        throw refusal('somethingNew', 500, 'raw server text');
      },
    });
    await fillAndSubmit(host, 'nok', 'secret-pw-1');
    expect(messageLine(host)).toBe(tx('errUnknown'));
    expect(host.textContent).not.toContain('raw server text');
  });

  it('a network failure on submit shows errNetwork and the button is enabled again', async () => {
    const { host } = await signedOutScreen({
      login: async () => {
        throw new TypeError('Failed to fetch');
      },
    });
    await fillAndSubmit(host, 'nok', 'secret-pw-1');
    expect(messageLine(host)).toBe(tx('errNetwork'));
    expect(submitButton(host).disabled).toBe(false);
  });
});

describe('LoginScreen — language (E10, W10)', () => {
  it('its texts and its error sentences follow setLang', async () => {
    setLang('en');
    const { host } = await signedOutScreen({
      login: async () => {
        throw refusal('invalidCredentials', 401);
      },
    });
    expect(host.querySelector('header h1')?.textContent?.trim()).toBe(tx('title', EN));
    expect(labelOf(host, identifierInput(host))).toBe(tx('identifier', EN));
    expect(labelOf(host, passwordInput(host))).toBe(tx('password', EN));
    type(identifierInput(host), 'nok');
    type(passwordInput(host), 'wrong-pw-1');
    submitButton(host, EN).click();
    await settle();
    expect(messageLine(host)).toBe(tx('err_invalidCredentials', EN));

    setLang('th');
    await settle();
    expect(host.querySelector('header h1')?.textContent?.trim()).toBe(tx('title'));
    expect(messageLine(host), 'the shown refusal switches language with the screen').toBe(tx('err_invalidCredentials'));
  });
});
