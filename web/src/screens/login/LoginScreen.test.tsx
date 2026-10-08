// V1-login (#396 §7 `LoginScreen.tsx` [required], §9 edge cases, §10.3 R1–R7): v3's own sign-in
// screen, ported from xchange-v2-source@4d0bdbb:src/devices-public/login.html (lines 1-4).
// Written before the implementation by the lane controller. Do not edit, delete or weaken.
//
// The API this test pins, in `web/src/screens/login/LoginScreen.tsx`:
//
//   export function LoginScreen(props: { auth: AuthApi; navigate: (url: string) => void }): JSX.Element;
//
// What it pins, row by row:
// - L7: on load `status`; a user → navigate(home(user)), FRONT → /desk, everyone else → /.
// - L1/L2/L4–L6 + R6: inside AppShell titled `th.title`; a username field (required,
//   pattern `[a-zA-Z0-9._\-]{3,40}`, autocomplete username), a password field (type password,
//   required, maxlength 128, autocomplete current-password), a submit button `th.submit`.
// - R7: no setup mode — no name row, `setupRequired` in a status answer changes nothing.
// - L9: a status failure shows its text in a role="alert" line; the form stays usable.
// - L10: submit → `login({username, password})`; mustChange → /staff, else home(user); a refusal
//   shows the sentence verbatim in the alert line and enables the button again; one request while
//   pending (the button is disabled).
// - R3: the password is V0-A's PasswordField, concealed on submit.
// - R6 (#401 F1–F3): a field that breaks its rule sends nothing and is marked and focused.
// - W10: every text comes from the screen's own table and follows setLang.
// The texts are asserted through `./i18n` (whose Thai is pinned in i18n.test.ts).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import type { AuthApi, AuthUser } from './auth-api';
import { en, th } from './i18n';
import { LoginScreen } from './LoginScreen';

type StatusAnswer = { user: AuthUser | null };
type LoginAnswer = { user: AuthUser; mustChange?: boolean };
type Creds = { username: string; password: string };

const OWNER: AuthUser = { id: 'owner', username: 'owner1', name: 'Somchai', role: 'OWNER', branchId: null };
const FRONT: AuthUser = { id: 'employee-7', username: 'nok', name: 'Nok', role: 'FRONT', branchId: 'XC-B01' };
const RATES: AuthUser = { id: 'employee-9', username: 'ploy', name: 'Ploy', role: 'RATES', branchId: 'XC-B01' };

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

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

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

function labelOf(host: HTMLElement, input: HTMLInputElement): string | undefined {
  return host.querySelector(`label[for="${input.id}"]`)?.textContent?.trim();
}

function alertTexts(host: HTMLElement): string[] {
  return Array.from(host.querySelectorAll('[role="alert"]')).map((e) => (e.textContent ?? '').trim());
}

