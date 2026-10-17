import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { DenominationGroups } from './DenominationGroups';
import { th as screenTh } from './i18n';
import { Toast, toast } from '../parts/toast';

interface DeferredRequest {
  branch: string;
  path: string;
  body?: Record<string, unknown>;
  resolve: (body: unknown, status?: number) => void;
  rejectNetwork: () => void;
}

describe('Admin denomination groups currency-switch guard (#2717)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let pendingReads: DeferredRequest[];
  let pendingWrites: DeferredRequest[];

  const testData = {
    provider: 'RATE_BOARD',
    currencies: [
      {
        currency: 'USD',
        groups: [
          { group: 'USD_GRP1', instrumentKey: 'KEY_USD_1', removable: true },
          { group: 'USD_GRP2', instrumentKey: 'KEY_USD_2', removable: true },
        ],
      },
      {
        currency: 'EUR',
        groups: [
          { group: 'EUR_GRP1', instrumentKey: 'KEY_EUR_1', removable: true },
        ],
      },
    ],
  };

  function jsonResponse(status: number, body: unknown): Response {
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });
  }

  async function settle(): Promise<void> {
    for (let round = 0; round < 4; round++) {
      for (let i = 0; i < 5; i++) await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  beforeEach(() => {
    setLang('th');
    toast('');
    pendingReads = [];
    pendingWrites = [];
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://desk.test');
        const path = url.pathname;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (path === '/api/v3/views/admin_denomination_groups' && method === 'GET') {
          const branch = url.searchParams.get('branch') ?? '';
          return new Promise<Response>((resolve, reject) => {
            pendingReads.push({
              branch,
              path,
              resolve: (body: unknown, status = 200) => resolve(jsonResponse(status, body)),
              rejectNetwork: () => reject(new TypeError('Failed to fetch')),
            });
          });
        }

        if (
          (path === '/api/denomination-groups' ||
            path === '/api/denomination-groups/map' ||
            path === '/api/denomination-groups/remove') &&
          method === 'POST'
        ) {
          const parsed = init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : {};
          const branch = (parsed.branch as string) ?? '';
          return new Promise<Response>((resolve, reject) => {
            pendingWrites.push({
              branch,
              path,
              body: parsed,
              resolve: (body: unknown, status = 200) => resolve(jsonResponse(status, body)),
              rejectNetwork: () => reject(new TypeError('Failed to fetch')),
            });
          });
        }

        return Promise.resolve(jsonResponse(200, { version: 1, changed: false }));
      }),
    );
  });

  afterEach(async () => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    toast('');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
    await settle();
  });

  function mount(initialBranch = 'MAIN'): { setBranch: (b: string) => void } {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-16', to: '2026-10-16' });
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

  function selectCurrency(currency: string): void {
    const select = root.querySelector<HTMLSelectElement>('select')!;
    expect(select).not.toBeNull();
    select.value = currency;
    select.dispatchEvent(new Event('input', { bubbles: true }));
    select.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function typeNewGroup(code: string): void {
    const addInput = root.querySelector<HTMLInputElement>('.add-group-form input')!;
    expect(addInput).not.toBeNull();
    addInput.value = code;
    addInput.dispatchEvent(new Event('input', { bubbles: true }));
    addInput.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function getAddButton(): HTMLButtonElement {
    const btn = [...root.querySelectorAll<HTMLButtonElement>('.add-group-form button')].find(
      (b) => b.textContent?.trim() === screenTh.add,
    );
    expect(btn).toBeDefined();
    return btn!;
  }

  it('Case 1: Add on USD fails (bad code or 500), then the picker moves to EUR -> .add-group-form .error-msg is gone on EUR', async () => {
    mount('MAIN');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: testData });
    await settle();

    selectCurrency('USD');
    await settle();

    // Trigger an error: invalid group name
    typeNewGroup('bad code!');
    getAddButton().click();
    await settle();

    expect(root.querySelector('.add-group-form .error-msg')).not.toBeNull();

    // Switch picker to EUR
    selectCurrency('EUR');
    await settle();

    expect(root.querySelector('.add-group-form .error-msg')).toBeNull();
  });

  it('Case 2: Add on USD held, picker to EUR, type EUR_VIP, USD\'s Add succeeds -> EUR_VIP stays in the input; the view still reloads', async () => {
    mount('MAIN');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: testData });
    await settle();

    selectCurrency('USD');
    await settle();

    typeNewGroup('NEW_USD');
    getAddButton().click();
    await settle();

    expect(pendingWrites.length).toBe(1);
    expect(pendingWrites[0]!.body?.currency).toBe('USD');

    // Picker moves to EUR while USD Add is in flight
    selectCurrency('EUR');
    await settle();

    // Admin starts typing for EUR
    typeNewGroup('EUR_VIP');
    await settle();
    expect(root.querySelector<HTMLInputElement>('.add-group-form input')!.value).toBe('EUR_VIP');

    // USD Add succeeds
    pendingWrites[0]!.resolve({ ok: true }, 200);
    await settle();

    // The view reloaded
    expect(pendingReads.length).toBeGreaterThan(1);
    pendingReads[1]!.resolve({ version: 2, changed: true, data: testData });
    await settle();

    // EUR_VIP must stay in the input
    expect(root.querySelector<HTMLInputElement>('.add-group-form input')!.value).toBe('EUR_VIP');
  });

  it('Case 3: Add on USD held, picker to EUR, USD\'s Add fails (500) -> No .error-msg and no toast on EUR', async () => {
    mount('MAIN');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: testData });
    await settle();

    selectCurrency('USD');
    await settle();

    typeNewGroup('NEW_USD');
    getAddButton().click();
    await settle();

    expect(pendingWrites.length).toBe(1);

    // Switch picker to EUR
    selectCurrency('EUR');
    await settle();

    // USD Add fails with 500
    pendingWrites[0]!.resolve(
      { error: { code: 'serverError', message: 'USD Add exploded' } },
      500,
    );
    await settle();

    expect(root.querySelector('.add-group-form .error-msg')).toBeNull();
    expect(root.querySelector('.toast')?.textContent ?? '').toBe('');
  });

  it('Case 4: A USD row\'s Save or Remove held, picker to EUR, the write fails -> No toast on EUR', async () => {
    mount('MAIN');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: testData });
    await settle();

    selectCurrency('USD');
    await settle();

    const row1 = root.querySelector('[data-group="USD_GRP1"]')!;
    const row2 = root.querySelector('[data-group="USD_GRP2"]')!;
    expect(row1).not.toBeNull();
    expect(row2).not.toBeNull();

    const saveBtn = [...row1.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === screenTh.save,
    )!;
    const removeBtn = [...row2.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === screenTh.remove,
    )!;

    // Trigger save on USD_GRP1 and remove on USD_GRP2
    saveBtn.click();
    removeBtn.click();
    await settle();

    expect(pendingWrites.length).toBe(2);

    // Switch picker to EUR
    selectCurrency('EUR');
    await settle();

    // Both USD writes fail with 500
    pendingWrites[0]!.resolve(
      { error: { code: 'serverError', message: 'Save failed' } },
      500,
    );
    pendingWrites[1]!.resolve(
      { error: { code: 'serverError', message: 'Remove failed' } },
      500,
    );
    await settle();

    expect(root.querySelector('.toast')?.textContent ?? '').toBe('');
  });

  it('Case 5: Add fails on USD with no switch -> The error and toast show (kept, rework.test.tsx:352-400)', async () => {
    mount('MAIN');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: testData });
    await settle();

    selectCurrency('USD');
    await settle();

    typeNewGroup('NEW_USD');
    getAddButton().click();
    await settle();

    expect(pendingWrites.length).toBe(1);

    // USD Add fails with 500 without currency switch
    pendingWrites[0]!.resolve(
      { error: { code: 'serverError', message: 'Server error' } },
      500,
    );
    await settle();

    expect(root.querySelector('.add-group-form .error-msg')).not.toBeNull();
    expect(root.querySelector('.toast')?.textContent ?? '').not.toBe('');
  });

  it('Edge case: USD->EUR->USD while USD\'s Add is in flight: its result applies because currency matches again', async () => {
    mount('MAIN');
    pendingReads[0]!.resolve({ version: 1, changed: true, data: testData });
    await settle();

    selectCurrency('USD');
    await settle();

    typeNewGroup('NEW_USD');
    getAddButton().click();
    await settle();

    expect(pendingWrites.length).toBe(1);

    // Switch to EUR
    selectCurrency('EUR');
    await settle();

    // Switch back to USD
    selectCurrency('USD');
    await settle();

    // USD Add succeeds
    pendingWrites[0]!.resolve({ ok: true }, 200);
    await settle();

    if (pendingReads.length > 1) {
      pendingReads[1]!.resolve({ version: 2, changed: true, data: testData });
      await settle();
    }

    // Input was cleared on USD upon success
    expect(root.querySelector<HTMLInputElement>('.add-group-form input')!.value).toBe('');
    expect(root.querySelector('.add-group-form .error-msg')).toBeNull();
  });
});
