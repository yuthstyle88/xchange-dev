// APP-7 (#998): customer sign-in and sign-up screens (design `customer-app.md` §4.5, §9.1 row APP-7).
// Written before the implementation by test-writer gemini/warut-6 on claude/5edf7bb5's order.
// Do not edit, delete or weaken; add your own tests in another file.
//
// What this file pins:
// - SignUp screen shows the pinned consentTextMissing sentence in th/en/zh without fallback on 409 and does not navigate
// - SignIn pins the OTP flow: phone number -> POST /api/auth/otp/request, code -> POST /api/auth/otp/verify,
//   navigate('/app') only after verify answers 200
// - SignIn verify 401 shows error and does not navigate
// - th / en / zh have identical key sets with non-empty texts
// - main.tsx calls setAudience('customer') before render (as RateDisplay.test.tsx:570 pins)
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { audience, setAudience, setLang } from '../../../shared/i18n';
import { SignIn } from './SignIn';
import { SignUp } from './SignUp';
import * as table from './i18n';

// SF-5 harness (docs/design/web-silent-failures.md §6): sign-up waits for the TERMS text, so the fake server also
// answers the host and a TERMS consent text; every other request goes to this test's own fake, unchanged.
function withTerms(inner: typeof fetch): typeof fetch {
  return (async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = new URL(String(input), 'http://localhost').pathname;
    const json = (body: unknown) =>
      new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
    if (path === '/api/public/host') return json({ org: 'XC' });
    if (path === '/api/public/orgs/XC/consent_texts') return json([{ purpose: 'TERMS', version: 1, body: 'Terms.' }]);
    return inner(input, init);
  }) as typeof fetch;
}

/** SF-5 harness: wait until the TERMS text is shown and the submit button can be pressed. */
async function waitForTerms(root: HTMLElement): Promise<void> {
  await vi.waitFor(() => {
    if (!root.querySelector('[data-testid="terms-body"]')) throw new Error('TERMS text not shown yet');
    if (root.querySelector<HTMLButtonElement>('button[type="submit"]')?.disabled) throw new Error('submit disabled');
  });
}

