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

function tx(key: string, t: Record<string, string> = TH): string {
  expect(t[key], `i18n key ${key}`).toBeTypeOf('string');
  return t[key]!;
}

const TELLER: SessionUser = {
  id: 'u-7',
  name: 'Nok',
  organizationId: 'org-1',
  roles: [{ role: 'TELLER', branchId: 'XC-B01' }],
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

const notExpected = (name: string) => async () => {
  throw new Error(`${name} was not expected in this test`);
};

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

function submitButton(host: HTMLElement, t: Record<string, string> = TH): HTMLButtonElement {
  const text = tx('submit', t);
  const el = Array.from(host.querySelectorAll('button')).find((b) => b.textContent?.trim() === text);
  if (!el) throw new Error(`no button with text ${JSON.stringify(text)}`);
  return el;
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

describe('LoginScreen — identifier trimming (#2387)', () => {
  it('row 1: trims trailing space on email identifier ("nok@example.com ")', async () => {
    const { host, calls } = await signedOutScreen({ login: async () => ({ user: TELLER }) });
    await fillAndSubmit(host, 'nok@example.com ', 'secret-pw-1');
    expect(calls.login).toEqual([{ identifier: 'nok@example.com', password: 'secret-pw-1' }]);
  });

  it('row 2: trims leading spaces on username identifier ("  nok")', async () => {
    const { host, calls } = await signedOutScreen({ login: async () => ({ user: TELLER }) });
    await fillAndSubmit(host, '  nok', 'secret-pw-1');
    expect(calls.login).toEqual([{ identifier: 'nok', password: 'secret-pw-1' }]);
  });

  it('row 3: trims surrounding tabs from a paste ("\\tsomchai\\t")', async () => {
    const { host, calls } = await signedOutScreen({ login: async () => ({ user: TELLER }) });
    await fillAndSubmit(host, '\tsomchai\t', 'secret-pw-1');
    expect(calls.login).toEqual([{ identifier: 'somchai', password: 'secret-pw-1' }]);
  });

  it('row 4: sends clean identifiers ("nok", "nok@example.com", "+66812345678", "0812345678") identically whether clean or space-padded', async () => {
    for (const clean of ['nok', 'nok@example.com', '+66812345678', '0812345678']) {
      const { host, calls } = await signedOutScreen({ login: async () => ({ user: TELLER }) });
      await fillAndSubmit(host, clean, 'secret-pw-1');
      await fillAndSubmit(host, ` ${clean} `, 'secret-pw-1');
      expect(calls.login).toEqual([
        { identifier: clean, password: 'secret-pw-1' },
        { identifier: clean, password: 'secret-pw-1' },
      ]);
      dispose?.();
      dispose = undefined;
      document.body.innerHTML = '';
    }
  });

  it('row 5: never trims password (" secret ") while trimming identifier', async () => {
    const { host, calls } = await signedOutScreen({ login: async () => ({ user: TELLER }) });
    await fillAndSubmit(host, 'nok@example.com ', ' secret ');
    expect(calls.login).toEqual([{ identifier: 'nok@example.com', password: ' secret ' }]);
  });

  it('edge case 1: whitespace-only identifier trims to "" and shows server err_invalidRequest without new client refusal', async () => {
    const { host, calls } = await signedOutScreen({
      login: async () => {
        throw new ApiError({ code: 'invalidRequest', message: 'SERVER-SENTENCE-MUST-NOT-SHOW', status: 400 });
      },
    });
    await fillAndSubmit(host, '   ', 'secret-pw-1');
    expect(calls.login).toEqual([{ identifier: '', password: 'secret-pw-1' }]);
    expect(messageLine(host)).toBe(tx('err_invalidRequest'));
  });

  it('edge case 2: keeps a space inside a username while trimming outer whitespace', async () => {
    const { host, calls } = await signedOutScreen({ login: async () => ({ user: TELLER }) });
    await fillAndSubmit(host, ' somchai jaidee ', 'secret-pw-1');
    expect(calls.login).toEqual([{ identifier: 'somchai jaidee', password: 'secret-pw-1' }]);
  });
});
