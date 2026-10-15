// RW-4 (#2012): customer sign-in steps post through ApiClient.post and route errors through wordedError.
//
// Step 1 trace (SignIn.tsx URL -> server handler -> origin check):
// 1. POST /api/auth/otp/request (SignIn.tsx requestCode)
//    -> handler: v3_otp_request in api/src/presentation/http/auth.rs:237-276 (wired at auth.rs:89-92)
//    -> origin check: YES — `if !same_origin(&req)` at auth.rs:242-244 (origin.rs:33-55 requires
//       `x-xchange-request: 1` and matching `Origin`), refusing with 400 `invalidRequest`.
// 2. POST /api/auth/otp/verify (SignIn.tsx verify)
//    -> handler: v3_otp_verify in api/src/presentation/http/auth.rs:278-331 (wired at auth.rs:94-97)
//    -> origin check: YES — `if !same_origin(&req)` at auth.rs:284-286 (origin.rs:33-55 requires
//       `x-xchange-request: 1` and matching `Origin`), refusing with 400 `invalidRequest`.

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { en } from '../../../shared/i18n/en';
import { th } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';
import { SignIn } from './SignIn';

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

describe('SignIn ApiClient.post and wordedError (RW-4 #2012 §9)', () => {
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

  function getAlertText(): string | null {
    const el = host.querySelector('[role="alert"], .xc-sign-in-error');
    return el ? (el.textContent ?? '').trim() : null;
  }

  function typePhone(phone: string): void {
    const phoneInput = host.querySelector<HTMLInputElement>('input[name="phone"]');
    expect(phoneInput, 'phone input exists').not.toBeNull();
    phoneInput!.value = phone;
    phoneInput!.dispatchEvent(new Event('input', { bubbles: true }));
    phoneInput!.dispatchEvent(new Event('change', { bubbles: true }));
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

  it("row 1: each sign-in step's POST carries x-xchange-request: 1", async () => {
    const navigate = vi.fn();
    const calls: RecordedCall[] = [];

    const mockFetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ url, method, headers, body });

      if (url === '/api/auth/otp/request') {
        return jsonResponse(200, { challengeId: 'ch-row-1', expiresAt: '2026-10-16T00:00:00Z' });
      }
      if (url === '/api/auth/otp/verify') {
        return jsonResponse(200, { user: { id: 'u-1', name: 'Customer' } });
      }
      return jsonResponse(404, { error: { code: 'notFound', message: url } });
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    // Step 1: request OTP code
    typePhone('0812345678');
    clickSubmit();

    await vi.waitFor(() => {
      const req = calls.find((c) => c.url === '/api/auth/otp/request' && c.method === 'POST');
      expect(req, 'POST /api/auth/otp/request was sent').toBeDefined();
    });

    // Step 2: verify OTP code
    typeCode('123456');
    clickSubmit();

    await vi.waitFor(() => {
      const ver = calls.find((c) => c.url === '/api/auth/otp/verify' && c.method === 'POST');
      expect(ver, 'POST /api/auth/otp/verify was sent').toBeDefined();
    });

    const req = calls.find((c) => c.url === '/api/auth/otp/request' && c.method === 'POST')!;
    const ver = calls.find((c) => c.url === '/api/auth/otp/verify' && c.method === 'POST')!;
    expect(
      req.headers.get('x-xchange-request'),
      'POST /api/auth/otp/request carries x-xchange-request: 1',
    ).toBe('1');
    expect(
      ver.headers.get('x-xchange-request'),
      'POST /api/auth/otp/verify carries x-xchange-request: 1',
    ).toBe('1');
  });

  it('row 2: a wrong code (4xx keyed) shows its key text', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (url === '/api/auth/otp/request') {
        return jsonResponse(200, { challengeId: 'ch-row-2' });
      }
      if (url === '/api/auth/otp/verify') {
        return jsonResponse(400, {
          error: {
            code: 'invalidRequest',
            message: 'Raw server fallback text that must be replaced by key',
          },
        });
      }
      return jsonResponse(404, {});
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    typePhone('0812345678');
    clickSubmit();

    await vi.waitFor(() => {
      expect(mockFetch).toHaveBeenCalledTimes(1);
    });

    typeCode('000000');
    clickSubmit();

    await vi.waitFor(() => {
      expect(getAlertText()).toBeTruthy();
    });
    expect(getAlertText()).toBe(th['error.invalidRequest']);
    expect(getAlertText()).not.toBe(th['error.serverError']);
    expect(getAlertText()).not.toContain('Raw server fallback text');
    expect(navigate).not.toHaveBeenCalled();
  });

  it("row 3: 500 {code:'roleNotAllowedHere'} shows th['error.serverError']", async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (url === '/api/auth/otp/request') {
        return jsonResponse(200, { challengeId: 'ch-row-3' });
      }
      return jsonResponse(500, {
        error: {
          code: 'roleNotAllowedHere',
          message: 'Role is not allowed on this route',
        },
      });
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    typePhone('0812345678');
    clickSubmit();

    await vi.waitFor(() => {
      expect(mockFetch).toHaveBeenCalledTimes(1);
    });

    typeCode('123456');
    clickSubmit();

    await vi.waitFor(() => {
      expect(getAlertText()).toBeTruthy();
    });
    expect(getAlertText()).toBe(th['error.serverError']);
    expect(getAlertText()).not.toBe(th['error.roleNotAllowedHere']);
    expect(getAlertText()).not.toContain('Role is not allowed on this route');
    expect(navigate).not.toHaveBeenCalled();
  });

  it('row 4: network TypeError shows error.network', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn().mockRejectedValue(new TypeError('Failed to fetch'));

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    typePhone('0812345678');
    clickSubmit();

    await vi.waitFor(() => {
      expect(getAlertText()).toBeTruthy();
    });
    expect(getAlertText()).toBe(th['error.network']);
    expect(getAlertText()).not.toContain('Failed to fetch');

    // Also verify in EN
    dispose?.();
    host.innerHTML = '';
    setLang('en');
    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    typePhone('0812345678');
    clickSubmit();

    await vi.waitFor(() => {
      expect(getAlertText()).toBeTruthy();
    });
    expect(getAlertText()).toBe(en['error.network']);
    expect(getAlertText()).not.toContain('Failed to fetch');
    expect(navigate).not.toHaveBeenCalled();
  });

  it('row 5: success performs the same navigation as today', async () => {
    const navigate = vi.fn();
    const calls: RecordedCall[] = [];

    const mockFetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ url, method, headers, body });

      if (url === '/api/auth/otp/request') {
        return jsonResponse(200, { challengeId: 'ch-row-5' });
      }
      if (url === '/api/auth/otp/verify') {
        return jsonResponse(200, { user: { id: 'u-cust-5', name: 'Customer Five' } });
      }
      return jsonResponse(404, {});
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    typePhone('0812345678');
    clickSubmit();

    await vi.waitFor(() => {
      expect(calls).toHaveLength(1);
    });
    expect(calls[0]!.url).toBe('/api/auth/otp/request');
    expect(calls[0]!.body).toEqual({ destination: '+66812345678', channel: 'sms' });
    expect(navigate).not.toHaveBeenCalled();

    typeCode('654321');
    clickSubmit();

    await vi.waitFor(() => {
      expect(navigate).toHaveBeenCalledTimes(1);
    });
    expect(calls).toHaveLength(2);
    expect(calls[1]!.url).toBe('/api/auth/otp/verify');
    expect(calls[1]!.body).toEqual({ challengeId: 'ch-row-5', code: '654321' });
    expect(navigate).toHaveBeenCalledWith('/app');
  });
});
