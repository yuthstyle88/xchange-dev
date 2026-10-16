// RW-6 (#2014): SignUp.tsx registration goes through ApiClient.post (design xchange-v3-web.md §12.23).
// One test per row of the §9 behaviour table.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { SignUp } from './SignUp';
import * as table from './i18n';

function headerValue(headers: HeadersInit | undefined, name: string): string | undefined {
  if (!headers) return undefined;
  const target = name.toLowerCase();
  if (headers instanceof Headers) {
    return headers.get(target) ?? undefined;
  }
  if (Array.isArray(headers)) {
    const found = headers.find(([k]) => k.toLowerCase() === target);
    return found?.[1];
  }
  for (const [k, v] of Object.entries(headers)) {
    if (k.toLowerCase() === target) return v;
  }
  return undefined;
}

describe('SignUp registration through ApiClient.post (RW-6 #2014)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.restoreAllMocks();
    setLang('th');
  });

  function getErrorText(): string | null {
    const el = host.querySelector('.xc-sign-in-error');
    return el ? (el.textContent ?? '').trim() : null;
  }

  it('(1) the register POST carries x-xchange-request: 1', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn().mockImplementation(async () => {
      return new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    });

    dispose = render(
      () => <SignUp navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
    expect(submitBtn).not.toBeNull();
    submitBtn!.click();

    await vi.waitFor(() => {
      expect(mockFetch).toHaveBeenCalledTimes(1);
    });

    const [url, init] = mockFetch.mock.calls[0] as [string, RequestInit | undefined];
    expect(url).toBe('/api/app/register');
    expect(init?.method).toBe('POST');
    expect(headerValue(init?.headers, 'x-xchange-request')).toBe('1');
  });

  it('(2) a keyed 4xx refusal (consentTextMissing) shows its key text', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn().mockImplementation(async () => {
      return new Response(
        JSON.stringify({
          error: {
            code: 'consentTextMissing',
            field: 'lang',
            message: 'Consent text missing',
          },
        }),
        { status: 409, headers: { 'Content-Type': 'application/json' } },
      );
    });

    dispose = render(
      () => <SignUp navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
    expect(submitBtn).not.toBeNull();
    submitBtn!.click();

    await vi.waitFor(() => {
      expect(getErrorText()).toBe(table.th.consentTextMissing);
    });
    expect(getErrorText()).not.toBe(th['error.serverError']);
    expect(navigate).not.toHaveBeenCalled();
  });

  it("(3) 500 {code:'roleNotAllowedHere'} shows th['error.serverError']", async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn().mockImplementation(async () => {
      return new Response(
        JSON.stringify({
          error: {
            code: 'roleNotAllowedHere',
            message: 'Role not allowed here',
          },
        }),
        { status: 500, headers: { 'Content-Type': 'application/json' } },
      );
    });

    dispose = render(
      () => <SignUp navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
    expect(submitBtn).not.toBeNull();
    submitBtn!.click();

    await vi.waitFor(() => {
      expect(getErrorText()).toBe(th['error.serverError']);
    });
    expect(getErrorText()).not.toBe(th['error.roleNotAllowedHere']);
    expect(navigate).not.toHaveBeenCalled();
  });

  it('(4) network TypeError shows error.network', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn().mockRejectedValue(new TypeError('Failed to fetch'));

    dispose = render(
      () => <SignUp navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
    expect(submitBtn).not.toBeNull();
    submitBtn!.click();

    await vi.waitFor(() => {
      expect(getErrorText()).toBe(th['error.network']);
    });
    expect(getErrorText()).not.toContain('Failed to fetch');

    dispose();
    setLang('en');
    dispose = render(
      () => <SignUp navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    const submitBtnEn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
    expect(submitBtnEn).not.toBeNull();
    submitBtnEn!.click();

    await vi.waitFor(() => {
      expect(getErrorText()).toBe(en['error.network']);
    });
    expect(getErrorText()).not.toContain('Failed to fetch');
    expect(navigate).not.toHaveBeenCalled();
  });

  it('(5) success navigates to /app as today', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn().mockImplementation(async () => {
      return new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    });

    dispose = render(
      () => <SignUp navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
    expect(submitBtn).not.toBeNull();
    submitBtn!.click();

    await vi.waitFor(() => {
      expect(navigate).toHaveBeenCalledTimes(1);
    });
    expect(navigate).toHaveBeenCalledWith('/app');
    expect(getErrorText()).toBeNull();
  });
});
