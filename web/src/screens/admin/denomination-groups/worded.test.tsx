// WE-14 (#1763): screens/admin/denomination-groups/ — show failed calls through wordedError
// Tests for §4.2 (three tests per changed catch: handleSave, handleRemove, loadView, handleAddGroup)
// and §4.3 (stale data cleared on branch switch when read fails).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import { DenominationGroups } from './DenominationGroups';
import { th as screenTh } from './i18n';
import { Toast, toast } from '../parts/toast';

interface RefusalFixture {
  status: number;
  code: string;
  message: string;
}

type EndpointMock = RefusalFixture | 'network' | null;

describe('WE-14 screens/admin/denomination-groups/ worded error (#1763)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let viewMock: EndpointMock = null;
  let mapMock: EndpointMock = null;
  let removeMock: EndpointMock = null;
  let addMock: EndpointMock = null;

  const initialCurrencies = [
    {
      currency: 'USD',
      groups: [
        { group: 'LARGE', instrumentKey: 'USD_100', removable: true },
      ],
    },
  ];

  beforeEach(() => {
    setLang('th');
    viewMock = null;
    mapMock = null;
    removeMock = null;
    addMock = null;
    toast('');
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://desk.test');
        const path = url.pathname;
        const method = (init?.method ?? 'GET').toUpperCase();

        const respondWithMock = (mock: EndpointMock): Promise<Response> | null => {
          if (mock === 'network') {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
          if (mock) {
            return Promise.resolve(
              new Response(
                JSON.stringify({ error: { code: mock.code, message: mock.message } }),
                {
                  status: mock.status,
                  headers: { 'content-type': 'application/json' },
                },
              ),
            );
          }
          return null;
        };

        if (path === '/api/v3/views/admin_denomination_groups' && method === 'GET') {
          const mocked = respondWithMock(viewMock);
          if (mocked) return mocked;
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: {
                  provider: 'RATE_BOARD',
                  currencies: initialCurrencies,
                },
              }),
              { status: 200, headers: { 'content-type': 'application/json' } },
            ),
          );
        }

        if (path === '/api/denomination-groups/map' && method === 'POST') {
          const mocked = respondWithMock(mapMock);
          if (mocked) return mocked;
          return Promise.resolve(
            new Response(JSON.stringify({ ok: true }), {
              status: 200,
              headers: { 'content-type': 'application/json' },
            }),
          );
        }

        if (path === '/api/denomination-groups/remove' && method === 'POST') {
          const mocked = respondWithMock(removeMock);
          if (mocked) return mocked;
          return Promise.resolve(
            new Response(JSON.stringify({ ok: true }), {
              status: 200,
              headers: { 'content-type': 'application/json' },
            }),
          );
        }

        if (path === '/api/denomination-groups' && method === 'POST') {
          const mocked = respondWithMock(addMock);
          if (mocked) return mocked;
          return Promise.resolve(
            new Response(JSON.stringify({ ok: true }), {
              status: 200,
              headers: { 'content-type': 'application/json' },
            }),
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
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    toast('');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(initialBranch = 'MAIN'): { setBranch: (b: string) => void } {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-14', to: '2026-10-14' });
    const [shown] = createSignal(true);
    dispose = render(
      () =>
        (
          <>
            <DenominationGroups branch={branch} period={period} shown={shown} />
            <Toast />
          </>
        ) as JSX.Element,
      root,
    );
    return { setBranch };
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

  // ── Catch 1: handleSave (DenominationGroups.tsx:68) ─────────────────────────

  describe('Catch 1: handleSave (POST /api/denomination-groups/map)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      mapMock = 'network';
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const saveBtn = row.querySelector('button') as HTMLButtonElement;
      saveBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBe(sharedTh['error.network']);
      });

      setLang('en');
      saveBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal with keyed non-server code shows th["error.serverError"]', async () => {
      mapMock = {
        status: 500,
        code: 'roleNotAllowedHere',
        message: 'Internal save error 500',
      };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const saveBtn = row.querySelector('button') as HTMLButtonElement;
      saveBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      const serverMessage = 'ไม่สามารถจับคู่คีย์ป้ายเรทได้ในขณะนี้';
      mapMock = {
        status: 400,
        code: 'unmappedDenomSaveCode',
        message: serverMessage,
      };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const saveBtn = row.querySelector('button') as HTMLButtonElement;
      saveBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBe(serverMessage);
      });
    });

    it('keyed non-5xx code roleNotAllowedHere shows th["error.roleNotAllowedHere"]', async () => {
      mapMock = {
        status: 403,
        code: 'roleNotAllowedHere',
        message: 'Wire role failure',
      };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const saveBtn = row.querySelector('button') as HTMLButtonElement;
      saveBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBe(sharedTh['error.roleNotAllowedHere']);
      });
    });
  });

  // ── Catch 2: handleRemove (DenominationGroups.tsx:93) ───────────────────────

  describe('Catch 2: handleRemove (POST /api/denomination-groups/remove)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      removeMock = 'network';
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const buttons = row.querySelectorAll('button');
      const removeBtn = buttons[1] as HTMLButtonElement;
      expect(removeBtn.textContent).toBe(screenTh.remove);
      removeBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBe(sharedTh['error.network']);
      });

      setLang('en');
      removeBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal with keyed non-server code shows th["error.serverError"]', async () => {
      removeMock = {
        status: 500,
        code: 'roleNotAllowedHere',
        message: 'Internal remove error 500',
      };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const buttons = row.querySelectorAll('button');
      const removeBtn = buttons[1] as HTMLButtonElement;
      removeBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      const serverMessage = 'ไม่สามารถลบกลุ่มธนบัตรที่กำลังใช้งานอยู่ได้';
      removeMock = {
        status: 400,
        code: 'unmappedDenomRemoveCode',
        message: serverMessage,
      };
      mount();
      await selectCurrency('USD');

      const row = root.querySelector('[data-group="LARGE"]')!;
      const buttons = row.querySelectorAll('button');
      const removeBtn = buttons[1] as HTMLButtonElement;
      removeBtn.click();

      await vi.waitFor(() => {
        const errorEl = row.querySelector('.row-error');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBe(serverMessage);
      });
    });
  });

  // ── Catch 3: loadView (DenominationGroups.tsx:154) ──────────────────────────

  describe('Catch 3: loadView (GET /api/v3/views/admin_denomination_groups)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      viewMock = 'network';
      mount();

      await vi.waitFor(() => {
        const toastEl = root.querySelector('.toast');
        expect(toastEl).not.toBeNull();
        expect(toastEl!.textContent).toBe(sharedTh['error.network']);
      });

      setLang('en');
      dispose?.();
      dispose = undefined;
      root.innerHTML = '';
      toast('');
      mount();

      await vi.waitFor(() => {
        const toastEl = root.querySelector('.toast');
        expect(toastEl).not.toBeNull();
        expect(toastEl!.textContent).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal with keyed non-server code shows th["error.serverError"]', async () => {
      viewMock = {
        status: 500,
        code: 'roleNotAllowedHere',
        message: 'Internal view error 500',
      };
      mount();

      await vi.waitFor(() => {
        const toastEl = root.querySelector('.toast');
        expect(toastEl).not.toBeNull();
        expect(toastEl!.textContent).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      const serverMessage = 'ไม่สามารถดึงข้อมูลกลุ่มธนบัตรของสาขานี้ได้';
      viewMock = {
        status: 400,
        code: 'unmappedDenomViewCode',
        message: serverMessage,
      };
      mount();

      await vi.waitFor(() => {
        const toastEl = root.querySelector('.toast');
        expect(toastEl).not.toBeNull();
        expect(toastEl!.textContent).toBe(serverMessage);
      });
    });
  });

  // ── Catch 4: handleAddGroup (DenominationGroups.tsx:196) ────────────────────

  describe('Catch 4: handleAddGroup (POST /api/denomination-groups)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      addMock = 'network';
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
        expect(errorEl!.textContent).toBe(sharedTh['error.network']);
      });

      setLang('en');
      addBtn.click();

      await vi.waitFor(() => {
        const errorEl = root.querySelector('.add-group-form .error-msg');
        expect(errorEl).not.toBeNull();
        expect(errorEl!.textContent).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal with keyed non-server code shows th["error.serverError"]', async () => {
      addMock = {
        status: 500,
        code: 'roleNotAllowedHere',
        message: 'Internal add error 500',
      };
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
        expect(errorEl!.textContent).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      const serverMessage = 'รหัสกลุ่มธนบัตรนี้ถูกจองไว้ในระบบแล้ว';
      addMock = {
        status: 400,
        code: 'unmappedDenomAddCode',
        message: serverMessage,
      };
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
        expect(errorEl!.textContent).toBe(serverMessage);
      });
    });
  });

  // ── Criterion 3: Stale data cleared on branch switch when read fails ────────

  describe('Criterion 3: stale data on branch switch', () => {
    it('after a branch switch, a failed read clears the previous branch data', async () => {
      const { setBranch } = mount('MAIN');
      await selectCurrency('USD');
      expect(root.querySelector('[data-group="LARGE"]')).not.toBeNull();

      viewMock = {
        status: 500,
        code: 'roleNotAllowedHere',
        message: 'Branch B2 read failed',
      };
      setBranch('B2');

      await vi.waitFor(() => {
        expect(root.querySelector('[data-group="LARGE"]')).toBeNull();
        const toastEl = root.querySelector('.toast');
        expect(toastEl).not.toBeNull();
        expect(toastEl!.textContent).toBe(sharedTh['error.serverError']);
      });
    });
  });
});
