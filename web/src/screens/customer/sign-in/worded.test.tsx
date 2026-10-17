// WE-30 (#1779): screens/customer/sign-in/ — show failed calls through wordedError
// Tests for §4.2 (three tests for the changed catch in SignUp: network TypeError, 500 server error, unshared code refusal).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { SignUp } from './SignUp';

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
      () => <SignUp navigate={navigate} fetch={mockFetch} />,
      host,
    );
    return { navigate };
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
      submitBtn!.click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe('เบอร์โทรศัพท์นี้ถูกระงับการใช้งาน');
      });
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });
});
