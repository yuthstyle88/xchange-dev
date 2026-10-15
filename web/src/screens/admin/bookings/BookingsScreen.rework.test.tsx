import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { BookingsScreen } from './BookingsScreen';
import { en, th } from './i18n';

describe('BookingsScreen rework tests (#1702)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let settingsFailure: 'network' | { status: number; body: unknown } | null = null;
  let currentSettings: unknown;

  beforeEach(() => {
    setLang('th');
    settingsFailure = null;
    currentSettings = {
      rules: { enabled: true, minutes: 30 },
      memberCount: 5,
      publicUrl: 'https://example.com/book',
      counterRates: [
        { currency: 'USD', buy: '35.50', sell: '36.00' },
        { currency: 'JPY', buy: '0.24', sell: '0.25' },
      ],
      freshness: { stale: false, sourceDate: '2026-10-14', publishedAt: '10:00' },
    };

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown) => {
        const url = String(input);

        if (url.includes('/api/v3/views/admin_booking_settings')) {
          if (settingsFailure) {
            if (settingsFailure === 'network') {
              return Promise.reject(new TypeError('Failed to fetch'));
            }
            return Promise.resolve(
              new Response(JSON.stringify(settingsFailure.body), {
                status: settingsFailure.status,
                headers: { 'content-type': 'application/json' },
              }),
            );
          }
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: currentSettings,
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
        }

        if (url.includes('/api/v3/views/admin_bookings')) {
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: [],
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
        }

        if (url.includes('/api/v3/views/admin_branches')) {
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: { branches: [{ id: 'MAIN', name: 'Main Branch' }] },
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
        }

        return Promise.resolve(
          new Response(JSON.stringify({ version: 1, changed: false }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
        );
      }),
    );

    root = document.createElement('div');
    document.body.appendChild(root);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    closeDialog();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(): HTMLElement {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '', to: '' });
    const [shown] = createSignal(true);
    dispose = render(
      () => (
        <>
          <BookingsScreen branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );
    return root;
  }

  function getCreateButton(): HTMLButtonElement {
    const buttons = [...root.querySelectorAll('button')];
    const btn = buttons.find(
      (b) => b.textContent?.includes('สร้างการจอง') || b.textContent?.includes('Create booking'),
    );
    expect(btn, 'create booking button should exist').toBeDefined();
    return btn!;
  }

  function getCurrencySelect(dialog: HTMLElement): HTMLSelectElement {
    const labels = [...dialog.querySelectorAll('label')];
    const currencyLabel = labels.find(
      (l) => l.textContent?.includes('สกุลเงิน') || l.textContent?.includes('Currency'),
    );
    expect(currencyLabel, 'currency label should exist').toBeDefined();
    const select = currencyLabel!.querySelector('select');
    expect(select, 'currency select should exist').toBeDefined();
    return select!;
  }

  describe('Row 1: settings poll failure shows worded error', () => {
    it('1. network failure on settings poll shows generic worded error', async () => {
      settingsFailure = 'network';
      mount();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedTh['error.network']);
      });

      expect(root.textContent).not.toContain('Failed to fetch');
    });

    it('2. network failure on settings poll in English shows English worded error', async () => {
      setLang('en');
      settingsFailure = 'network';
      mount();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedEn['error.network']);
      });

      expect(root.textContent).not.toContain('Failed to fetch');
    });

    it('3. refusal with known code on settings poll shows worded error for that code', async () => {
      settingsFailure = {
        status: 403,
        body: {
          error: {
            code: 'roleNotAllowed',
            message: 'raw forbidden message',
          },
        },
      };
      mount();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedTh['error.roleNotAllowed']);
      });

      expect(root.textContent).not.toContain('raw forbidden message');
    });

    it('4. refusal with unknown code on settings poll shows generic worded message, never raw code or message', async () => {
      settingsFailure = {
        status: 500,
        body: {
          error: {
            code: 'unknownCode888',
            message: 'raw internal error string',
          },
        },
      };
      mount();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedTh['error.serverError']);
      });

      expect(root.textContent).not.toContain('unknownCode888');
      expect(root.textContent).not.toContain('raw internal error string');
    });

    it('5. stale settings from previous successful read remain visible when a refresh poll fails, and the error shows', async () => {
      mount();

      // First read succeeds: member count and minutes are visible
      await vi.waitFor(() => {
        expect(root.textContent).toContain('5');
        expect(root.textContent).toContain('30');
      });

      // Now next settings poll fails
      settingsFailure = 'network';
      const refreshBtn = [...root.querySelectorAll('button')].find(
        (b) => b.textContent?.includes('โหลดการจองล่าสุด') || b.textContent?.includes('Reload latest bookings'),
      );
      expect(refreshBtn).toBeDefined();
      refreshBtn!.click();

      await vi.waitFor(() => {
        expect(root.textContent).toContain(sharedTh['error.network']);
      });

      // Stale data remains visible
      expect(root.textContent).toContain('5');
      expect(root.textContent).toContain('30');
    });
  });

  describe('Row 2: empty currency list has no preselection and shows worded empty state', () => {
    it('6. empty currency list has no EUR preselection and shows worded empty state in Thai', async () => {
      currentSettings = {
        rules: { enabled: true, minutes: 30 },
        memberCount: 0,
        publicUrl: '',
        counterRates: [],
        freshness: { stale: false, sourceDate: null, publishedAt: null },
      };

      mount();
      await vi.waitFor(() => {
        expect(root.querySelector('.admin-bookings')).not.toBeNull();
      });

      getCreateButton().click();

      await vi.waitFor(() => {
        const dialog = document.querySelector('dialog');
        expect(dialog).not.toBeNull();
      });

      const dialog = document.querySelector('dialog')!;
      const select = getCurrencySelect(dialog);

      // Must NOT preselect 'EUR'
      expect(select.value).not.toBe('EUR');
      expect(select.value).toBe('');

      // Must show worded empty state in current language
      expect(dialog.textContent).toContain(th.noCurrencies);

      // Submitting the dialog with an empty currency list must refuse with worded message
      const nameInput = dialog.querySelector('input[type="text"]') as HTMLInputElement;
      const phoneInput = dialog.querySelector('input[type="tel"]') as HTMLInputElement;
      nameInput.value = 'Somchai';
      nameInput.dispatchEvent(new Event('input', { bubbles: true }));
      phoneInput.value = '0812345678';
      phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

      const submitBtn = dialog.querySelector('button[type="submit"]') as HTMLButtonElement;
      submitBtn.click();

      await vi.waitFor(() => {
        expect(dialog.querySelector('.error')?.textContent).toBe(th.noCurrencies);
      });

      // No booking create request is sent with EUR or any currency
      const bookingCalls = (globalThis.fetch as any).mock.calls.filter((c: any[]) =>
        String(c[0]).includes('/api/v2/bookings'),
      );
      expect(bookingCalls).toHaveLength(0);
    });

    it('7. empty currency list has no EUR preselection and shows worded empty state in English', async () => {
      setLang('en');
      currentSettings = {
        rules: { enabled: true, minutes: 30 },
        memberCount: 0,
        publicUrl: '',
        counterRates: [],
        freshness: { stale: false, sourceDate: null, publishedAt: null },
      };

      mount();
      await vi.waitFor(() => {
        expect(root.querySelector('.admin-bookings')).not.toBeNull();
      });

      getCreateButton().click();

      await vi.waitFor(() => {
        const dialog = document.querySelector('dialog');
        expect(dialog).not.toBeNull();
      });

      const dialog = document.querySelector('dialog')!;
      const select = getCurrencySelect(dialog);

      // Must NOT preselect 'EUR'
      expect(select.value).not.toBe('EUR');
      expect(select.value).toBe('');

      // Must show worded empty state in English
      expect(dialog.textContent).toContain(en.noCurrencies);

      // Submitting the dialog with an empty currency list must refuse with worded message in English
      const nameInput = dialog.querySelector('input[type="text"]') as HTMLInputElement;
      const phoneInput = dialog.querySelector('input[type="tel"]') as HTMLInputElement;
      nameInput.value = 'John Doe';
      nameInput.dispatchEvent(new Event('input', { bubbles: true }));
      phoneInput.value = '0812345678';
      phoneInput.dispatchEvent(new Event('input', { bubbles: true }));

      const submitBtn = dialog.querySelector('button[type="submit"]') as HTMLButtonElement;
      submitBtn.click();

      await vi.waitFor(() => {
        expect(dialog.querySelector('.error')?.textContent).toBe(en.noCurrencies);
      });

      // No booking create request is sent with EUR or any currency
      const bookingCalls = (globalThis.fetch as any).mock.calls.filter((c: any[]) =>
        String(c[0]).includes('/api/v2/bookings'),
      );
      expect(bookingCalls).toHaveLength(0);
    });
  });
});