function type(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

async function signedOutScreen(login?: (p: Creds) => Promise<LoginAnswer>) {
  const fake = fakeAuth(async () => ({ user: null }), login);
  const m = mount(fake.auth);
  await settle();
  return { ...fake, ...m };
}

describe('LoginScreen — on load (#396 L7, §9)', () => {
  it('a FRONT user from status goes to /desk', async () => {
    const { auth, calls } = fakeAuth(async () => ({ user: FRONT }));
    const { navigate } = mount(auth);
    await settle();
    expect(calls.status).toBe(1);
    expect(navigate.mock.calls).toEqual([['/desk']]);
  });

  it('the owner from status goes to /', async () => {
    const { auth } = fakeAuth(async () => ({ user: OWNER }));
    const { navigate } = mount(auth);
    await settle();
    expect(navigate.mock.calls).toEqual([['/']]);
  });

  it('any other staff role from status goes to /', async () => {
    const { auth } = fakeAuth(async () => ({ user: RATES }));
    const { navigate } = mount(auth);
    await settle();
    expect(navigate.mock.calls).toEqual([['/']]);
  });

  it("nobody signed in: v2's form is shown inside AppShell and nothing navigates (L1, L2, L4–L6, R6)", async () => {
    const { host, navigate, calls } = await signedOutScreen();
    expect(calls.status).toBe(1);
    expect(navigate).not.toHaveBeenCalled();
    expect(host.querySelector('header h1')?.textContent?.trim()).toBe(th.title);
    expect(host.textContent).toContain('XCHANGE × T.K.');

    const user = usernameInput(host);
    expect(user.type).toBe('text');
    expect(user.required).toBe(true);
    expect(user.getAttribute('pattern')).toBe('[a-zA-Z0-9._\\-]{3,40}');
    expect(labelOf(host, user)).toBe(th.username);

    const pw = passwordInput(host);
    expect(pw.type).toBe('password');
    expect(pw.required).toBe(true);
    expect(pw.getAttribute('maxlength')).toBe('128');
    expect(labelOf(host, pw)).toBe(th.password);

    const button = submitButton(host);
    expect(button.type).toBe('submit');
    expect(button.disabled).toBe(false);
    expect(button.form).not.toBeNull();
    expect(user.form).toBe(button.form);
    expect(pw.form).toBe(button.form);
  });

  it('has no setup mode: no name row, and setupRequired in a status answer changes nothing (R7)', async () => {
    const answer = { setupRequired: true, user: null } as unknown as StatusAnswer;
    const { auth } = fakeAuth(async () => answer);
    const { host, navigate } = mount(auth);
    await settle();
    expect(navigate).not.toHaveBeenCalled();
    const form = submitButton(host).form!;
    expect(form.querySelectorAll('input')).toHaveLength(2);
    expect(host.querySelector('header h1')?.textContent?.trim()).toBe(th.title);
    expect(host.textContent).not.toContain('สร้างบัญชีเจ้าของ');
    expect(host.textContent).not.toContain('ชื่อเจ้าของระบบ');
    const pw = passwordInput(host);
    expect(pw.hasAttribute('minlength')).toBe(false);
  });

  it('a refused status shows its sentence in the alert line and the form stays usable (L9)', async () => {
    const { auth, calls } = fakeAuth(
      async () => {
        throw new ApiError({ code: 'refused', message: 'เซิร์ฟเวอร์ขัดข้อง', status: 500 });
      },
      async () => ({ user: FRONT, mustChange: false }),
    );
    const { host, navigate } = mount(auth);
    await settle();
    expect(alertTexts(host)).toContain('เซิร์ฟเวอร์ขัดข้อง');
    expect(navigate).not.toHaveBeenCalled();
    const button = submitButton(host);
    expect(button.disabled).toBe(false);

    type(usernameInput(host), 'nok');
    type(passwordInput(host), 'secret-pw-1');
    button.click();
    await settle();
    expect(calls.login).toEqual([{ username: 'nok', password: 'secret-pw-1' }]);
    expect(navigate.mock.calls).toEqual([['/desk']]);
  });

  it('a network failure of status shows the error text in the alert line (L9)', async () => {
    const { auth } = fakeAuth(async () => {
      throw new TypeError('Failed to fetch');
    });
    const { host, navigate } = mount(auth);
    await settle();
    expect(alertTexts(host)).toContain('Failed to fetch');
    expect(navigate).not.toHaveBeenCalled();
    expect(submitButton(host).disabled).toBe(false);
  });
});

describe('LoginScreen — signing in (#396 L10, §9)', () => {
  it('sends exactly {username, password} once, and mustChange goes to /staff whatever the role', async () => {
    const { host, navigate, calls } = await signedOutScreen(async () => ({ user: RATES, mustChange: true }));
    type(usernameInput(host), 'ploy');
    type(passwordInput(host), 'secret-pw-1');
    submitButton(host).click();
    await settle();
    expect(calls.login).toEqual([{ username: 'ploy', password: 'secret-pw-1' }]);
    expect(navigate.mock.calls).toEqual([['/staff']]);
  });

  it.each([
    ['FRONT, mustChange false', '/desk', { user: FRONT, mustChange: false }],
    ['RATES, mustChange false', '/', { user: RATES, mustChange: false }],
    ['the owner, no mustChange', '/', { user: OWNER }],
  ] as Array<[string, string, LoginAnswer]>)('%s goes home to %s', async (_name, to, answer) => {
    const { host, navigate } = await signedOutScreen(async () => answer);
    type(usernameInput(host), answer.user.username);
    type(passwordInput(host), 'secret-pw-1');
    submitButton(host).click();
    await settle();
    expect(navigate.mock.calls).toEqual([[to]]);
  });

  it('a refusal is shown verbatim in the alert line, the button is enabled again, the password concealed, nothing navigates', async () => {
    const { host, navigate, calls } = await signedOutScreen(async () => {
      throw new ApiError({ code: 'refused', message: 'ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง', status: 400 });
    });
    const pw = passwordInput(host);
    type(usernameInput(host), 'nok');
    type(pw, 'wrong-pw-1');
    const eye = pw.parentElement!.querySelector('button')!;
    eye.click();
    expect(pw.type).toBe('text');

    const button = submitButton(host);
    button.click();
    await settle();
    expect(calls.login).toHaveLength(1);
    expect(alertTexts(host)).toContain('ชื่อบัญชีหรือรหัสผ่านไม่ถูกต้อง');
    expect(button.disabled).toBe(false);
    expect(pw.type).toBe('password');
    expect(navigate).not.toHaveBeenCalled();
  });

  it('the button is disabled while a login is pending, and a second press sends nothing', async () => {
    const pending = deferred<LoginAnswer>();
    const { host, navigate, calls } = await signedOutScreen(() => pending.promise);
    type(usernameInput(host), 'nok');
    type(passwordInput(host), 'secret-pw-1');
    const button = submitButton(host);
    button.click();
    await settle();
    expect(button.disabled).toBe(true);
    button.click();
    await settle();
    expect(calls.login).toHaveLength(1);

    pending.resolve({ user: FRONT, mustChange: false });
    await settle();
    expect(calls.login).toHaveLength(1);
    expect(navigate.mock.calls).toEqual([['/desk']]);
  });

  it('a username that breaks the rule sends nothing; the field is marked and focused (R6, #401)', async () => {
    const { host, calls } = await signedOutScreen(async () => ({ user: FRONT, mustChange: false }));
    const user = usernameInput(host);
    type(user, 'ab');
    type(passwordInput(host), 'secret-pw-1');
    submitButton(host).form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    expect(calls.login).toHaveLength(0);
    expect(user.getAttribute('aria-invalid')).toBe('true');
    expect(document.activeElement).toBe(user);
  });

  it('an empty password sends nothing; the field is marked and focused (R6, #401)', async () => {
    const { host, calls } = await signedOutScreen(async () => ({ user: FRONT, mustChange: false }));
    const pw = passwordInput(host);
    type(usernameInput(host), 'nok');
    submitButton(host).form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    expect(calls.login).toHaveLength(0);
    expect(pw.getAttribute('aria-invalid')).toBe('true');
    expect(document.activeElement).toBe(pw);
  });

  it('makes no request of its own: the only fetch is AppShell /health/ready', async () => {
    const { host } = await signedOutScreen(async () => ({ user: FRONT, mustChange: false }));
    type(usernameInput(host), 'nok');
    type(passwordInput(host), 'secret-pw-1');
    submitButton(host).click();
    await settle();
    expect(fetched.length).toBeGreaterThan(0);
    for (const url of fetched) expect(url).toMatch(/\/health\/ready$/);
  });
});

describe('LoginScreen — language (W10)', () => {
  it('its texts follow setLang', async () => {
    setLang('en');
    const { host } = await signedOutScreen();
    expect(host.querySelector('header h1')?.textContent?.trim()).toBe(en.title);
    expect(labelOf(host, usernameInput(host))).toBe(en.username);
    expect(labelOf(host, passwordInput(host))).toBe(en.password);
    const button = submitButton(host, en.submit);
    expect(button.type).toBe('submit');

    setLang('th');
    await settle();
    expect(host.querySelector('header h1')?.textContent?.trim()).toBe(th.title);
    expect(submitButton(host, th.submit).type).toBe('submit');
  });
});
