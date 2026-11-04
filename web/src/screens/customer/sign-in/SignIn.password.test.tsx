// #5958: customer sign-in password door beside OTP (design customer-app.md §4.5, O-6).
// P1–P8 behaviour table from #5958.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';
import { SignIn } from './SignIn';
import * as table from './i18n';

interface RecordedCall {
  url: string;
  method: string;
  headers: Headers;
  body: unknown;
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('SignIn password door (ticket #5958 P1–P8)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeAll(() => warmModules(() => import('./SignIn')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAudience('customer');
    setLang('th');
  });

  function clickTab(text: string): void {
    const tabs = Array.from(host.querySelectorAll<HTMLButtonElement>('[role="tab"], .xc-sign-in-tab'));
    const target = tabs.find((b) => (b.textContent ?? '').trim() === text);
    expect(target, `tab "${text}" exists`).toBeDefined();
    target!.click();
  }

  function typePhone(phone: string): void {
    const phoneInput = host.querySelector<HTMLInputElement>('input[name="phone"]');
    expect(phoneInput, 'phone input exists').not.toBeNull();
    phoneInput!.value = phone;
    phoneInput!.dispatchEvent(new Event('input', { bubbles: true }));
    phoneInput!.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function typePassword(pwd: string): void {
    const pwdInput = host.querySelector<HTMLInputElement>('input[name="password"]');
    expect(pwdInput, 'password input exists').not.toBeNull();
    pwdInput!.value = pwd;
    pwdInput!.dispatchEvent(new Event('input', { bubbles: true }));
    pwdInput!.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function typeCode(code: string): void {
    const codeInput = host.querySelector<HTMLInputElement>('input[name="code"]');
    expect(codeInput, 'code input exists').not.toBeNull();
    codeInput!.value = code;
    codeInput!.dispatchEvent(new Event('input', { bubbles: true }));
    codeInput!.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function clickSubmit(): void {
    const btn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
    expect(btn, 'submit button exists').not.toBeNull();
    btn!.click();
  }

  it('P1: open /app/sign-in renders today\'s OTP form by default', () => {
    const navigate = vi.fn();
    dispose = render(() => <SignIn navigate={navigate} />, host);

    // Default mode is OTP: phone and code inputs exist, no password input
    expect(host.querySelector('input[name="phone"]')).not.toBeNull();
    expect(host.querySelector('input[name="code"]')).not.toBeNull();
    expect(host.querySelector('input[name="password"]')).toBeNull();

    // Mode switch offers both OTP and password doors
    const tabs = Array.from(host.querySelectorAll('[role="tab"]'));
    expect(tabs.length).toBe(2);
    expect(tabs[0]!.textContent?.trim()).toBe(table.th.modeOtp);
    expect(tabs[1]!.textContent?.trim()).toBe(table.th.modePassword);
    expect(tabs[0]!.getAttribute('aria-selected')).toBe('true');
    expect(tabs[1]!.getAttribute('aria-selected')).toBe('false');
  });

  it('P2: password mode, 081-234-5678 sends one POST /api/auth/login with {identifier, password}', async () => {
    const navigate = vi.fn();
    const calls: RecordedCall[] = [];

    const mockFetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ url, method, headers, body });
      return jsonResponse(200, { user: { id: 'u-1', name: 'Customer' } });
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    clickTab(table.th.modePassword);
    expect(host.querySelector('input[name="password"]')).not.toBeNull();

    typePhone('081-234-5678');
    typePassword('mySecretPass1');
    clickSubmit();

    await vi.waitFor(() => {
      expect(calls.filter((c) => c.url === '/api/auth/login' && c.method === 'POST')).toHaveLength(1);
    });

    const call = calls.find((c) => c.url === '/api/auth/login')!;
    expect(call.body).toEqual({
      identifier: '+66812345678',
      password: 'mySecretPass1',
    });
    expect(call.headers.get('x-xchange-request')).toBe('1');
  });

  it('P3: P2 answers 200 navigates to next() or /app', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn(async () => jsonResponse(200, { user: { id: 'u-1' } }));

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} search="?next=%2Fapp%2Fbookings" />,
      host,
    );

    clickTab(table.th.modePassword);
    typePhone('0812345678');
    typePassword('pass123');
    clickSubmit();

    await vi.waitFor(() => {
      expect(navigate).toHaveBeenCalledWith('/app/bookings');
    });
  });

  it('P4: 401 invalidCredentials shows worded error under password and clears password', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn(async () =>
      jsonResponse(401, {
        error: { code: 'invalidCredentials', message: 'ชื่อผู้ใช้หรือรหัสผ่านไม่ถูกต้อง' },
      }),
    );

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    clickTab(table.th.modePassword);
    typePhone('0812345678');
    typePassword('wrongPassword');
    clickSubmit();

    await vi.waitFor(() => {
      const err = host.querySelector('[data-field-error="password"]');
      expect(err).not.toBeNull();
      expect(err!.textContent?.trim()).toBe(th['input.code.invalidCredentials']);
    });

    const pwdInput = host.querySelector<HTMLInputElement>('input[name="password"]');
    expect(pwdInput?.value).toBe('');
    expect(navigate).not.toHaveBeenCalled();
  });

  it('P5: 429 tooManyAttempts / 503 are worded as today\'s showError does', async () => {
    const navigate = vi.fn();
    let status = 429;
    let errBody: unknown = { error: { code: 'tooManyAttempts' } };

    const mockFetch = vi.fn(async () => jsonResponse(status, errBody));

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    clickTab(table.th.modePassword);
    typePhone('0812345678');
    typePassword('pass123');
    clickSubmit();

    // 429 tooManyAttempts targets phone field
    await vi.waitFor(() => {
      const phoneErr = host.querySelector('[data-field-error="phone"]');
      expect(phoneErr).not.toBeNull();
      expect(phoneErr!.textContent?.trim()).toBe(th['input.code.tooManyAttempts']);
    });

    // 503 identityUnavailable is server failure -> form error
    status = 503;
    errBody = { error: { code: 'identityUnavailable' } };
    typePassword('pass123');
    clickSubmit();

    await vi.waitFor(() => {
      const formErr = host.querySelector('p[role="alert"].xc-sign-in-error');
      expect(formErr).not.toBeNull();
      expect(formErr!.textContent?.trim()).toBe(th['error.serverError']);
    });
  });

  it('P6: empty phone or empty password shows field error and sends nothing', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn();

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    clickTab(table.th.modePassword);

    // Both empty
    clickSubmit();
    expect(mockFetch).not.toHaveBeenCalled();
    expect(host.querySelector('[data-field-error="phone"]')?.textContent?.trim()).toBe(
      th['input.code.phoneRequired'],
    );
    expect(host.querySelector('[data-field-error="password"]')?.textContent?.trim()).toBe(
      table.th.passwordRequired,
    );

    // Empty phone only
    typePassword('secret');
    clickSubmit();
    expect(mockFetch).not.toHaveBeenCalled();
    expect(host.querySelector('[data-field-error="phone"]')?.textContent?.trim()).toBe(
      th['input.code.phoneRequired'],
    );

    // Empty password only
    typePhone('0812345678');
    const pwdInput = host.querySelector<HTMLInputElement>('input[name="password"]')!;
    pwdInput.value = '';
    pwdInput.dispatchEvent(new Event('input', { bubbles: true }));
    clickSubmit();
    expect(mockFetch).not.toHaveBeenCalled();
    expect(host.querySelector('[data-field-error="password"]')?.textContent?.trim()).toBe(
      table.th.passwordRequired,
    );
  });

  it('P7: double submit sends one request and button is disabled while in flight', async () => {
    const navigate = vi.fn();
    let resolveLogin!: (res: Response) => void;
    const pendingPromise = new Promise<Response>((r) => {
      resolveLogin = r;
    });

    const mockFetch = vi.fn(async () => pendingPromise);

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    clickTab(table.th.modePassword);
    typePhone('0812345678');
    typePassword('pass123');

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    expect(submitBtn.disabled).toBe(false);

    clickSubmit();
    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(submitBtn.disabled).toBe(true);

    // Double submit attempt while in flight
    clickSubmit();
    expect(mockFetch).toHaveBeenCalledTimes(1);

    resolveLogin(jsonResponse(200, { user: { id: 'u-1' } }));
    await vi.waitFor(() => {
      expect(navigate).toHaveBeenCalledWith('/app');
    });
  });

  it('P8: switch back to OTP keeps today\'s form and its state untouched', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (url === '/api/auth/otp/request') {
        return jsonResponse(200, { challengeId: 'ch-test-p8' });
      }
      return jsonResponse(404, {});
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    // Enter phone and request OTP
    typePhone('0812345678');
    clickSubmit();

    await vi.waitFor(() => {
      expect(host.textContent).toContain(table.th.codeSent);
    });

    // Enter verification code
    typeCode('987654');

    // Switch to password mode
    clickTab(table.th.modePassword);
    expect(host.querySelector('input[name="password"]')).not.toBeNull();
    expect(host.querySelector('input[name="code"]')).toBeNull();

    // Switch back to OTP mode
    clickTab(table.th.modeOtp);
    expect(host.querySelector('input[name="password"]')).toBeNull();
    const phoneInput = host.querySelector<HTMLInputElement>('input[name="phone"]');
    const codeInput = host.querySelector<HTMLInputElement>('input[name="code"]');
    expect(phoneInput?.value).toBe('0812345678');
    expect(codeInput?.value).toBe('987654');
    expect(host.textContent).toContain(table.th.codeSent);

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
    expect(submitBtn?.textContent?.trim()).toBe(table.th.verify);
  });
});
