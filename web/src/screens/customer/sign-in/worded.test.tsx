// WE-30 (#1779): screens/customer/sign-in/ — show failed calls through wordedError
// Tests for §4.2 (three tests for the changed catch in SignUp: network TypeError, 500 server error, unshared code refusal).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { SignUp } from './SignUp';

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

describe('SignUp worded error handling (#1779)', () => {
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

  function mount(mockFetch: typeof fetch) {
    const navigate = vi.fn();
    dispose = render(
      () => <SignUp navigate={navigate} fetch={withTerms(mockFetch)} />,
      host,
    );
    return { navigate };
  }

  function fillValid(): void {
    const set = (sel: string, val: string) => {
      const el = host.querySelector<HTMLInputElement>(sel)!;
      el.value = val;
      el.dispatchEvent(new Event('input', { bubbles: true }));
    };
    set('input[name="displayName"]', 'Somchai');
    set('input[name="identifier"]', '0812345678');
    set('input[name="password"]', 'pass-1234');
  }

  function getErrorText(): string | null {
    const el = host.querySelector('.xc-sign-in-error');
    return el ? (el.textContent ?? '').trim() : null;
  }

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 1: submit (register call)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 1: submit register failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      const mockFetch = vi.fn().mockRejectedValue(new TypeError('Failed to fetch'));
      mount(mockFetch as unknown as typeof fetch);

      const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
      expect(submitBtn).not.toBeNull();
      await waitForTerms(host);
      fillValid();
      submitBtn!.click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(th['error.network']);
      });
      expect(getErrorText()).not.toContain('Failed to fetch');

      // In EN
      dispose?.();
      setLang('en');
      mount(mockFetch as unknown as typeof fetch);

      const submitBtnEn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
      expect(submitBtnEn).not.toBeNull();
      await waitForTerms(host);
      fillValid();
      submitBtnEn!.click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(en['error.network']);
      });
      expect(getErrorText()).not.toContain('Failed to fetch');
    });

    it('(2) 500 server error shows error.serverError in TH', async () => {
      const mockFetch = vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: { code: 'serverCrash', message: 'Internal registration database dead' },
          }),
          { status: 500 },
        ),
      );
      mount(mockFetch as unknown as typeof fetch);

      const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
      expect(submitBtn).not.toBeNull();
      await waitForTerms(host);
      fillValid();
      submitBtn!.click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(th['error.serverError']);
      });
      expect(getErrorText()).not.toContain('Internal registration database dead');
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      const mockFetch = vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: { code: 'customUnsharedCode', message: 'เบอร์โทรศัพท์นี้ถูกระงับการใช้งาน' },
          }),
          { status: 400 },
        ),
      );
      mount(mockFetch as unknown as typeof fetch);

      const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]');
      expect(submitBtn).not.toBeNull();
      await waitForTerms(host);
      fillValid();
      submitBtn!.click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe('เบอร์โทรศัพท์นี้ถูกระงับการใช้งาน');
      });
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });
});
