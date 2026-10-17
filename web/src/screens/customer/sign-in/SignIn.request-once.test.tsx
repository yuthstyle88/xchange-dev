import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';
import { SignIn } from './SignIn';

interface RecordedCall {
  url: string;
  method: string;
  body: unknown;
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('Customer SignIn OTP request in-flight guard (#2240)', () => {
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

  function typePhone(phone: string): HTMLInputElement {
    const phoneInput = host.querySelector<HTMLInputElement>('input[name="phone"]');
    expect(phoneInput, 'phone input exists').not.toBeNull();
    phoneInput!.value = phone;
    phoneInput!.dispatchEvent(new Event('input', { bubbles: true }));
    phoneInput!.dispatchEvent(new Event('change', { bubbles: true }));
    return phoneInput!;
  }

  function typeCode(code: string): HTMLInputElement {
    const codeInput = host.querySelector<HTMLInputElement>('input[name="code"]');
    expect(codeInput, 'code input exists').not.toBeNull();
    codeInput!.value = code;
    codeInput!.dispatchEvent(new Event('input', { bubbles: true }));
    codeInput!.dispatchEvent(new Event('change', { bubbles: true }));
    return codeInput!;
  }

  function submitButton(): HTMLButtonElement {
    const btn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
    expect(btn, 'submit button exists').not.toBeNull();
    return btn!;
  }

  function getAlertText(): string | null {
    const el = host.querySelector('[role="alert"], .xc-sign-in-error');
    return el ? (el.textContent ?? '').trim() : null;
  }

  it('row 1: Double-click "request code" before any answer -> Exactly one POST /api/auth/otp/request', async () => {
    const navigate = vi.fn();
    const calls: RecordedCall[] = [];
    let resolveFirstRequest!: (res: Response) => void;

    const mockFetch = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ url, method, body });

      if (url === '/api/auth/otp/request') {
        if (calls.filter((c) => c.url === '/api/auth/otp/request').length === 1) {
          return new Promise<Response>((resolve) => {
            resolveFirstRequest = resolve;
          });
        }
        return Promise.resolve(jsonResponse(200, { challengeId: 'ch-second' }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    typePhone('0812345678');
    const btn = submitButton();

    // Double-click "request code" before the first request answers
    btn.click();
    btn.click();

    expect(
      calls.filter((c) => c.url === '/api/auth/otp/request' && c.method === 'POST'),
    ).toHaveLength(1);
    expect(btn.disabled, 'request button is disabled while request is in flight').toBe(true);

    resolveFirstRequest(jsonResponse(200, { challengeId: 'ch-first' }));

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-sign-in-link')).not.toBeNull();
    });

    expect(
      calls.filter((c) => c.url === '/api/auth/otp/request' && c.method === 'POST'),
    ).toHaveLength(1);
    expect(btn.disabled).toBe(false);
  });

  it('row 2: First request answers, then "resend code" -> A second POST is sent (resend still works)', async () => {
    const navigate = vi.fn();
    const calls: RecordedCall[] = [];
    let reqCount = 0;

    const mockFetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ url, method, body });

      if (url === '/api/auth/otp/request') {
        reqCount += 1;
        return jsonResponse(200, { challengeId: `ch-${reqCount}` });
      }
      if (url === '/api/auth/otp/verify') {
        return jsonResponse(200, { user: { id: 'u-1', name: 'Customer' } });
      }
      return jsonResponse(404, {});
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    typePhone('0812345678');
    submitButton().click();

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-sign-in-link')).not.toBeNull();
    });
    expect(calls.filter((c) => c.url === '/api/auth/otp/request')).toHaveLength(1);

    // Click "resend code" after first request answered
    const resendBtn = host.querySelector<HTMLButtonElement>('.xc-sign-in-link');
    expect(resendBtn).not.toBeNull();
    resendBtn!.click();

    await vi.waitFor(() => {
      expect(calls.filter((c) => c.url === '/api/auth/otp/request')).toHaveLength(2);
    });

    // Verify uses the latest challengeId from the resend
    typeCode('123456');
    submitButton().click();

    await vi.waitFor(() => {
      expect(navigate).toHaveBeenCalledWith('/app');
    });
    const verifyCall = calls.find((c) => c.url === '/api/auth/otp/verify');
    expect(verifyCall?.body).toEqual({ challengeId: 'ch-2', code: '123456' });
  });

  it('row 3: Request fails -> Error shown via wordedError; the button is enabled again', async () => {
    const navigate = vi.fn();
    let rejectFirstRequest!: (err: unknown) => void;
    let attempt = 0;

    const mockFetch = vi.fn(() => {
      attempt += 1;
      if (attempt === 1) {
        return new Promise<Response>((_, reject) => {
          rejectFirstRequest = reject;
        });
      }
      return Promise.resolve(jsonResponse(200, { challengeId: 'ch-recovered' }));
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    typePhone('0812345678');
    const btn = submitButton();
    btn.click();

    expect(btn.disabled, 'button is disabled while request is in flight').toBe(true);

    rejectFirstRequest(new TypeError('Failed to fetch'));

    await vi.waitFor(() => {
      expect(getAlertText()).toBe(th['error.network']);
    });
    expect(btn.disabled, 'button is enabled again after failure').toBe(false);

    // Clicking again after failure sends a new request
    btn.click();
    await vi.waitFor(() => {
      expect(host.querySelector('.xc-sign-in-link')).not.toBeNull();
    });
    expect(mockFetch).toHaveBeenCalledTimes(2);
  });

  it('row 4: Code entered and verified -> Verify flow unchanged', async () => {
    const navigate = vi.fn();
    const calls: RecordedCall[] = [];

    const mockFetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ url, method, body });

      if (url === '/api/auth/otp/request') {
        return jsonResponse(200, { challengeId: 'ch-verify-row4' });
      }
      if (url === '/api/auth/otp/verify') {
        return jsonResponse(200, { user: { id: 'u-4', name: 'Customer Four' } });
      }
      return jsonResponse(404, {});
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    typePhone('0812345678');
    submitButton().click();

    await vi.waitFor(() => {
      expect(calls.filter((c) => c.url === '/api/auth/otp/request')).toHaveLength(1);
    });

    typeCode('654321');
    submitButton().click();

    await vi.waitFor(() => {
      expect(navigate).toHaveBeenCalledWith('/app');
    });
    expect(calls.find((c) => c.url === '/api/auth/otp/verify')?.body).toEqual({
      challengeId: 'ch-verify-row4',
      code: '654321',
    });
  });

  it('edge case (§9): Enter in the phone field plus a click -> still one request', async () => {
    const navigate = vi.fn();
    const calls: RecordedCall[] = [];
    let resolveFirstRequest!: (res: Response) => void;

    const mockFetch = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ url, method, body });

      if (url === '/api/auth/otp/request') {
        if (calls.filter((c) => c.url === '/api/auth/otp/request').length === 1) {
          return new Promise<Response>((resolve) => {
            resolveFirstRequest = resolve;
          });
        }
        return Promise.resolve(jsonResponse(200, { challengeId: 'ch-duplicate' }));
      }
      if (url === '/api/auth/otp/verify') {
        return Promise.resolve(jsonResponse(200, { user: { id: 'u-1', name: 'Customer' } }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    typePhone('0812345678');
    const form = host.querySelector<HTMLFormElement>('form.xc-sign-in-form');
    expect(form, 'form exists').not.toBeNull();
    const btn = submitButton();

    // Pressing Enter in the phone field submits the form, followed immediately by a click on the submit button
    form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    btn.click();
    form!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    expect(
      calls.filter((c) => c.url === '/api/auth/otp/request' && c.method === 'POST'),
    ).toHaveLength(1);

    resolveFirstRequest(jsonResponse(200, { challengeId: 'ch-enter-once' }));

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-sign-in-link')).not.toBeNull();
    });

    expect(
      calls.filter((c) => c.url === '/api/auth/otp/request' && c.method === 'POST'),
    ).toHaveLength(1);

    typeCode('111222');
    btn.click();

    await vi.waitFor(() => {
      expect(navigate).toHaveBeenCalledWith('/app');
    });
    expect(calls.find((c) => c.url === '/api/auth/otp/verify')?.body).toEqual({
      challengeId: 'ch-enter-once',
      code: '111222',
    });
  });
});
