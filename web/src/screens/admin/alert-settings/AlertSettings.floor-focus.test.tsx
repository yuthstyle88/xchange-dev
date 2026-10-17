import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { AlertSettings } from './AlertSettings';

const B01 = 'XC-B01';
const VIEW = '/api/v3/views/branch_alert_settings';
const SAVE = '/api/v3/commands/branch_alert_settings_save';

interface Floor {
  currency: string;
  minQuantity: string;
}

interface Settings {
  branch: string;
  staleDataMinutes: number | null;
  floors: Floor[];
  version: number | null;
}

describe('#2609 floor input focus across edits', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let stored: Record<string, Settings>;

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);

    stored = {
      [B01]: {
        branch: B01,
        staleDataMinutes: 30,
        floors: [
          { currency: 'USD', minQuantity: '1000' },
          { currency: 'JPY', minQuantity: '50000' },
        ],
        version: 4,
      },
    };

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && u.pathname === VIEW) {
        const branch = u.searchParams.get('branch') ?? '';
        const data = structuredClone(stored[branch] ?? { branch, staleDataMinutes: null, floors: [], version: null });
        return new Response(JSON.stringify({ version: data.version ?? 1, changed: true, data }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      if (method === 'POST' && u.pathname === SAVE) {
        const body = typeof init?.body === 'string' ? JSON.parse(init.body) : {};
        const branch = body.branch ?? B01;
        const current = stored[branch] ?? { branch, staleDataMinutes: null, floors: [], version: null };
        const nextVersion = (current.version ?? 0) + 1;
        const updated: Settings = {
          branch,
          staleDataMinutes: body.staleDataMinutes ?? null,
          floors: body.floors ?? [],
          version: nextVersion,
        };
        stored[branch] = updated;
        return new Response(JSON.stringify(updated), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
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
    vi.restoreAllMocks();
  });

  async function settle(): Promise<void> {
    for (let round = 0; round < 4; round++) {
      for (let i = 0; i < 5; i++) await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mountScreen(): Promise<void> {
    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown] = createSignal(true);
    dispose = render(
      () => <AlertSettings branch={branch} period={period} shown={shown} />,
      root,
    );
    await settle();
  }

  function floorInput(el: ParentNode, code: string): HTMLInputElement {
    const row = el.querySelector<HTMLElement>(`[data-floor="${code}"]`);
    expect(row, `floor row for ${code}`).not.toBeNull();
    const input = row!.querySelector<HTMLInputElement>('input');
    expect(input, `input for ${code}`).not.toBeNull();
    return input!;
  }

  it('focusing USD input and typing "1" then "2" keeps the same DOM node and activeElement focus', async () => {
    await mountScreen();
    const input = floorInput(root, 'USD');
    input.focus();
    expect(document.activeElement).toBe(input);

    // Type "1"
    input.value = '1';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();

    expect(floorInput(root, 'USD'), 'the same DOM node after typing 1').toBe(input);
    expect(document.activeElement, 'still document.activeElement after typing 1').toBe(input);
    expect(input.value).toBe('1');

    // Type "2" -> value becomes "12"
    input.value = '12';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();

    expect(floorInput(root, 'USD'), 'the same DOM node after typing 2').toBe(input);
    expect(document.activeElement, 'still document.activeElement after typing 2').toBe(input);
    expect(input.value).toBe('12');
  });

  it('removing the row above the focused one does not move the typed value to another currency', async () => {
    await mountScreen();
    const jpyInput = floorInput(root, 'JPY');
    jpyInput.focus();
    expect(document.activeElement).toBe(jpyInput);

    // Type into JPY
    jpyInput.value = '60000';
    jpyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();

    expect(floorInput(root, 'JPY'), 'same JPY input DOM node').toBe(jpyInput);
    expect(document.activeElement, 'JPY input stays focused after typing').toBe(jpyInput);
    expect(jpyInput.value).toBe('60000');

    // Remove USD (the row above JPY)
    const removeUsdBtn = root.querySelector<HTMLButtonElement>('[data-floor="USD"] .remove-floor-button');
    expect(removeUsdBtn).not.toBeNull();
    removeUsdBtn!.click();
    await settle();

    // USD is gone
    expect(root.querySelector('[data-floor="USD"]')).toBeNull();

    // JPY row still holds the typed value '60000', not USD's old value or an empty value
    const remainingJpyInput = floorInput(root, 'JPY');
    expect(remainingJpyInput.value).toBe('60000');
  });
});