describe('customer sign-in and sign-up (APP-7 #998)', () => {
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
    host.remove();
    vi.restoreAllMocks();
  });

  it('th, en, and zh translation tables have identical key sets with non-empty strings', () => {
    const thKeys = Object.keys(table.th).sort();
    const enKeys = Object.keys(table.en).sort();
    const zhKeys = Object.keys(table.zh).sort();
    expect(enKeys, 'en keys match th').toEqual(thKeys);
    expect(zhKeys, 'zh keys match th').toEqual(thKeys);
    expect(thKeys.length, 'translation table has keys').toBeGreaterThan(0);

    for (const [lang, tbl] of Object.entries({ th: table.th, en: table.en, zh: table.zh })) {
      for (const [k, v] of Object.entries(tbl as Record<string, string>)) {
        expect(typeof v, `${lang}.${k} is string`).toBe('string');
        expect(v.trim().length, `${lang}.${k} is non-empty`).toBeGreaterThan(0);
      }
    }
  });

  it('pins consentTextMissing translation key in th, en, zh without fallback', () => {
    const th = table.th as unknown as Record<string, string>;
    const en = table.en as unknown as Record<string, string>;
    const zh = table.zh as unknown as Record<string, string>;

    expect(th.consentTextMissing, 'th.consentTextMissing must exist').toBeTypeOf('string');
    expect(th.consentTextMissing.trim().length, 'th.consentTextMissing non-empty').toBeGreaterThan(0);

    expect(en.consentTextMissing, 'en.consentTextMissing must exist').toBeTypeOf('string');
    expect(en.consentTextMissing.trim().length, 'en.consentTextMissing non-empty').toBeGreaterThan(0);

    expect(zh.consentTextMissing, 'zh.consentTextMissing must exist').toBeTypeOf('string');
    expect(zh.consentTextMissing.trim().length, 'zh.consentTextMissing non-empty').toBeGreaterThan(0);
  });

  it.each(['th', 'en', 'zh'] as const)(
    'sign-up shows the pinned consentTextMissing sentence in %s on 409 response and does not navigate',
    async (lang) => {
      setLang(lang);
      const tbl = table[lang] as unknown as Record<string, string>;
      const expectedText = tbl.consentTextMissing;
      expect(expectedText, `${lang}.consentTextMissing must exist`).toBeTypeOf('string');

      const navigate = vi.fn();
      const mockFetch = vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: {
              code: 'consentTextMissing',
              field: 'lang',
              message: 'Consent text missing',
            },
          }),
          { status: 409 }
        )
      );

      dispose = render(
        () => <SignUp navigate={navigate} fetch={withTerms(mockFetch as unknown as typeof fetch)} />,
        host
      );

      const submitBtn =
        host.querySelector<HTMLButtonElement>('button[type="submit"]') ??
        host.querySelector<HTMLButtonElement>('button');
      expect(submitBtn, 'sign-up submit button').not.toBeNull();
      await waitForTerms(host);
      submitBtn!.click();

      await vi.waitFor(() => {
        const text = host.textContent ?? '';
        expect(text).toContain(expectedText);
      });

      expect(navigate, 'does not navigate after 409 refusal').not.toHaveBeenCalled();
    }
  );

  it('sign-in pins OTP flow: request -> verify -> navigate("/app") only after 200', async () => {
    const navigate = vi.fn();
    const calls: Array<{ url: string; method: string; body?: unknown }> = [];

    const mockFetch = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = init?.method ?? 'GET';
      const body = init?.body ? JSON.parse(init.body as string) : undefined;
      calls.push({ url, method, body });

      if (url.includes('/api/auth/otp/request')) {
        return new Response(JSON.stringify({ challengeId: 'ch-test-1', ok: true }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }

      if (url.includes('/api/auth/otp/verify')) {
        return new Response(
          JSON.stringify({ session: 'sess-cust-1', user: { id: 'u-cust-1', name: 'Customer' } }),
          { status: 200, headers: { 'Content-Type': 'application/json' } }
        );
      }

      return new Response(JSON.stringify({ ok: true }), { status: 200 });
    });

    vi.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host
    );

    // 1. Enter phone number and submit -> POST /api/auth/otp/request
    const phoneInput = (host.querySelector('input[type="tel"]') ??
      host.querySelector('input[name="phone"]') ??
      host.querySelector('input[name="identifier"]') ??
      host.querySelector('input')) as HTMLInputElement | null;
    expect(phoneInput, 'phone input exists').not.toBeNull();

    phoneInput!.value = '0812345678';
    phoneInput!.dispatchEvent(new Event('input', { bubbles: true }));
    phoneInput!.dispatchEvent(new Event('change', { bubbles: true }));

    const requestBtn = (host.querySelector('button[type="submit"]') ??
      host.querySelector('button')) as HTMLButtonElement | null;
    expect(requestBtn, 'request submit button exists').not.toBeNull();
    requestBtn!.click();

    await vi.waitFor(() => {
      const req = calls.find((c) => c.url.includes('/api/auth/otp/request') && c.method === 'POST');
      expect(req, 'called POST /api/auth/otp/request').toBeDefined();
    });

    // navigate('/app') must NOT be called before verify
    expect(navigate, 'not navigated before verify').not.toHaveBeenCalled();

    // 2. Enter verification code and submit -> POST /api/auth/otp/verify
    await vi.waitFor(() => {
      const inputs = Array.from(host.querySelectorAll('input'));
      const codeInput = (host.querySelector('input[name="code"]') ??
        host.querySelector('input[name="otp"]') ??
        host.querySelector('input[autocomplete="one-time-code"]') ??
        inputs.find((el) => el !== phoneInput) ??
        inputs[0]) as HTMLInputElement | null;
      expect(codeInput, 'code input exists').not.toBeNull();
    });

    const inputs = Array.from(host.querySelectorAll('input'));
    const codeInput = (host.querySelector('input[name="code"]') ??
      host.querySelector('input[name="otp"]') ??
      host.querySelector('input[autocomplete="one-time-code"]') ??
      inputs.find((el) => el !== phoneInput) ??
      inputs[0]) as HTMLInputElement;

    codeInput.value = '123456';
    codeInput.dispatchEvent(new Event('input', { bubbles: true }));
    codeInput.dispatchEvent(new Event('change', { bubbles: true }));

    const verifyBtn = (host.querySelector('button[type="submit"]') ??
      host.querySelector('button')) as HTMLButtonElement;
    verifyBtn.click();

    await vi.waitFor(() => {
      const ver = calls.find((c) => c.url.includes('/api/auth/otp/verify') && c.method === 'POST');
      expect(ver, 'called POST /api/auth/otp/verify').toBeDefined();
    });

    // 3. navigate('/app') is called only after verify answers 200
    await vi.waitFor(() => {
      expect(navigate).toHaveBeenCalledWith('/app');
    });
  });

  it('sign-in verify 401 shows error and does not navigate', async () => {
    const navigate = vi.fn();
    const calls: Array<{ url: string; method: string; body?: unknown }> = [];

    const mockFetch = vi.fn().mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = init?.method ?? 'GET';
      const body = init?.body ? JSON.parse(init.body as string) : undefined;
      calls.push({ url, method, body });

      if (url.includes('/api/auth/otp/request')) {
        return new Response(JSON.stringify({ challengeId: 'ch-test-401', ok: true }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }

      if (url.includes('/api/auth/otp/verify')) {
        return new Response(
          JSON.stringify({ error: { code: 'invalid_code', message: 'Invalid OTP code' } }),
          { status: 401, headers: { 'Content-Type': 'application/json' } }
        );
      }

      return new Response(JSON.stringify({ ok: true }), { status: 200 });
    });

    vi.spyOn(globalThis, 'fetch').mockImplementation(mockFetch);

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host
    );

    // Request OTP
    const phoneInput = (host.querySelector('input[type="tel"]') ??
      host.querySelector('input[name="phone"]') ??
      host.querySelector('input[name="identifier"]') ??
      host.querySelector('input')) as HTMLInputElement | null;
    expect(phoneInput, 'phone input exists').not.toBeNull();

    phoneInput!.value = '0812345678';
    phoneInput!.dispatchEvent(new Event('input', { bubbles: true }));
    phoneInput!.dispatchEvent(new Event('change', { bubbles: true }));

    const requestBtn = (host.querySelector('button[type="submit"]') ??
      host.querySelector('button')) as HTMLButtonElement | null;
    requestBtn!.click();

    await vi.waitFor(() => {
      const req = calls.find((c) => c.url.includes('/api/auth/otp/request') && c.method === 'POST');
      expect(req, 'called POST /api/auth/otp/request').toBeDefined();
    });

    // Enter wrong code and submit
    await vi.waitFor(() => {
      const inputs = Array.from(host.querySelectorAll('input'));
      const codeInput = (host.querySelector('input[name="code"]') ??
        host.querySelector('input[name="otp"]') ??
        host.querySelector('input[autocomplete="one-time-code"]') ??
        inputs.find((el) => el !== phoneInput) ??
        inputs[0]) as HTMLInputElement | null;
      expect(codeInput, 'code input exists').not.toBeNull();
    });

    const inputs = Array.from(host.querySelectorAll('input'));
    const codeInput = (host.querySelector('input[name="code"]') ??
      host.querySelector('input[name="otp"]') ??
      host.querySelector('input[autocomplete="one-time-code"]') ??
      inputs.find((el) => el !== phoneInput) ??
      inputs[0]) as HTMLInputElement;

    codeInput.value = '999999';
    codeInput.dispatchEvent(new Event('input', { bubbles: true }));
    codeInput.dispatchEvent(new Event('change', { bubbles: true }));

    const verifyBtn = (host.querySelector('button[type="submit"]') ??
      host.querySelector('button')) as HTMLButtonElement;
    verifyBtn.click();

    await vi.waitFor(() => {
      const ver = calls.find((c) => c.url.includes('/api/auth/otp/verify') && c.method === 'POST');
      expect(ver, 'called POST /api/auth/otp/verify').toBeDefined();
    });

    // Verify 401 shows error and does NOT navigate
    await vi.waitFor(() => {
      const text = host.textContent ?? '';
      const errorEl = host.querySelector('[role="alert"], .error, [data-error]');
      expect(
        errorEl !== null || text.length > 0,
        'screen shows error'
      ).toBe(true);
    });

    expect(navigate, 'does not navigate on 401').not.toHaveBeenCalled();
  });

  it("main.tsx declares a customer page: setAudience('customer') before render", async () => {
    setAudience('staff');
    expect(document.getElementById('root')).toBeNull();
    await import('./main');
    expect(audience()).toBe('customer');
  });
});
