// #763 (login follow-up of #762): in the invite step, "no account yet? create one" registers an Identity account
// and then claims the invite; a refused claim shows the sentence for its `reason`.
// Decided (lane controller, 2026-10-10, on #763): registration lives inside the invite step only; refusals are
// told apart by code, never by the server's message.
// #762's LoginScreen.test.tsx stays as it is and must stay green.
// Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken.
//
// What this test pins, in `web/src/screens/login/LoginScreen.tsx` (props unchanged: { auth, navigate }):
// - R1 a `createAccount` button (type=button) appears only in the invite step.
// - R2 pressing it shows a phone field (name phone, required), an optional e-mail field (name email, type email,
//      not required), keeps the password and invite-code fields, and asks for no identifier (gone, disabled or not
//      required): the phone is the identifier the claim signs in with.
// - R3 submit → `register({phone, password, email?})` once, and only after it resolves `claim({identifier: phone,
//      password, inviteCode})` once (the BFF's register answers 201 with no cookie; the claim signs in itself,
//      api/src/presentation/http/auth.rs:238-285 @ abb7ea1) → navigate(homeOf(user)).
// - R4 an empty e-mail is not passed on (register gets no e-mail, or an empty one that auth-api drops).
// - R5 register refused → no claim; the sentence for its code from ./i18n.
// - R5a register refused `alreadyRegistered` (lane controller's ruling on #763, 2026-10-10): the screen's own
//      sentence for that code (never the server message), and the screen returns to the invite step's sign-in path:
//      create mode closes, the identifier field is back (required, enabled) and prefilled with the phone, the invite
//      code is kept; the next submit claims with {identifier, password, inviteCode} and never registers again.
//      (Trap: useForm keeps validating unmounted inputs, so a required phone removed by <Show> blocks that submit.)
// - R6 register succeeded but the claim failed → the next submit claims again without registering again.
// - R7 a refused claim (after register, or a plain claim) shows `err_inviteRefused_<reason>` from ./i18n, chosen by
//      `ApiError.reason`, never by its message; an unknown or absent reason → `err_inviteRefused`; a reason on any
//      other code is ignored. TH and EN.
// - R8 end to end over the real ApiClient: session → register → invite/claim, and the reason in the JSON reaches
//      the screen.
// - R9 no web source (outside i18n tables and tests) holds any of user_service.rs's refusal sentences.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient, ApiError } from '../../shared/api/client';
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
interface ClaimCreds extends Creds {
  inviteCode: string;
}
interface RegisterParams {
  phone: string;
  password: string;
  email?: string;
}
interface RegisterAuthApi {
  session(): Promise<{ user: SessionUser | null }>;
  login(p: Creds): Promise<{ user: SessionUser }>;
  claim(p: ClaimCreds): Promise<{ user: SessionUser }>;
  register(p: RegisterParams): Promise<unknown>;
}
type Screen = (props: { auth: RegisterAuthApi; navigate: (url: string) => void }) => JSX.Element;
type AuthApiFn = (api: Pick<ApiClient, 'post'>) => RegisterAuthApi;

const screens = import.meta.glob<{ LoginScreen?: unknown }>('./LoginScreen.tsx', { eager: true });
const apis = import.meta.glob<{ authApi?: unknown }>('./auth-api.ts', { eager: true });
const TH = table.th as unknown as Record<string, string>;
const EN = table.en as unknown as Record<string, string>;

const REASONS = ['inviteInvalid', 'alreadyLinked', 'userNotFound', 'userDisabled'] as const;
/** user_service.rs's refusal sentences (@ abb7ea1 :141, :149, :155, :158), one per reason above. */
const SERVER_SENTENCE: Record<(typeof REASONS)[number], string> = {
  inviteInvalid: 'รหัสเชิญไม่ถูกต้อง ถูกใช้งานแล้ว หรือหมดอายุแล้ว',
  alreadyLinked: 'ผู้ใช้นี้ลงทะเบียนแล้ว',
  userNotFound: 'ไม่พบผู้ใช้งาน',
  userDisabled: 'บัญชีถูกระงับ',
};

function tx(key: string, t: Record<string, string> = TH): string {
  expect(t[key], `i18n key ${key}`).toBeTypeOf('string');
  return t[key]!;
}

const TELLER: SessionUser = { id: 'u-7', name: 'Nok', organizationId: 'org-1', roles: [{ role: 'TELLER', branchId: 'XC-B01' }] };
const RATES: SessionUser = { id: 'u-9', name: 'Ploy', organizationId: 'org-1', roles: [{ role: 'RATE_MANAGER', branchId: 'XC-B01' }] };

