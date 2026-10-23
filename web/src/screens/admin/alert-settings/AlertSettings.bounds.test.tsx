// T15 (#4914): Admin alert settings input bounds tests.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import {
  makeBranchAlertSettingsData,
  makeStockFloorDto,
} from '../../../shared/testing/fixtures';
import { Toast } from '../parts/toast';
import { AlertSettings } from './AlertSettings';
import * as table from './i18n';

const B01 = 'XC-B01';
const VIEW = '/api/v3/views/branch_alert_settings';
const SAVE = '/api/v3/commands/branch_alert_settings_save';

interface SentSave {
  body: Record<string, unknown>;
}

describe('T15 (#4914): AlertSettings input bounds', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let saves: SentSave[];

  beforeEach(() => {
    setLang('th');
    saves = [];
    root = document.createElement('div');
    document.body.appendChild(root);

    const initial = makeBranchAlertSettingsData({
      branch: B01,
      staleDataMinutes: 30,
      floors: [
        makeStockFloorDto({ currency: 'USD', minQuantity: '1000' }),
        makeStockFloorDto({ currency: 'JPY', minQuantity: '50000' }),
      ],
      version: 1,
    });

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && u.pathname === VIEW) {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: structuredClone(initial) }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (method === 'POST' && u.pathname === SAVE) {
        const body = typeof init?.body === 'string' ? JSON.parse(init.body) : {};
        saves.push({ body });
        return new Response(
          JSON.stringify(
            makeBranchAlertSettingsData({
              branch: B01,
              staleDataMinutes: (body.staleDataMinutes ?? null) as number | null,
              floors: Array.isArray(body.floors) ? body.floors : [],
              version: 2,
            }),
          ),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      return new Response(JSON.stringify({ version: 1, changed: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 6; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mountScreen(): Promise<void> {
    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-23', to: '2026-10-23' });
    const [shown] = createSignal(true);
    dispose = render(
      () => (
        <>
          <AlertSettings branch={branch} period={period} shown={shown} />
          <Toast />
        </>
      ),
      root,
    );
    await settle();
  }

  function typeInput(input: HTMLInputElement, value: string): void {
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function minutesInput(): HTMLInputElement {
    const el = root.querySelector<HTMLInputElement>('#stale-data-minutes');
    expect(el).not.toBeNull();
    return el!;
  }

  function saveButton(): HTMLButtonElement {
    const btn = Array.from(root.querySelectorAll<HTMLButtonElement>('button')).find(
      (b) => b.textContent?.trim() === table.th.save,
    );
    expect(btn).toBeDefined();
    return btn!;
  }

  it('AS1: minutes 12abc -> worded line; no request', async () => {
    await mountScreen();
    typeInput(minutesInput(), '12abc');
    saveButton().click();
    await settle();

    expect(saves.length).toBe(0);
    const err = root.querySelector('.alert-settings-field .alert-settings-error');
    expect(err?.textContent).toBe(table.th.invalidMinutes);
  });

  it('AS2: minutes empty -> saved as null', async () => {
    await mountScreen();
    typeInput(minutesInput(), '');
    saveButton().click();
    await settle();

    expect(saves.length).toBe(1);
    expect(saves[0]!.body.staleDataMinutes).toBeNull();
  });

  it('AS3: floor min 1.23456 -> worded line at that floor', async () => {
    await mountScreen();
    const usdRow = root.querySelector<HTMLElement>('[data-floor="USD"]')!;
    const jpyRow = root.querySelector<HTMLElement>('[data-floor="JPY"]')!;
    typeInput(usdRow.querySelector<HTMLInputElement>('input')!, '1.23456');
    saveButton().click();
    await settle();

    expect(saves.length).toBe(0);
    expect(usdRow.querySelector('.alert-settings-error')?.textContent).toBe(
      table.th.invalidFloorQuantity,
    );
    expect(jpyRow.querySelector('.alert-settings-error')).toBeNull();
  });

  it('AS4: add currency XX1 -> worded line; no floor added', async () => {
    await mountScreen();
    const addInput = root.querySelector<HTMLInputElement>('#add-currency')!;
    const addBtn = Array.from(root.querySelectorAll<HTMLButtonElement>('button')).find(
      (b) => b.textContent?.trim() === table.th.addFloor,
    )!;

    typeInput(addInput, 'XX1');
    addBtn.click();
    await settle();

    expect(root.querySelector('[data-floor="XX1"]')).toBeNull();
    expect(root.querySelector('.add-floor-section .alert-settings-error')?.textContent).toBe(
      table.th.invalidCurrencyCode,
    );
  });

  it('AS5: minutes 15 -> saved as today', async () => {
    await mountScreen();
    typeInput(minutesInput(), '15');
    saveButton().click();
    await settle();

    expect(saves.length).toBe(1);
    expect(saves[0]!.body.staleDataMinutes).toBe(15);
  });
});
