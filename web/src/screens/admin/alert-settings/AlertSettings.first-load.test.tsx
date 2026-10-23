// R5-1 (#4893): AlertSettings disables inputs and Save until the branch's first branch_alert_settings read answers.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import {
  makeBranchAlertSettingsData,
  makeStockFloorDto,
} from '../../../shared/testing/fixtures';
import type { AdminMenuProps } from '../menus';
import { requestRefresh } from '../parts/refresh';
import { AlertSettings } from './AlertSettings';

const VIEW_PATH = '/api/v3/views/branch_alert_settings';
const SAVE_PATH = '/api/v3/commands/branch_alert_settings_save';

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe('R5-1 (#4893): AlertSettings first-load disabling', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let viewHandler: (branch: string) => Response | Promise<Response>;
  let saveBodies: Array<Record<string, unknown>>;

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    saveBodies = [];
    viewHandler = (branch) =>
      jsonResponse({
        version: 1,
        changed: true,
        data: makeBranchAlertSettingsData({ branch, staleDataMinutes: 20, floors: [], version: 1 }),
      });

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && u.pathname === VIEW_PATH) {
        return viewHandler(u.searchParams.get('branch') ?? '');
      }
      if (method === 'POST' && u.pathname === SAVE_PATH) {
        const parsed = typeof init?.body === 'string' ? JSON.parse(init.body) : {};
        saveBodies.push(parsed);
        return jsonResponse(
          makeBranchAlertSettingsData({
            branch: String(parsed.branch ?? 'B01'),
            staleDataMinutes: parsed.staleDataMinutes ?? null,
            floors: parsed.floors ?? [],
            version: 2,
          }),
        );
      }
      return jsonResponse({ error: { code: 'notFound', message: 'not found' } }, 404);
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = null;
    container.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  function mount(initialBranch = 'B01') {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-23', to: '2026-10-23' });
    const [shown] = createSignal(true);
    const props: AdminMenuProps = { branch, period, shown };
    dispose = render(() => <AlertSettings {...props} />, container);
    return { branch, setBranch };
  }

  function minutesInput() {
    return container.querySelector<HTMLInputElement>('#stale-data-minutes')!;
  }

  function addCurrencyInput() {
    return container.querySelector<HTMLInputElement>('#add-currency')!;
  }

  function addFloorBtn() {
    return container.querySelector<HTMLButtonElement>('.add-floor-button')!;
  }

  function saveBtn() {
    return container.querySelector<HTMLButtonElement>('.alert-settings-actions button')!;
  }

  it('F1: while the first read is held pending, minutes input, floor inputs, remove buttons, add-currency input, add button and Save are disabled', () => {
    const gate = deferred<Response>();
    viewHandler = () => gate.promise;
    mount();

    expect(minutesInput().disabled).toBe(true);
    expect(addCurrencyInput().disabled).toBe(true);
    expect(addFloorBtn().disabled).toBe(true);
    expect(saveBtn().disabled).toBe(true);

    addCurrencyInput().disabled = false;
    addFloorBtn().disabled = false;
    addCurrencyInput().value = 'USD';
    addCurrencyInput().dispatchEvent(new Event('input', { bubbles: true }));
    addFloorBtn().click();

    const floorIn = container.querySelector<HTMLInputElement>('[data-floor="USD"] input')!;
    const removeBtn = container.querySelector<HTMLButtonElement>('[data-floor="USD"] .remove-floor-button')!;
    expect(floorIn.disabled).toBe(true);
    expect(removeBtn.disabled).toBe(true);
  });

  it('F2: F1, then the read answers (staleDataMinutes: 20, USD floor 1000) -> all enabled; minutes shows 20', async () => {
    const gate = deferred<Response>();
    viewHandler = () => gate.promise;
    mount();

    expect(minutesInput().disabled).toBe(true);
    expect(addCurrencyInput().disabled).toBe(true);
    expect(addFloorBtn().disabled).toBe(true);
    expect(saveBtn().disabled).toBe(true);

    gate.resolve(
      jsonResponse({
        version: 1,
        changed: true,
        data: makeBranchAlertSettingsData({
          branch: 'B01',
          staleDataMinutes: 20,
          floors: [makeStockFloorDto({ currency: 'USD', minQuantity: '1000' })],
          version: 1,
        }),
      }),
    );

    await vi.waitFor(() => {
      expect(container.querySelector('[data-floor="USD"]')).not.toBeNull();
    });

    const floorIn = container.querySelector<HTMLInputElement>('[data-floor="USD"] input')!;
    const removeBtn = container.querySelector<HTMLButtonElement>('[data-floor="USD"] .remove-floor-button')!;

    expect(minutesInput().disabled).toBe(false);
    expect(addCurrencyInput().disabled).toBe(false);
    expect(addFloorBtn().disabled).toBe(false);
    expect(saveBtn().disabled).toBe(false);
    expect(floorIn.disabled).toBe(false);
    expect(removeBtn.disabled).toBe(false);
    expect(minutesInput().value).toBe('20');
  });

  it('F3: F2, then type 45 and Save -> the save sends staleDataMinutes: 45', async () => {
    const gate = deferred<Response>();
    viewHandler = () => gate.promise;
    mount();

    gate.resolve(
      jsonResponse({
        version: 1,
        changed: true,
        data: makeBranchAlertSettingsData({
          branch: 'B01',
          staleDataMinutes: 20,
          floors: [makeStockFloorDto({ currency: 'USD', minQuantity: '1000' })],
          version: 1,
        }),
      }),
    );

    await vi.waitFor(() => {
      expect(minutesInput().disabled).toBe(false);
    });

    minutesInput().value = '45';
    minutesInput().dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn().click();

    await vi.waitFor(() => {
      expect(saveBodies.length).toBe(1);
    });
    expect(saveBodies[0]!.staleDataMinutes).toBe(45);
  });

  it('F4: the first read fails (500) -> the worded load error shows as today, and the inputs and Save are enabled', async () => {
    viewHandler = () =>
      jsonResponse({ error: { code: 'serverError', message: 'Internal error' } }, 500);
    mount();

    await vi.waitFor(() => {
      const errEl = container.querySelector('.alert-settings-error');
      expect(errEl?.textContent).toBe(th['error.serverError']);
    });

    expect(minutesInput().disabled).toBe(false);
    expect(addCurrencyInput().disabled).toBe(false);
    expect(addFloorBtn().disabled).toBe(false);
    expect(saveBtn().disabled).toBe(false);

    addCurrencyInput().value = 'EUR';
    addCurrencyInput().dispatchEvent(new Event('input', { bubbles: true }));
    addFloorBtn().click();

    const floorIn = container.querySelector<HTMLInputElement>('[data-floor="EUR"] input')!;
    const removeBtn = container.querySelector<HTMLButtonElement>('[data-floor="EUR"] .remove-floor-button')!;
    expect(floorIn.disabled).toBe(false);
    expect(removeBtn.disabled).toBe(false);
  });

  it('F5: after F2, switch the branch with the new branch read held pending -> disabled again until that read answers', async () => {
    const gateA = deferred<Response>();
    const gateB = deferred<Response>();
    viewHandler = (branch) => (branch === 'B01' ? gateA.promise : gateB.promise);
    const { setBranch } = mount('B01');

    gateA.resolve(
      jsonResponse({
        version: 1,
        changed: true,
        data: makeBranchAlertSettingsData({
          branch: 'B01',
          staleDataMinutes: 20,
          floors: [makeStockFloorDto({ currency: 'USD', minQuantity: '1000' })],
          version: 1,
        }),
      }),
    );

    await vi.waitFor(() => {
      expect(minutesInput().disabled).toBe(false);
    });

    setBranch('B02');

    await vi.waitFor(() => {
      expect(minutesInput().disabled).toBe(true);
    });
    expect(addCurrencyInput().disabled).toBe(true);
    expect(addFloorBtn().disabled).toBe(true);
    expect(saveBtn().disabled).toBe(true);

    gateB.resolve(
      jsonResponse({
        version: 2,
        changed: true,
        data: makeBranchAlertSettingsData({
          branch: 'B02',
          staleDataMinutes: 35,
          floors: [makeStockFloorDto({ currency: 'EUR', minQuantity: '500' })],
          version: 2,
        }),
      }),
    );

    await vi.waitFor(() => {
      expect(minutesInput().disabled).toBe(false);
    });
    expect(addCurrencyInput().disabled).toBe(false);
    expect(addFloorBtn().disabled).toBe(false);
    expect(saveBtn().disabled).toBe(false);
    expect(minutesInput().value).toBe('35');
  });

  it('F6: after F2, a refresh-tick read is held pending -> inputs and Save stay enabled', async () => {
    const gate1 = deferred<Response>();
    const gate2 = deferred<Response>();
    let call = 0;
    viewHandler = () => (++call === 1 ? gate1.promise : gate2.promise);
    mount('B01');

    gate1.resolve(
      jsonResponse({
        version: 1,
        changed: true,
        data: makeBranchAlertSettingsData({
          branch: 'B01',
          staleDataMinutes: 20,
          floors: [makeStockFloorDto({ currency: 'USD', minQuantity: '1000' })],
          version: 1,
        }),
      }),
    );

    await vi.waitFor(() => {
      expect(minutesInput().disabled).toBe(false);
      expect(container.querySelector('[data-floor="USD"]')).not.toBeNull();
    });

    requestRefresh();

    await vi.waitFor(() => {
      expect(call).toBe(2);
    });

    const floorIn = container.querySelector<HTMLInputElement>('[data-floor="USD"] input')!;
    const removeBtn = container.querySelector<HTMLButtonElement>('[data-floor="USD"] .remove-floor-button')!;
    expect(minutesInput().disabled).toBe(false);
    expect(addCurrencyInput().disabled).toBe(false);
    expect(addFloorBtn().disabled).toBe(false);
    expect(saveBtn().disabled).toBe(false);
    expect(floorIn.disabled).toBe(false);
    expect(removeBtn.disabled).toBe(false);
  });
});
