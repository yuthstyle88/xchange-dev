// WE-3 (#1752): screens/admin/alert-settings/ — show failed calls through wordedError
// Tests for §4.2 (three tests per changed catch: loadSettings view and handleSave command)
// and §4.3 (stale data cleared on branch switch before failed read).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { AlertSettings } from './AlertSettings';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';

const VIEW_URL = '/api/v3/views/branch_alert_settings';
const SAVE_URL = '/api/v3/commands/branch_alert_settings_save';

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('AlertSettings worded error handling (#1752)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;

  let viewHandler: (url: string) => Response | Promise<Response>;
  let saveHandler: (body: unknown) => Response | Promise<Response>;

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);

    // Default successful view
    viewHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: {
          branch: 'B01',
          staleDataMinutes: 15,
          floors: [{ currency: 'USD', minQuantity: '1000.00' }],
          version: 1,
        },
      });

    saveHandler = () =>
      jsonResponse({
        version: 2,
        staleDataMinutes: 15,
        floors: [{ currency: 'USD', minQuantity: '1000.00' }],
      });

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (PAGE_WIDE.some((p) => url.includes(p))) {
          return jsonResponse({ version: 1, changed: false });
        }
        if (url.includes(VIEW_URL)) {
          return viewHandler(url);
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
    container.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  async function settle(): Promise<void> {
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 10));
  }

  function mount(initialBranch = 'B01') {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-15' });
    const [shown] = createSignal(true);

    dispose = render(
      () => (
        <AlertSettings
          branch={branch}
          period={period}
          shown={shown}
        />
      ),
      container,
    );

    return { branch, setBranch };
  }

  function getErrorText(): string | null {
    const el = container.querySelector('.alert-settings-error');
    return el ? (el.textContent ?? '').trim() : null;
  }

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 1: loadSettings (view call)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 1: loadSettings view failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      viewHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();
      await settle();

      expect(getErrorText()).toBe(th['error.network']);
      expect(getErrorText()).not.toContain('Failed to fetch');

      // In EN
      dispose?.();
      setLang('en');
      mount();
      await settle();

      expect(getErrorText()).toBe(en['error.network']);
      expect(getErrorText()).not.toContain('Failed to fetch');
    });

    it('(2) 500 server error shows error.serverError in TH', async () => {
      viewHandler = () =>
        jsonResponse(
          { error: { code: 'roleNotAllowedHere', message: 'Internal database connection dead' } },
          500,
        );
      mount();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
      expect(getErrorText()).not.toContain('Internal database connection dead');
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      viewHandler = () =>
        jsonResponse(
          { error: { code: 'customUnsharedCode', message: 'ข้อความแจ้งเตือนจากระบบเฉพาะ' } },
          400,
        );
      mount();
      await settle();

      expect(getErrorText()).toBe('ข้อความแจ้งเตือนจากระบบเฉพาะ');
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 2: handleSave (command call)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 2: handleSave command failures', () => {
    it('(1) network TypeError on save shows error.network in TH and EN', async () => {
      mount();
      await settle();

      saveHandler = () => Promise.reject(new TypeError('Network request failed'));

      const saveBtn = container.querySelector<HTMLButtonElement>('.alert-settings-actions button');
      expect(saveBtn).not.toBeNull();
      saveBtn!.click();
      await settle();

      expect(getErrorText()).toBe(th['error.network']);
      expect(getErrorText()).not.toContain('Network request failed');

      // In EN
      setLang('en');
      saveBtn!.click();
      await settle();

      expect(getErrorText()).toBe(en['error.network']);
      expect(getErrorText()).not.toContain('Network request failed');
    });

    it('(2) 500 server error on save shows error.serverError in TH', async () => {
      mount();
      await settle();

      saveHandler = () =>
        jsonResponse(
          { error: { code: 'roleNotAllowedHere', message: 'Postgres transaction aborted' } },
          500,
        );

      const saveBtn = container.querySelector<HTMLButtonElement>('.alert-settings-actions button');
      expect(saveBtn).not.toBeNull();
      saveBtn!.click();
      await settle();

      expect(getErrorText()).toBe(th['error.serverError']);
      expect(getErrorText()).not.toContain('Postgres transaction aborted');
    });

    it('(3) refusal on save with unshared code and server message shows that message', async () => {
      mount();
      await settle();

      saveHandler = () =>
        jsonResponse(
          { error: { code: 'customSaveRefusal', message: 'ยอดขั้นต่ำต้องไม่เกินขีดจำกัดที่กำหนด' } },
          400,
        );

      const saveBtn = container.querySelector<HTMLButtonElement>('.alert-settings-actions button');
      expect(saveBtn).not.toBeNull();
      saveBtn!.click();
      await settle();

      expect(getErrorText()).toBe('ยอดขั้นต่ำต้องไม่เกินขีดจำกัดที่กำหนด');
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // §4.3: Branch switch stale data handling
  // ──────────────────────────────────────────────────────────────────────────
  describe('§4.3: Branch switch stale data handling', () => {
    it('after branch switch, a failed read does not leave the previous branch data showing', async () => {
      const { setBranch } = mount('B01');
      await settle();

      // B01 loaded data successfully
      expect(container.querySelector('[data-floor="USD"]')).not.toBeNull();
      const minutesInput = container.querySelector<HTMLInputElement>('#stale-data-minutes');
      expect(minutesInput?.value).toBe('15');

      // Switch to B02, where read fails
      viewHandler = () =>
        jsonResponse(
          { error: { code: 'roleNotAllowedHere', message: 'Failed to read B02' } },
          500,
        );
      setBranch('B02');
      await settle();

      // Error must show
      expect(getErrorText()).toBe(th['error.serverError']);
      // Previous branch B01 floors and minutes must NOT stay on screen
      expect(container.querySelector('[data-floor="USD"]')).toBeNull();
      expect(container.querySelector('.no-floors')).not.toBeNull();
      expect(minutesInput?.value).toBe('');
    });
  });
});
