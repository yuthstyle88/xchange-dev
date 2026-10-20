import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { StartingPrices } from './StartingPrices';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { th as screenTh, en as screenEn } from './i18n';
import type { AdminMenuProps } from '../menus';
import { makeOrgStartingPriceItem } from '../../../shared/testing/fixtures';

const VIEW_URL = '/api/v3/views/org_starting_prices';
const SAVE_URL = '/api/v3/commands/org_starting_price_save';

const SAMPLE_ITEMS = [
  makeOrgStartingPriceItem({
    currency: 'USD',
    version: 1,
    buy: '34.00',
    sell: '34.50',
    enteredBy: 'admin-1',
    enteredAt: '2026-10-15T01:00:00Z',
    reason: 'Initial rate',
  }),
];

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function errorResponse(status: number, code: string, message: string): Response {
  return jsonResponse({ error: { code, message } }, status);
}

describe('WE-25: screens/admin/starting-prices/ — wordedError handling (#1774)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  let viewHandler: () => Response | Promise<Response>;
  let saveHandler: (body: unknown) => Response | Promise<Response>;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);

    viewHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: SAMPLE_ITEMS,
      });

    saveHandler = () =>
      jsonResponse({
        unpriced: [],
      });

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;

        if (url.includes(VIEW_URL)) {
          return viewHandler();
        }

        if (url.includes(SAVE_URL)) {
          const body = init?.body ? JSON.parse(init.body as string) : {};
          return saveHandler(body);
        }

        return jsonResponse({ error: { code: 'notFound', message: 'not found' } }, 404);
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount() {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
    const [shown] = createSignal(true);
    const props: AdminMenuProps = { branch, period, shown };
    dispose = render(() => <StartingPrices {...props} />, host);
  }

  function getLoadError(): string | null {
    const el = host.querySelector('.xc-sp-error span');
    return el ? (el.textContent ?? '').trim() : null;
  }

  function getFormError(): string | null {
    const el = host.querySelector('.xc-sp-form-error');
    return el ? (el.textContent ?? '').trim() : null;
  }

  async function fillAndSavePrice(buyVal = '34.20') {
    await vi.waitFor(() => {
      expect(host.querySelector('[data-currency="USD"]')).not.toBeNull();
    });

    const usdRow = host.querySelector('[data-currency="USD"]')!;
    const buyInput = usdRow.querySelector<HTMLInputElement>('input[data-field="buy"]')!;
    buyInput.value = buyVal;
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));

    const reasonInput = host.querySelector<HTMLInputElement>('#xc-sp-reason')!;
    reasonInput.value = 'Update USD rate';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const saveBtn = host.querySelector<HTMLButtonElement>('.xc-sp-save-btn')!;
    saveBtn.click();
  }

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 1: loadView (org_starting_prices)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 1: loadView (org_starting_prices)', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      viewHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();

      await vi.waitFor(() => {
        expect(getLoadError()).toBe(th['error.network']);
      });
      expect(getLoadError()).not.toContain('Failed to fetch');

      // In EN
      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
      setLang('en');

      mount();
      await vi.waitFor(() => {
        expect(getLoadError()).toBe(en['error.network']);
      });
      expect(getLoadError()).not.toContain('Failed to fetch');
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      viewHandler = () => errorResponse(500, 'roleNotAllowedHere', 'Internal DB failure');
      mount();

      await vi.waitFor(() => {
        expect(getLoadError()).toBe(th['error.serverError']);
      });
      expect(getLoadError()).not.toContain('Internal DB failure');
      expect(getLoadError()).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      const serverMsg = 'ราคาตั้งต้นยังไม่พร้อมใช้งานชั่วคราว';
      viewHandler = () => errorResponse(400, 'unkeyedStartingPriceCode', serverMsg);
      mount();

      await vi.waitFor(() => {
        expect(getLoadError()).toBe(serverMsg);
      });
      expect(getLoadError()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 2: handleSave (org_starting_price_save)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 2: handleSave (org_starting_price_save)', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      mount();
      saveHandler = () => Promise.reject(new TypeError('Network error'));
      await fillAndSavePrice();

      await vi.waitFor(() => {
        expect(getFormError()).toBe(th['error.network']);
      });
      expect(getFormError()).not.toContain('Network error');

      // In EN
      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
      setLang('en');

      mount();
      await fillAndSavePrice();

      await vi.waitFor(() => {
        expect(getFormError()).toBe(en['error.network']);
      });
      expect(getFormError()).not.toContain('Network error');
    });

    it('(2) 500 refusal shows error.serverError in TH', async () => {
      mount();
      saveHandler = () => errorResponse(500, 'roleNotAllowedHere', 'Save internal error');
      await fillAndSavePrice();

      await vi.waitFor(() => {
        expect(getFormError()).toBe(th['error.serverError']);
      });
      expect(getFormError()).not.toContain('Save internal error');
      expect(getFormError()).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      const serverMsg = 'ไม่อนุญาตให้แก้ไขราคาในขณะนี้';
      mount();
      saveHandler = () => errorResponse(400, 'unkeyedSaveRefusalCode', serverMsg);
      await fillAndSavePrice();

      await vi.waitFor(() => {
        expect(getFormError()).toBe(serverMsg);
      });
      expect(getFormError()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // WE4 Screen-specific wording for idempotencyConflict and validationFailed
  // ──────────────────────────────────────────────────────────────────────────
  describe('WE4 screen-specific wording', () => {
    it('idempotencyConflict uses screen-specific sentence in TH and EN', async () => {
      mount();
      saveHandler = () => errorResponse(409, 'idempotencyConflict', 'Conflict on idempotency key');
      await fillAndSavePrice();

      await vi.waitFor(() => {
        expect(getFormError()).toBe(screenTh['error.idempotencyConflict']);
      });

      // In EN
      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
      setLang('en');

      mount();
      await fillAndSavePrice();

      await vi.waitFor(() => {
        expect(getFormError()).toBe(screenEn['error.idempotencyConflict']);
      });
    });

    it('validationFailed (without field) uses screen-specific sentence in TH and EN', async () => {
      mount();
      saveHandler = () => errorResponse(400, 'validationFailed', 'Validation error');
      await fillAndSavePrice();

      await vi.waitFor(() => {
        expect(getFormError()).toBe(screenTh['error.validationFailed']);
      });

      // In EN
      dispose?.();
      dispose = undefined;
      host.innerHTML = '';
      setLang('en');

      mount();
      await fillAndSavePrice();

      await vi.waitFor(() => {
        expect(getFormError()).toBe(screenEn['error.validationFailed']);
      });
    });

    it('a 500 with code idempotencyConflict shows error.serverError (not screen text)', async () => {
      mount();
      saveHandler = () => errorResponse(500, 'idempotencyConflict', 'Conflict on 500');
      await fillAndSavePrice();

      await vi.waitFor(() => {
        expect(getFormError()).toBe(th['error.serverError']);
      });
      expect(getFormError()).not.toBe(screenTh['error.idempotencyConflict']);
    });
  });
});
