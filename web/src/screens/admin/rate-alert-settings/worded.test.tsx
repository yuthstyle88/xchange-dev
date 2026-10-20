// WE-20 (#1769): screens/admin/rate-alert-settings/ — show failed calls through wordedError
// Tests for §4.2 (three tests per changed catch: loadSettings view and handleSave command)
// and §4.3 (stale data cleared on branch switch before failed read).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { RateAlertSettings } from './RateAlertSettings';
import { makeRateAlertSettingsData, type RateAlertSettingsData } from '../../../shared/testing/fixtures';

const VIEW_URL = '/api/v3/views/rate_alert_settings';
const SAVE_URL = '/api/rate-alert-settings';

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('RateAlertSettings worded error handling (#1769)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;

  let viewHandler: (url: string) => Response | Promise<Response>;
  let saveHandler: (body: unknown) => Response | Promise<Response>;

  const defaultData: RateAlertSettingsData = makeRateAlertSettingsData({
    defaultCooldownSeconds: 86400,
    maxActivePerCustomer: 10,
    defaultLifetimeDays: 30,
    observePolicy: 'ALL_FRESH',
  });

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);

    // Default successful view returning defaultData
    viewHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: defaultData,
      });

    // Default successful save returning updated data
    saveHandler = (body) => jsonResponse(body);

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (url.includes(VIEW_URL)) {
          return viewHandler(url);
        }
        if (url.includes(SAVE_URL) && method === 'PUT') {
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
        <RateAlertSettings
          branch={branch}
          period={period}
          shown={shown}
        />
      ),
      container,
    );

    return { branch, setBranch };
  }

  function getLoadErrorText(): string | null {
    const el = container.querySelector('.rate-alert-error span');
    return el ? (el.textContent ?? '').trim() : null;
  }

  function getSaveErrorText(): string | null {
    const el = container.querySelector('.rate-alert-save-error');
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

      expect(getLoadErrorText()).toBe(th['error.network']);
      expect(getLoadErrorText()).not.toContain('Failed to fetch');

      // In EN
      dispose?.();
      setLang('en');
      mount();
      await settle();

      expect(getLoadErrorText()).toBe(en['error.network']);
      expect(getLoadErrorText()).not.toContain('Failed to fetch');
    });

    it('(2) 500 server error shows error.serverError in TH', async () => {
      viewHandler = () =>
        jsonResponse(
          { error: { code: 'serverCrash', message: 'Internal database connection dead' } },
          500,
        );
      mount();
      await settle();

      expect(getLoadErrorText()).toBe(th['error.serverError']);
      expect(getLoadErrorText()).not.toContain('Internal database connection dead');
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      viewHandler = () =>
        jsonResponse(
          { error: { code: 'customUnsharedCode', message: 'ข้อความปฏิเสธการอ่านจากระบบ' } },
          400,
        );
      mount();
      await settle();

      expect(getLoadErrorText()).toBe('ข้อความปฏิเสธการอ่านจากระบบ');
      expect(getLoadErrorText()).not.toBe(th['error.serverError']);
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

      const submitBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]');
      expect(submitBtn).not.toBeNull();
      submitBtn!.click();
      await settle();

      expect(getSaveErrorText()).toBe(th['error.network']);
      expect(getSaveErrorText()).not.toContain('Network request failed');

      // In EN
      setLang('en');
      submitBtn!.click();
      await settle();

      expect(getSaveErrorText()).toBe(en['error.network']);
      expect(getSaveErrorText()).not.toContain('Network request failed');
    });

    it('(2) 500 server error on save shows error.serverError in TH', async () => {
      mount();
      await settle();

      saveHandler = () =>
        jsonResponse(
          { error: { code: 'serverCrash', message: 'Postgres transaction aborted' } },
          500,
        );

      const submitBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]');
      expect(submitBtn).not.toBeNull();
      submitBtn!.click();
      await settle();

      expect(getSaveErrorText()).toBe(th['error.serverError']);
      expect(getSaveErrorText()).not.toContain('Postgres transaction aborted');
    });

    it('(3) refusal on save with unshared code and server message shows that message', async () => {
      mount();
      await settle();

      saveHandler = () =>
        jsonResponse(
          { error: { code: 'customSaveRefusal', message: 'การตั้งค่าเตือนราคาถูกระงับชั่วคราว' } },
          400,
        );

      const submitBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]');
      expect(submitBtn).not.toBeNull();
      submitBtn!.click();
      await settle();

      expect(getSaveErrorText()).toBe('การตั้งค่าเตือนราคาถูกระงับชั่วคราว');
      expect(getSaveErrorText()).not.toBe(th['error.serverError']);
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
      expect(container.querySelector('form')).not.toBeNull();
      const cooldownInput = container.querySelector<HTMLInputElement>('input[name="defaultCooldownHours"]');
      expect(cooldownInput?.value).toBe('24');

      // Switch to B02, where read fails
      viewHandler = () =>
        jsonResponse(
          { error: { code: 'serverCrash', message: 'Failed to read B02' } },
          500,
        );
      setBranch('B02');
      await settle();

      // Error must show
      expect(getLoadErrorText()).toBe(th['error.serverError']);
      // Previous branch B01 form and inputs must NOT stay on screen
      expect(container.querySelector('form')).toBeNull();
      expect(container.querySelector('.rate-alert-card')).toBeNull();
      expect(container.querySelector('input[name="defaultCooldownHours"]')).toBeNull();
    });
  });
});
