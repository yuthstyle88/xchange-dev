// #4924 (T24): customer sign-in and sign-up input validation & field-routed Identity refusals.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { en } from '../../../shared/i18n/en';
import { th } from '../../../shared/i18n/th';
import { SignIn } from './SignIn';
import { SignUp } from './SignUp';
import * as table from './i18n';

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

async function waitForTerms(root: HTMLElement): Promise<void> {
  await vi.waitFor(() => {
    if (!root.querySelector('[data-testid="terms-body"]')) throw new Error('TERMS text not shown yet');
    if (root.querySelector<HTMLButtonElement>('button[type="submit"]')?.disabled) throw new Error('submit disabled');
  });
}

describe('Customer SignIn / SignUp input validation (#4924)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

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
    vi.restoreAllMocks();
    setLang('th');
  });

  it('SI1: sign-up blank name, submit -> stopped at the name; no request', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn();

    dispose = render(
      () => <SignUp navigate={navigate} fetch={withTerms(mockFetch as unknown as typeof fetch)} />,
      host,
    );
    await waitForTerms(host);

    const nameInput = host.querySelector<HTMLInputElement>('input[name="displayName"]')!;
    const identifierInput = host.querySelector<HTMLInputElement>('input[name="identifier"]')!;
    const passwordInput = host.querySelector<HTMLInputElement>('input[name="password"]')!;

    nameInput.value = '   ';
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));
    identifierInput.value = '0812345678';
    identifierInput.dispatchEvent(new Event('input', { bubbles: true }));
    passwordInput.value = 'pass-1234';
    passwordInput.dispatchEvent(new Event('input', { bubbles: true }));

    host.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();

    expect(nameInput.getAttribute('aria-invalid')).toBe('true');
    expect(document.activeElement).toBe(nameInput);
    expect(mockFetch).not.toHaveBeenCalled();
    expect(navigate).not.toHaveBeenCalled();
  });

  it('SI2: sign-up 101-character name -> the input holds 100', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn();

    dispose = render(
      () => <SignUp navigate={navigate} fetch={withTerms(mockFetch as unknown as typeof fetch)} />,
      host,
    );
    await waitForTerms(host);

    const nameInput = host.querySelector<HTMLInputElement>('input[name="displayName"]')!;
    expect(nameInput.maxLength).toBe(100);

    nameInput.value = 'A'.repeat(101);
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));
    expect(nameInput.value).toHaveLength(100);
  });

  it('SI3: sign-in empty phone -> worded line; no request', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn();

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    const phoneInput = host.querySelector<HTMLInputElement>('input[name="phone"]')!;
    phoneInput.value = '   ';
    phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

    host.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();

    const phoneErr = host.querySelector('[data-field-error="phone"]');
    expect(phoneErr?.textContent?.trim()).toBe(th['input.code.phoneRequired']);
    expect(phoneInput.getAttribute('aria-invalid')).toBe('true');
    expect(mockFetch).not.toHaveBeenCalled();
  });

  it('SI4: sign-in empty code, Verify -> worded line; no new code requested', async () => {
    const navigate = vi.fn();
    const mockFetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === '/api/auth/otp/request') {
        return new Response(JSON.stringify({ challengeId: 'ch-si4' }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      return new Response('{}', { status: 404 });
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    const phoneInput = host.querySelector<HTMLInputElement>('input[name="phone"]')!;
    phoneInput.value = '0812345678';
    phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    submitBtn.click();

    await vi.waitFor(() => {
      expect(submitBtn.textContent?.trim()).toBe(table.th.verify);
    });
    expect(mockFetch).toHaveBeenCalledTimes(1);

    // Code is empty when clicking Verify
    const codeInput = host.querySelector<HTMLInputElement>('input[name="code"]')!;
    codeInput.value = '   ';
    codeInput.dispatchEvent(new Event('input', { bubbles: true }));

    submitBtn.click();

    const codeErr = host.querySelector('[data-field-error="code"]');
    expect(codeErr?.textContent?.trim()).toBe(table.th.codeRequired);
    expect(codeInput.getAttribute('aria-invalid')).toBe('true');
    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(navigate).not.toHaveBeenCalled();
  });

  it('SI5: sign-in (en) server 400 invalidCode -> the English input.code.invalidCode text at the code field', async () => {
    setLang('en');
    const navigate = vi.fn();
    const mockFetch = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url === '/api/auth/otp/request') {
        return new Response(JSON.stringify({ challengeId: 'ch-si5' }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      if (url === '/api/auth/otp/verify') {
        return new Response(
          JSON.stringify({
            error: {
              code: 'invalidCode',
              message: 'รหัสยืนยันไม่ถูกต้อง',
            },
          }),
          { status: 400, headers: { 'Content-Type': 'application/json' } },
        );
      }
      return new Response('{}', { status: 404 });
    });

    dispose = render(
      () => <SignIn navigate={navigate} fetch={mockFetch as unknown as typeof fetch} />,
      host,
    );

    const phoneInput = host.querySelector<HTMLInputElement>('input[name="phone"]')!;
    phoneInput.value = '0812345678';
    phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    submitBtn.click();

    await vi.waitFor(() => {
      expect(submitBtn.textContent?.trim()).toBe(table.en.verify);
    });

    const codeInput = host.querySelector<HTMLInputElement>('input[name="code"]')!;
    codeInput.value = '000000';
    codeInput.dispatchEvent(new Event('input', { bubbles: true }));
    submitBtn.click();

    await vi.waitFor(() => {
      const codeErr = host.querySelector('[data-field-error="code"]');
      expect(codeErr?.textContent?.trim()).toBe(en['input.code.invalidCode']);
    });
    expect(codeInput.getAttribute('aria-invalid')).toBe('true');
    expect(navigate).not.toHaveBeenCalled();
  });
});