let dispose: (() => void) | undefined;

beforeEach(() => {
  // AuthLayout's footer reads /health/ready; the screen's own calls go through the AuthApi given.
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
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

const notExpected = (name: string) => async () => {
  throw new Error(`${name} was not expected in this test`);
};

function fakeAuth(over: {
  claim?: (p: ClaimCreds) => Promise<{ user: SessionUser }>;
  register?: (p: RegisterParams) => Promise<unknown>;
}) {
  const calls = { order: [] as string[], login: [] as Creds[], claim: [] as ClaimCreds[], register: [] as RegisterParams[] };
  const claim = over.claim ?? notExpected('claim');
  const register = over.register ?? notExpected('register');
  const auth: RegisterAuthApi = {
    session: async () => ({ user: null }),
    login: (p) => {
      calls.login.push({ ...p });
      return notExpected('login')();
    },
    claim: (p) => {
      calls.order.push('claim');
      calls.claim.push({ ...p });
      return claim(p);
    },
    register: (p) => {
      calls.order.push('register');
      calls.register.push({ ...p });
      return register(p);
    },
  };
  return { auth, calls };
}

function mount(auth: RegisterAuthApi) {
  const LoginScreen = screens['./LoginScreen.tsx']?.LoginScreen;
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

function input(host: HTMLElement, name: string): HTMLInputElement | null {
  return host.querySelector<HTMLInputElement>(`input[name="${name}"]`);
}

function must(host: HTMLElement, name: string): HTMLInputElement {
  const el = input(host, name);
  if (!el) throw new Error(`no input name="${name}"`);
  return el;
}

function buttonWithText(host: HTMLElement, text: string): HTMLButtonElement | undefined {
  return Array.from(host.querySelectorAll('button')).find((b) => b.textContent?.trim() === text);
}

function mustButton(host: HTMLElement, text: string): HTMLButtonElement {
  const el = buttonWithText(host, text);
  if (!el) throw new Error(`no button with text ${JSON.stringify(text)}`);
  return el;
}

function labelOf(host: HTMLElement, el: HTMLInputElement): string | undefined {
  return host.querySelector(`label[for="${el.id}"]`)?.textContent?.trim();
}

function messageLine(host: HTMLElement): string {
  const el = host.querySelector('#message');
  if (!el) throw new Error('no #message line');
  return (el.textContent ?? '').trim();
}

function type(el: HTMLInputElement, value: string) {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

function refusal(code: string, status: number, message = 'SERVER-SENTENCE-MUST-NOT-SHOW', reason?: string): ApiError {
  const err = new ApiError({ code, message, status });
  return reason === undefined ? err : Object.assign(err, { reason });
}

/** The screen in its invite step with "create one" open. */
async function createStep(over: Parameters<typeof fakeAuth>[0], t: Record<string, string> = TH) {
  const fake = fakeAuth(over);
  const m = mount(fake.auth);
  await settle();
  mustButton(m.host, tx('haveInvite', t)).click();
  await settle();
  mustButton(m.host, tx('createAccount', t)).click();
  await settle();
  return { ...fake, ...m };
}

async function fillCreate(
  host: HTMLElement,
  v: { phone?: string; password?: string; email?: string; inviteCode?: string } = {},
  t: Record<string, string> = TH,
) {
  type(must(host, 'phone'), v.phone ?? '0812345678');
  type(must(host, 'password'), v.password ?? 'secret-pw-1');
  if (v.email !== undefined) type(must(host, 'email'), v.email);
  type(must(host, 'inviteCode'), v.inviteCode ?? 'INV-7Q2K');
  mustButton(host, tx('submit', t)).click();
  await settle();
}

describe('LoginScreen — create an account from the invite step (R1, R2)', () => {
  it('createAccount is offered only in the invite step, as a button that never submits', async () => {
    const { host } = mount(fakeAuth({}).auth);
    await settle();
    expect(buttonWithText(host, tx('createAccount')), 'not before the invite step').toBeUndefined();
    mustButton(host, tx('haveInvite')).click();
    await settle();
    const create = mustButton(host, tx('createAccount'));
    expect(create.type).toBe('button');
    expect(input(host, 'phone'), 'no phone field until asked for').toBeNull();
  });

  it('shows phone (required) and an optional e-mail; keeps password and invite code; hides the identifier', async () => {
    const { host } = await createStep({});
    const phone = must(host, 'phone');
    expect(phone.required).toBe(true);
    expect(labelOf(host, phone)).toBe(tx('phone'));
    const email = must(host, 'email');
    expect(email.type).toBe('email');
    expect(email.required).toBe(false);
    expect(labelOf(host, email)).toBe(tx('email'));
    expect(must(host, 'password').required).toBe(true);
    expect(must(host, 'inviteCode').required).toBe(true);
    // The phone is the identifier the claim signs in with, so no identifier is left for the person to fill:
    // the field is gone, disabled, or no longer required. (Trap: useForm keeps validating a field whose input
    // was unmounted, web/src/shared/ui/form.ts `fields` keeps `element`; a required input removed by <Show>
    // still blocks submit.)
    const id = input(host, 'identifier');
    expect(id === null || id.disabled || !id.required, 'no required identifier in create mode').toBe(true);
    const submit = mustButton(host, tx('submit'));
    for (const el of [phone, email, must(host, 'password'), must(host, 'inviteCode')]) expect(el.form).toBe(submit.form);
  });
});

describe('LoginScreen — register then claim (R3, R4)', () => {
  it('registers, then claims with the phone as identifier, then goes home', async () => {
    const { host, calls, navigate } = await createStep({
      register: async () => ({ registered: true }),
      claim: async () => ({ user: TELLER }),
    });
    await fillCreate(host, { phone: '0812345678', password: 'secret-pw-1', email: 'nok@example.com', inviteCode: 'INV-7Q2K' });
    expect(calls.order).toEqual(['register', 'claim']);
    expect(calls.register).toEqual([{ phone: '0812345678', password: 'secret-pw-1', email: 'nok@example.com' }]);
    expect(calls.claim).toEqual([{ identifier: '0812345678', password: 'secret-pw-1', inviteCode: 'INV-7Q2K' }]);
    expect(calls.login).toEqual([]);
    expect(navigate.mock.calls).toEqual([['/desk']]);
  });

  it('the claim waits for register to answer', async () => {
    const reg = deferred<unknown>();
    const { host, calls, navigate } = await createStep({ register: () => reg.promise, claim: async () => ({ user: RATES }) });
    await fillCreate(host);
    expect(calls.register).toHaveLength(1);
    expect(calls.claim, 'no claim while register is pending').toHaveLength(0);
    expect(mustButton(host, tx('submit')).disabled).toBe(true);
    reg.resolve({ registered: true });
    await settle();
    expect(calls.claim).toHaveLength(1);
    expect(navigate.mock.calls).toEqual([['/']]);
  });

  it('an empty e-mail is not passed on', async () => {
    const { host, calls } = await createStep({
      register: async () => ({ registered: true }),
      claim: async () => ({ user: TELLER }),
    });
    await fillCreate(host, { email: '' });
    expect(calls.register).toHaveLength(1);
    const sent = calls.register[0]!;
    expect({ phone: sent.phone, password: sent.password }).toEqual({ phone: '0812345678', password: 'secret-pw-1' });
    expect(sent.email ?? '', 'no e-mail, or an empty one that auth-api drops').toBe('');
  });
});

describe('LoginScreen — a refused register (R5)', () => {
  it.each([
    ['invalidPhone', 422],
    ['weakPassword', 422],
    ['phoneRequired', 422],
    ['identityUnavailable', 503],
  ])('%s (HTTP %i): no claim is sent and its own sentence shows', async (code, status) => {
    const { host, calls, navigate } = await createStep({
      register: async () => {
        throw refusal(code, status);
      },
      claim: async () => ({ user: TELLER }),
    });
    await fillCreate(host);
    expect(calls.register).toHaveLength(1);
    expect(calls.claim, 'no claim after a refused register').toEqual([]);
    expect(messageLine(host)).toBe(tx(`err_${code}`));
    expect(host.textContent).not.toContain('SERVER-SENTENCE-MUST-NOT-SHOW');
    expect(navigate).not.toHaveBeenCalled();
    expect(mustButton(host, tx('submit')).disabled).toBe(false);
  });

  it('a network failure of register sends no claim and shows errNetwork', async () => {
    const { host, calls } = await createStep({
      register: async () => {
        throw new TypeError('Failed to fetch');
      },
      claim: async () => ({ user: TELLER }),
    });
    await fillCreate(host);
    expect(calls.claim).toEqual([]);
    expect(messageLine(host)).toBe(tx('errNetwork'));
  });
});

/** The BFF's sentence for alreadyRegistered (identity_client.rs:95). The screen's own sentence must show instead. */
const BFF_ALREADY_REGISTERED = 'มีบัญชีนี้อยู่แล้ว กรุณาเข้าสู่ระบบ';

describe('LoginScreen — the phone already has an account (R5a)', () => {
  it('shows the screen\'s own sentence for alreadyRegistered, sends no claim, and returns to sign-in-then-claim', async () => {
    const { host, calls, navigate } = await createStep({
      register: async () => {
        throw refusal('alreadyRegistered', 409, BFF_ALREADY_REGISTERED);
      },
      claim: async () => ({ user: TELLER }),
    });
    await fillCreate(host, { phone: '0812345678', password: 'secret-pw-1', inviteCode: 'INV-7Q2K' });

    expect(calls.register).toHaveLength(1);
    expect(calls.claim, 'no claim after a refused register').toEqual([]);
    expect(messageLine(host)).toBe(tx('err_alreadyRegistered'));
    expect(host.textContent, 'the server sentence is never echoed').not.toContain(BFF_ALREADY_REGISTERED);
    expect(navigate).not.toHaveBeenCalled();

    const id = must(host, 'identifier');
    expect(id.disabled).toBe(false);
    expect(id.required, 'back on the sign-in path: the identifier is asked for').toBe(true);
    expect(id.value, 'prefilled with the phone typed').toBe('0812345678');
    expect(must(host, 'inviteCode').value, 'the invite code is kept').toBe('INV-7Q2K');
    expect(input(host, 'phone'), 'create mode is closed').toBeNull();

    type(must(host, 'password'), 'secret-pw-1');
    mustButton(host, tx('submit')).click();
    await settle();
    expect(calls.order, 'signs in and claims, never registers again').toEqual(['register', 'claim']);
    expect(calls.claim).toEqual([{ identifier: '0812345678', password: 'secret-pw-1', inviteCode: 'INV-7Q2K' }]);
    expect(calls.login).toEqual([]);
    expect(navigate.mock.calls).toEqual([['/desk']]);
  });

  it('in English too, from the code alone', async () => {
    setLang('en');
    const { host } = await createStep(
      {
        register: async () => {
          throw refusal('alreadyRegistered', 409, 'This account already exists. Please sign in.');
        },
      },
      EN,
    );
    await fillCreate(host, {}, EN);
    expect(messageLine(host)).toBe(tx('err_alreadyRegistered', EN));
    expect(host.textContent).not.toContain('This account already exists. Please sign in.');
  });

  it('the code decides, not the status: alreadyRegistered on a 400 still returns to sign-in', async () => {
    const { host, calls } = await createStep({
      register: async () => {
        throw refusal('alreadyRegistered', 400);
      },
    });
    await fillCreate(host);
    expect(calls.claim).toEqual([]);
    expect(messageLine(host)).toBe(tx('err_alreadyRegistered'));
    expect(must(host, 'identifier').value).toBe('0812345678');
  });
});

describe('LoginScreen — a refused claim after register (R6, R7)', () => {
  it.each(REASONS)('reason %s shows its own sentence, not the server message', async (reason) => {
    // The server message is another reason's sentence, so a screen that read the message would show the wrong one.
    const other = REASONS[(REASONS.indexOf(reason) + 1) % REASONS.length]!;
    const { host, navigate } = await createStep({
      register: async () => ({ registered: true }),
      claim: async () => {
        throw refusal('inviteRefused', 400, SERVER_SENTENCE[other], reason);
      },
    });
    await fillCreate(host);
    expect(messageLine(host)).toBe(tx(`err_inviteRefused_${reason}`));
    expect(navigate).not.toHaveBeenCalled();
  });

  it.each(REASONS)('reason %s in English', async (reason) => {
    setLang('en');
    const { host } = await createStep(
      {
        register: async () => ({ registered: true }),
        claim: async () => {
          throw refusal('inviteRefused', 400, SERVER_SENTENCE[reason], reason);
        },
      },
      EN,
    );
    await fillCreate(host, {}, EN);
    expect(messageLine(host)).toBe(tx(`err_inviteRefused_${reason}`, EN));
  });

  it.each([
    ['an unknown reason', 'somethingNew'],
    ['no reason', undefined],
  ])('%s shows the generic err_inviteRefused', async (_n, reason) => {
    const { host } = await createStep({
      register: async () => ({ registered: true }),
      claim: async () => {
        throw refusal('inviteRefused', 400, 'raw server text', reason);
      },
    });
    await fillCreate(host);
    expect(messageLine(host)).toBe(tx('err_inviteRefused'));
    expect(host.textContent).not.toContain('raw server text');
  });

  it('a reason on another code is ignored: the code chooses', async () => {
    const { host } = await createStep({
      register: async () => ({ registered: true }),
      claim: async () => {
        throw refusal('invalidCredentials', 401, 'x', 'userDisabled');
      },
    });
    await fillCreate(host);
    expect(messageLine(host)).toBe(tx('err_invalidCredentials'));
  });

  it('register succeeded but the claim failed: the next submit claims again without registering again', async () => {
    let first = true;
    const { host, calls, navigate } = await createStep({
      register: async () => ({ registered: true }),
      claim: async () => {
        if (first) {
          first = false;
          throw refusal('identityUnavailable', 503);
        }
        return { user: TELLER };
      },
    });
    await fillCreate(host);
    expect(messageLine(host)).toBe(tx('err_identityUnavailable'));
    expect(calls.order).toEqual(['register', 'claim']);

    mustButton(host, tx('submit')).click();
    await settle();
    expect(calls.order, 'the retry only claims').toEqual(['register', 'claim', 'claim']);
    expect(calls.register).toHaveLength(1);
    expect(navigate.mock.calls).toEqual([['/desk']]);
  });
});

describe('LoginScreen — a plain claim refused with a reason (R7)', () => {
  it.each(REASONS)('reason %s from a claim without register shows its own sentence', async (reason) => {
    const fake = fakeAuth({
      claim: async () => {
        throw refusal('inviteRefused', 400, 'SERVER-SENTENCE-MUST-NOT-SHOW', reason);
      },
    });
    const { host } = mount(fake.auth);
    await settle();
    mustButton(host, tx('haveInvite')).click();
    await settle();
    type(must(host, 'identifier'), 'nok');
    type(must(host, 'password'), 'secret-pw-1');
    type(must(host, 'inviteCode'), 'INV-1');
    mustButton(host, tx('submit')).click();
    await settle();
    expect(fake.calls.register).toEqual([]);
    expect(messageLine(host)).toBe(tx(`err_inviteRefused_${reason}`));
  });
});

describe('LoginScreen — end to end over the real ApiClient (R8)', () => {
  it('session → register (no empty e-mail) → claim; the JSON reason reaches the screen', async () => {
    const authApi = apis['./auth-api.ts']?.authApi;
    expect(authApi, 'auth-api.ts exports authApi').toBeTypeOf('function');
    const seen: Array<{ url: string; body: unknown }> = [];
    const answers: Record<string, [number, unknown]> = {
      '/api/auth/session': [200, { user: null }],
      '/api/auth/register': [201, { registered: true }],
      '/api/auth/invite/claim': [
        400,
        { error: { code: 'inviteRefused', reason: 'alreadyLinked', message: SERVER_SENTENCE.userDisabled } },
      ],
    };
    const client = new ApiClient(async (url: string, init?: RequestInit) => {
      seen.push({ url, body: init?.body ? JSON.parse(String(init.body)) : undefined });
      const [status, body] = answers[url] ?? [404, { error: { code: 'notFound', message: url } }];
      return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
    });
    const { host, navigate } = mount((authApi as AuthApiFn)(client));
    await settle();
    mustButton(host, tx('haveInvite')).click();
    await settle();
    mustButton(host, tx('createAccount')).click();
    await settle();
    await fillCreate(host, { phone: '0899999999', password: 'secret-pw-2', email: '', inviteCode: 'INV-9' });

    expect(seen.map((s) => s.url)).toEqual(['/api/auth/session', '/api/auth/register', '/api/auth/invite/claim']);
    expect(seen[1]!.body).toEqual({ phone: '0899999999', password: 'secret-pw-2' });
    expect(seen[2]!.body).toEqual({ identifier: '0899999999', password: 'secret-pw-2', inviteCode: 'INV-9' });
    expect(messageLine(host)).toBe(tx('err_inviteRefused_alreadyLinked'));
    expect(navigate).not.toHaveBeenCalled();
  });
});

describe('web never matches the server refusal sentences (R9, #763 §4.3)', () => {
  const sources = import.meta.glob<string>('/src/**/*.{ts,tsx}', { query: '?raw', import: 'default', eager: true });

  it('no source outside i18n tables and tests holds a user_service.rs refusal sentence', () => {
    const checked = Object.entries(sources).filter(
      ([path]) => !/\.test\.tsx?$/.test(path) && !/\/i18n\.ts$/.test(path) && !/\/i18n\//.test(path),
    );
    expect(checked.length, 'the glob found the web sources').toBeGreaterThan(20);
    expect(
      checked.some(([path]) => path.endsWith('/screens/login/LoginScreen.tsx')),
      'LoginScreen.tsx is among them',
    ).toBe(true);
    const hits: string[] = [];
    for (const [path, text] of checked) {
      for (const sentence of Object.values(SERVER_SENTENCE)) {
        if (text.includes(sentence)) hits.push(`${path}: ${sentence}`);
      }
    }
    expect(hits).toEqual([]);
  });
});
