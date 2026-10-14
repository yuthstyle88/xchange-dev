import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import { DenominationGroups } from './DenominationGroups';
import { th as screenTh, en as screenEn } from './i18n';
import { Toast, toast } from '../parts/toast';

interface RefusalMock {
  status: number;
  code?: string;
  message?: string;
  networkError?: boolean;
}

describe('Admin denomination groups rework (#1698)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let serverProvider: string | null = 'RATE_BOARD';
  let viewRefusal: RefusalMock | null = null;
  let mapRefusal: RefusalMock | null = null;
  let removeRefusal: RefusalMock | null = null;
  let addRefusal: RefusalMock | null = null;
  let sentRequests: { path: string; method: string; body: Record<string, unknown> | undefined }[] = [];

  const initialData = [
    {
      currency: 'USD',
      groups: [
        { group: 'LARGE', instrumentKey: 'USD_100', removable: true },
      ],
    },
  ];

  beforeEach(() => {
    setLang('th');
    serverProvider = 'RATE_BOARD';
    viewRefusal = null;
    mapRefusal = null;
    removeRefusal = null;
    addRefusal = null;
    sentRequests = [];
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://desk.test');
        const path = url.pathname;
        const method = (init?.method ?? 'GET').toUpperCase();
        const body = typeof init?.body === 'string' && init.body !== '' ? JSON.parse(init.body) : undefined;
        sentRequests.push({ path, method, body });

        if (path === '/api/v3/views/admin_denomination_groups') {
          if (viewRefusal) {
            if (viewRefusal.networkError) {
              return Promise.reject(new TypeError('Failed to fetch'));
            }
            return Promise.resolve(
              new Response(
                JSON.stringify({ error: { code: viewRefusal.code, message: viewRefusal.message } }),
                { status: viewRefusal.status, headers: { 'content-type': 'application/json' } },
              ),
            );
          }
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: {
                  provider: serverProvider,
                  currencies: initialData,
                },
              }),
              { status: 200, headers: { 'content-type': 'application/json' } },
            ),
          );
        }

        if (path === '/api/denomination-groups/map') {
          if (mapRefusal) {
            if (mapRefusal.networkError) {
              return Promise.reject(new TypeError('Failed to fetch'));
            }
            return Promise.resolve(
              new Response(
                JSON.stringify({ error: { code: mapRefusal.code, message: mapRefusal.message } }),
                { status: mapRefusal.status, headers: { 'content-type': 'application/json' } },
              ),
            );
          }
          return Promise.resolve(
            new Response(JSON.stringify({ ok: true }), { status: 200, headers: { 'content-type': 'application/json' } }),
          );
        }

        if (path === '/api/denomination-groups/remove') {
          if (removeRefusal) {
            if (removeRefusal.networkError) {
              return Promise.reject(new TypeError('Failed to fetch'));
            }
            return Promise.resolve(
              new Response(
                JSON.stringify({ error: { code: removeRefusal.code, message: removeRefusal.message } }),
                { status: removeRefusal.status, headers: { 'content-type': 'application/json' } },
              ),
            );
          }
          return Promise.resolve(
            new Response(JSON.stringify({ ok: true }), { status: 200, headers: { 'content-type': 'application/json' } }),
          );
        }

        if (path === '/api/denomination-groups') {
          if (addRefusal) {
            if (addRefusal.networkError) {
              return Promise.reject(new TypeError('Failed to fetch'));
            }
            return Promise.resolve(
              new Response(
                JSON.stringify({ error: { code: addRefusal.code, message: addRefusal.message } }),
                { status: addRefusal.status, headers: { 'content-type': 'application/json' } },
              ),
            );
          }
          return Promise.resolve(
            new Response(JSON.stringify({ ok: true }), { status: 200, headers: { 'content-type': 'application/json' } }),
          );
        }

        return Promise.resolve(
          new Response(JSON.stringify({ version: 1, changed: false }), { status: 200, headers: { 'content-type': 'application/json' } }),
        );
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    toast('');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(): HTMLElement {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-14', to: '2026-10-14' });
    const [shown] = createSignal(true);
    dispose = render(
      () => (
        <>
          <DenominationGroups branch={branch} period={period} shown={shown} />
          <Toast />
        </>
      ) as JSX.Element,
      root,
    );
    return root;
  }

  async function selectCurrency(currency: string): Promise<void> {
    await vi.waitFor(() => {
      const option = root.querySelector(`select option[value="${currency}"]`);
      expect(option).not.toBeNull();
    });
    const select = root.querySelector('select')!;
    select.value = currency;
    select.dispatchEvent(new Event('input', { bubbles: true }));
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => {
      const row = root.querySelector('[data-group="LARGE"]');
      expect(row).not.toBeNull();
    });
  }

  describe('Row 1: require provider and refuse to save with worded message when missing (DenominationGroups.tsx:51, 220)', () => {
    it('refuses to save when provider is missing from server data, in Thai', async () => {
      serverProvider = null;
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const saveBtn = row.querySelector('button')!;
      expect(saveBtn.textContent).toBe(screenTh.save);
      saveBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBe(screenTh.providerRequired);
      });

      // Assert no POST /api/denomination-groups/map was sent
      const mapRequests = sentRequests.filter((r) => r.path === '/api/denomination-groups/map');
      expect(mapRequests.length).toBe(0);
      expect(document.body.textContent).toContain(screenTh.providerRequired);
    });

    it('refuses to save when provider is missing from server data, in English', async () => {
      setLang('en');
      serverProvider = null;
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const saveBtn = row.querySelector('button')!;
      saveBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBe(screenEn.providerRequired);
      });

      const mapRequests = sentRequests.filter((r) => r.path === '/api/denomination-groups/map');
      expect(mapRequests.length).toBe(0);
      expect(document.body.textContent).toContain(screenEn.providerRequired);
    });
  });

  describe('Row 2: errorText lookup instead of raw err.message (DenominationGroups.tsx:60, 85, 146, 187)', () => {
    it('loadView (line 146): 500 serverError shows errorText in toast, never raw code or err.message', async () => {
      viewRefusal = { status: 500, code: 'roleNotAllowedHere', message: 'RAW-server-down-500' };
      mount();

      await vi.waitFor(() => {
        const toastEl = root.querySelector('.toast');
        expect(toastEl).not.toBeNull();
        expect(toastEl!.textContent).toBeTruthy();
      });
      const toastText = root.querySelector('.toast')!.textContent!;
      expect(toastText).toContain(sharedTh['error.serverError']);
      expect(toastText).not.toContain('RAW-server-down-500');
    });

    it('loadView (line 146): 500 serverError in English shows en errorText', async () => {
      setLang('en');
      viewRefusal = { status: 500, code: 'roleNotAllowedHere', message: 'RAW-server-down-500' };
      mount();

      await vi.waitFor(() => {
        const toastEl = root.querySelector('.toast');
        expect(toastEl).not.toBeNull();
        expect(toastEl!.textContent).toBeTruthy();
      });
      const toastText = root.querySelector('.toast')!.textContent!;
      expect(toastText).toContain(sharedEn['error.serverError']);
      expect(toastText).not.toContain('RAW-server-down-500');
    });

    it('handleSave (line 60): refusal roleNotAllowed shows errorText, not raw err.message', async () => {
      mapRefusal = { status: 403, code: 'roleNotAllowed', message: 'RAW-map-refused' };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const saveBtn = row.querySelector('button')!;
      saveBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBeTruthy();
      });
      const errorText = row.querySelector('.row-error')!.textContent!;
      expect(errorText).toContain(sharedTh['error.roleNotAllowed']);
      expect(errorText).not.toContain('RAW-map-refused');
    });

    it('handleSave (line 60): 500 with keyed non-server code shows serverError, not keyed error or err.message', async () => {
      mapRefusal = { status: 500, code: 'roleNotAllowedHere', message: 'RAW-map-500' };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const saveBtn = row.querySelector('button')!;
      saveBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBeTruthy();
      });
      const errorText = row.querySelector('.row-error')!.textContent!;
      expect(errorText).toContain(sharedTh['error.serverError']);
      expect(errorText).not.toContain('RAW-map-500');
    });

    it('handleRemove (line 85): refusal serverError shows errorText, not raw err.message', async () => {
      removeRefusal = { status: 500, code: 'roleNotAllowedHere', message: 'RAW-remove-refused' };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const buttons = row.querySelectorAll('button');
      const removeBtn = buttons[1]!;
      expect(removeBtn.textContent).toBe(screenTh.remove);
      removeBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBeTruthy();
      });
      const errorText = row.querySelector('.row-error')!.textContent!;
      expect(errorText).toContain(sharedTh['error.serverError']);
      expect(errorText).not.toContain('RAW-remove-refused');
    });

    it('handleRemove (line 85): non-ApiError rejection (network failure) shows generic worded error, never TypeError or raw message', async () => {
      removeRefusal = { status: 0, networkError: true };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const buttons = row.querySelectorAll('button');
      const removeBtn = buttons[1]!;
      expect(removeBtn.textContent).toBe(screenTh.remove);
      removeBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBeTruthy();
      });
      const errorText = row.querySelector('.row-error')!.textContent!;
      expect(errorText).toContain(sharedTh['error.network']);
      expect(errorText).not.toContain('TypeError');
      expect(errorText).not.toContain('Failed to fetch');
    });

    it('handleAddGroup (line 187): refusal roleNotAllowed shows errorText, not raw err.message', async () => {
      addRefusal = { status: 403, code: 'roleNotAllowed', message: 'RAW-add-refused' };
      mount();
      await selectCurrency('USD');

      const addInput = root.querySelector('.add-group-form input') as HTMLInputElement;
      addInput.value = 'MEDIUM';
      addInput.dispatchEvent(new Event('input', { bubbles: true }));

      const addBtn = root.querySelector('.add-group-form button') as HTMLButtonElement;
      addBtn.click();

      await vi.waitFor(() => {
        const errorEl = root.querySelector('.add-group-form .error-msg');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBeTruthy();
      });
      const errorText = root.querySelector('.add-group-form .error-msg')!.textContent!;
      expect(errorText).toContain(sharedTh['error.roleNotAllowed']);
      expect(errorText).not.toContain('RAW-add-refused');
    });

    it('handleAddGroup (line 187): 500 with keyed non-server code shows serverError, not keyed error or err.message', async () => {
      addRefusal = { status: 500, code: 'roleNotAllowedHere', message: 'RAW-add-500' };
      mount();
      await selectCurrency('USD');

      const addInput = root.querySelector('.add-group-form input') as HTMLInputElement;
      addInput.value = 'MEDIUM';
      addInput.dispatchEvent(new Event('input', { bubbles: true }));

      const addBtn = root.querySelector('.add-group-form button') as HTMLButtonElement;
      addBtn.click();

      await vi.waitFor(() => {
        const errorEl = root.querySelector('.add-group-form .error-msg');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBeTruthy();
      });
      const errorText = root.querySelector('.add-group-form .error-msg')!.textContent!;
      expect(errorText).toContain(sharedTh['error.serverError']);
      expect(errorText).not.toContain('RAW-add-500');
    });

    it('handleAddGroup (line 187): non-ApiError rejection (network failure) shows generic worded error, never TypeError or raw message', async () => {
      addRefusal = { status: 0, networkError: true };
      mount();
      await selectCurrency('USD');

      const addInput = root.querySelector('.add-group-form input') as HTMLInputElement;
      addInput.value = 'MEDIUM';
      addInput.dispatchEvent(new Event('input', { bubbles: true }));

      const addBtn = root.querySelector('.add-group-form button') as HTMLButtonElement;
      addBtn.click();

      await vi.waitFor(() => {
        const errorEl = root.querySelector('.add-group-form .error-msg');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBeTruthy();
      });
      const errorText = root.querySelector('.add-group-form .error-msg')!.textContent!;
      expect(errorText).toContain(sharedTh['error.network']);
      expect(errorText).not.toContain('TypeError');
      expect(errorText).not.toContain('Failed to fetch');
    });

    it('handleSave (line 60): unknown refusal code shows server message, never raw code', async () => {
      mapRefusal = { status: 400, code: 'someUnknownCode', message: 'RAW-map-unknown-code' };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const saveBtn = row.querySelector('button')!;
      saveBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBeTruthy();
      });
      const errorText = row.querySelector('.row-error')!.textContent!;
      expect(errorText).toContain('RAW-map-unknown-code');
      expect(errorText).not.toContain('someUnknownCode');
    });

    it('handleSave (line 60): unknown refusal code in en shows server message', async () => {
      setLang('en');
      mapRefusal = { status: 400, code: 'someUnknownCode', message: 'RAW-map-unknown-code' };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const saveBtn = row.querySelector('button')!;
      saveBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBeTruthy();
      });
      const errorText = row.querySelector('.row-error')!.textContent!;
      expect(errorText).toContain('RAW-map-unknown-code');
      expect(errorText).not.toContain('someUnknownCode');
    });

    it('handleSave (line 60): non-ApiError rejection (network failure) shows generic worded error, never TypeError or raw message', async () => {
      mapRefusal = { status: 0, networkError: true };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const saveBtn = row.querySelector('button')!;
      saveBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBeTruthy();
      });
      const errorText = row.querySelector('.row-error')!.textContent!;
      expect(errorText).toContain(sharedTh['error.network']);
      expect(errorText).not.toContain('TypeError');
      expect(errorText).not.toContain('Failed to fetch');
    });

    it('loadView (line 146): unknown refusal code shows server message in toast, never raw code', async () => {
      viewRefusal = { status: 400, code: 'someUnknownCode', message: 'RAW-view-unknown-code' };
      mount();

      await vi.waitFor(() => {
        const toastEl = root.querySelector('.toast');
        expect(toastEl).not.toBeNull();
        expect(toastEl!.textContent).toBeTruthy();
      });
      const toastText = root.querySelector('.toast')!.textContent!;
      expect(toastText).toContain('RAW-view-unknown-code');
      expect(toastText).not.toContain('someUnknownCode');
    });

    it('loadView (line 146): non-ApiError rejection (network failure) shows generic worded error in toast, never TypeError', async () => {
      viewRefusal = { status: 0, networkError: true };
      mount();

      await vi.waitFor(() => {
        const toastEl = root.querySelector('.toast');
        expect(toastEl).not.toBeNull();
        expect(toastEl!.textContent).toBeTruthy();
      });
      const toastText = root.querySelector('.toast')!.textContent!;
      expect(toastText).toContain(sharedTh['error.network']);
      expect(toastText).not.toContain('TypeError');
      expect(toastText).not.toContain('Failed to fetch');
    });

    it('loadView (line 146): after branch switch, a failed read clears previous branch groups', async () => {
      const [branch, setBranch] = createSignal('MAIN');
      const [period] = createSignal({ from: '2026-10-14', to: '2026-10-14' });
      const [shown] = createSignal(true);
      dispose = render(
        () => (
          <>
            <DenominationGroups branch={branch} period={period} shown={shown} />
            <Toast />
          </>
        ) as JSX.Element,
        root,
      );

      await selectCurrency('USD');
      expect(root.querySelector('[data-group="LARGE"]')).not.toBeNull();

      viewRefusal = { status: 500, code: 'roleNotAllowedHere', message: 'Internal Server Error' };
      setBranch('B2');

      await vi.waitFor(() => {
        expect(root.querySelector('[data-group="LARGE"]')).toBeNull();
      });
    });

    it('handleRemove (line 85): unknown refusal code shows server message, not raw code', async () => {
      removeRefusal = { status: 400, code: 'someUnknownCode', message: 'RAW-remove-unknown-code' };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const buttons = row.querySelectorAll('button');
      const removeBtn = buttons[1]!;
      removeBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBeTruthy();
      });
      const errorText = row.querySelector('.row-error')!.textContent!;
      expect(errorText).toContain('RAW-remove-unknown-code');
      expect(errorText).not.toContain('someUnknownCode');
    });

    it('handleAddGroup (line 187): unknown refusal code shows server message, not raw code', async () => {
      addRefusal = { status: 400, code: 'someUnknownCode', message: 'RAW-add-unknown-code' };
      mount();
      await selectCurrency('USD');

      const addInput = root.querySelector('.add-group-form input') as HTMLInputElement;
      addInput.value = 'MEDIUM';
      addInput.dispatchEvent(new Event('input', { bubbles: true }));

      const addBtn = root.querySelector('.add-group-form button') as HTMLButtonElement;
      addBtn.click();

      await vi.waitFor(() => {
        const errorEl = root.querySelector('.add-group-form .error-msg');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBeTruthy();
      });
      const errorText = root.querySelector('.add-group-form .error-msg')!.textContent!;
      expect(errorText).toContain('RAW-add-unknown-code');
      expect(errorText).not.toContain('someUnknownCode');
    });
  });
});
