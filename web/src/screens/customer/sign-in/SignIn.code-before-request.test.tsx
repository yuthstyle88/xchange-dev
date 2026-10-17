import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
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

describe('Customer SignIn code box non-empty before request (#2608)', () => {
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

  it('row 1: No challenge, nothing in flight, code box non-empty, submit -> One POST /api/auth/otp/request; no verify', async () => {
    const navigate = vi.fn();
    const calls: RecordedCall[] = [];

    const mockFetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ url, method, body });

      if (url === '/api/auth/otp/request') {
        return jsonResponse(200, { challengeId: 'ch-test-row1' });
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
    typeCode('987654');

    submitButton().click();

    await vi.waitFor(() => {
      expect(
        calls.filter((c) => c.url === '/api/auth/otp/request' && c.method === 'POST'),
      ).toHaveLength(1);
    });

    const requestCall = calls.find((c) => c.url === '/api/auth/otp/request');
    expect(requestCall?.body).toEqual({ destination: '+66812345678', channel: 'sms' });

    expect(calls.filter((c) => c.url === '/api/auth/otp/verify')).toHaveLength(0);
    expect(navigate).not.toHaveBeenCalled();
  });

  it('row 2: Challenge set, code typed, submit -> One verify with that challengeId (kept)', async () => {
    const navigate = vi.fn();
    const calls: RecordedCall[] = [];

    const mockFetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ url, method, body });

      if (url === '/api/auth/otp/request') {
        return jsonResponse(200, { challengeId: 'ch-test-row2' });
      }
      if (url === '/api/auth/otp/verify') {
        return jsonResponse(200, { user: { id: 'u-2', name: 'Customer Two' } });
      }
      return jsonResponse(404, {});
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    typePhone('0812345678');
    typeCode('123456');

    // First submit requests code and sets challenge
    submitButton().click();

    await vi.waitFor(() => {
      expect(calls.filter((c) => c.url === '/api/auth/otp/request')).toHaveLength(1);
      expect(host.querySelector('.xc-sign-in-link')).not.toBeNull();
    });

    // Second submit with challenge set sends verify
    submitButton().click();

    await vi.waitFor(() => {
      expect(calls.filter((c) => c.url === '/api/auth/otp/verify')).toHaveLength(1);
      expect(navigate).toHaveBeenCalledWith('/app');
    });

    const verifyCall = calls.find((c) => c.url === '/api/auth/otp/verify');
    expect(verifyCall?.body).toEqual({ challengeId: 'ch-test-row2', code: '123456' });
  });

  it('row 3: Request in flight, code typed, submit -> Verify waits for the request, then sends (kept)', async () => {
    const navigate = vi.fn();
    const calls: RecordedCall[] = [];
    let resolveRequest!: (res: Response) => void;

    const mockFetch = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ url, method, body });

      if (url === '/api/auth/otp/request') {
        return new Promise<Response>((resolve) => {
          resolveRequest = resolve;
        });
      }
      if (url === '/api/auth/otp/verify') {
        return Promise.resolve(jsonResponse(200, { user: { id: 'u-3', name: 'Customer Three' } }));
      }
      return Promise.resolve(jsonResponse(404, {}));
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    typePhone('0812345678');
    typeCode('654321');

    // First submit launches request (in flight)
    submitButton().click();

    await vi.waitFor(() => {
      expect(calls.filter((c) => c.url === '/api/auth/otp/request')).toHaveLength(1);
    });

    // While request is in flight, submit again
    submitButton().click();

    // Now resolve the in-flight request
    resolveRequest(jsonResponse(200, { challengeId: 'ch-test-row3' }));

    // Verify should wait for request to complete and then send
    await vi.waitFor(() => {
      expect(calls.filter((c) => c.url === '/api/auth/otp/verify')).toHaveLength(1);
      expect(navigate).toHaveBeenCalledWith('/app');
    });

    const verifyCall = calls.find((c) => c.url === '/api/auth/otp/verify');
    expect(verifyCall?.body).toEqual({ challengeId: 'ch-test-row3', code: '654321' });
  });
});
