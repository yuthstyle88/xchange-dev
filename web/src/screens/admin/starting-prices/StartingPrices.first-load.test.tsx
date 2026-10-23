// R5-1 (#4844): StartingPrices disables inputs and Save until the first org_starting_prices read answers.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { makeOrgStartingPriceItem } from '../../../shared/testing/fixtures';
import type { AdminMenuProps } from '../menus';
import { StartingPrices } from './StartingPrices';

const VIEW_PATH = '/api/v3/views/org_starting_prices';

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

describe('R5-1 (#4844): StartingPrices first-load disabling', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let viewHandler: () => Response | Promise<Response>;
  let viewCalls = 0;

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    viewCalls = 0;
    viewHandler = () => jsonResponse({ version: 1, changed: true, data: [] });

    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      if (u.pathname === VIEW_PATH) {
        viewCalls++;
        return viewHandler();
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

  function mount() {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-23', to: '2026-10-23' });
    const [shown] = createSignal(true);
    const props: AdminMenuProps = { branch, period, shown };
    dispose = render(() => <StartingPrices {...props} />, container);
  }

  it('F1: while the first read is held pending, the add-currency input and button, every buy/sell input, and Save are disabled', () => {
    const gate = deferred<Response>();
    viewHandler = () => gate.promise;
    mount();

    expect(container.querySelector('.xc-sp-loading')).not.toBeNull();
    const addInput = container.querySelector<HTMLInputElement>('.xc-sp-add-code-input')!;
    const addBtn = container.querySelector<HTMLButtonElement>('.xc-sp-add-code-btn')!;
    const saveBtn = container.querySelector<HTMLButtonElement>('.xc-sp-save-btn')!;

    expect(addInput.disabled).toBe(true);
    expect(addBtn.disabled).toBe(true);
    expect(saveBtn.disabled).toBe(true);

    addInput.disabled = false;
    addBtn.disabled = false;
    addInput.value = 'USD';
    addInput.dispatchEvent(new Event('input', { bubbles: true }));
    addBtn.click();

    const buySellInputs = [
      ...container.querySelectorAll<HTMLInputElement>('input[data-field="buy"], input[data-field="sell"]'),
    ];
    expect(buySellInputs.length).toBe(2);
    expect(buySellInputs.every((el) => el.disabled)).toBe(true);
  });

  it('F2: F1, then the read answers with USD buy 32.10000000 -> inputs are enabled and USD buy shows the stored value', async () => {
    const gate = deferred<Response>();
    viewHandler = () => gate.promise;
    mount();

    const addInput = container.querySelector<HTMLInputElement>('.xc-sp-add-code-input')!;
    const addBtn = container.querySelector<HTMLButtonElement>('.xc-sp-add-code-btn')!;
    const saveBtn = container.querySelector<HTMLButtonElement>('.xc-sp-save-btn')!;
    expect(addInput.disabled).toBe(true);
    expect(addBtn.disabled).toBe(true);
    expect(saveBtn.disabled).toBe(true);

    gate.resolve(
      jsonResponse({
        version: 1,
        changed: true,
        data: [
          makeOrgStartingPriceItem({
            currency: 'USD',
            version: 1,
            buy: '32.10000000',
            sell: '32.50000000',
          }),
        ],
      }),
    );

    await vi.waitFor(() => {
      expect(container.querySelector('[data-currency="USD"]')).not.toBeNull();
      expect(container.querySelector('.xc-sp-loading')).toBeNull();
    });

    const usdBuy = container.querySelector<HTMLInputElement>('[data-currency="USD"] input[data-field="buy"]')!;
    const usdSell = container.querySelector<HTMLInputElement>('[data-currency="USD"] input[data-field="sell"]')!;

    expect(addInput.disabled).toBe(false);
    expect(addBtn.disabled).toBe(false);
    expect(saveBtn.disabled).toBe(false);
    expect(usdBuy.disabled).toBe(false);
    expect(usdSell.disabled).toBe(false);
    expect(usdBuy.value).toBe('32.10000000');
  });

  it('F3: F2, then type 32.10 into USD buy -> it stays 32.10 (no later read overwrites it until a save)', async () => {
    const gate = deferred<Response>();
    viewHandler = () => gate.promise;
    mount();

    gate.resolve(
      jsonResponse({
        version: 1,
        changed: true,
        data: [
          makeOrgStartingPriceItem({
            currency: 'USD',
            version: 1,
            buy: '32.10000000',
            sell: '32.50000000',
          }),
        ],
      }),
    );

    await vi.waitFor(() => {
      expect(container.querySelector('[data-currency="USD"]')).not.toBeNull();
    });

    const usdBuy = container.querySelector<HTMLInputElement>('[data-currency="USD"] input[data-field="buy"]')!;
    const saveBtn = container.querySelector<HTMLButtonElement>('.xc-sp-save-btn')!;

    usdBuy.value = '32.10';
    usdBuy.dispatchEvent(new Event('input', { bubbles: true }));

    // Even after a save refused for an empty reason, no re-read runs and the typed value stays 32.10.
    saveBtn.click();
    await new Promise((r) => setTimeout(r, 10));

    expect(usdBuy.value).toBe('32.10');
    expect(viewCalls).toBe(1);
  });

  it('F4: the first read fails (500) -> the worded load error shows as today, and the inputs are enabled (the Retry path is unchanged)', async () => {
    viewHandler = () =>
      jsonResponse({ error: { code: 'serverError', message: 'Internal error' } }, 500);
    mount();

    await vi.waitFor(() => {
      const errEl = container.querySelector('.xc-sp-error span');
      expect(errEl?.textContent).toBe(th['error.serverError']);
    });

    const addInput = container.querySelector<HTMLInputElement>('.xc-sp-add-code-input')!;
    const addBtn = container.querySelector<HTMLButtonElement>('.xc-sp-add-code-btn')!;
    const saveBtn = container.querySelector<HTMLButtonElement>('.xc-sp-save-btn')!;
    const retryBtn = container.querySelector<HTMLButtonElement>('.xc-sp-retry-btn')!;

    expect(addInput.disabled).toBe(false);
    expect(addBtn.disabled).toBe(false);
    expect(saveBtn.disabled).toBe(false);
    expect(retryBtn).not.toBeNull();

    // Adding a currency row gives enabled buy/sell inputs, and Retry preserves typed values (loadView(false)).
    addInput.value = 'EUR';
    addInput.dispatchEvent(new Event('input', { bubbles: true }));
    addBtn.click();

    const eurBuy = container.querySelector<HTMLInputElement>('[data-currency="EUR"] input[data-field="buy"]')!;
    const eurSell = container.querySelector<HTMLInputElement>('[data-currency="EUR"] input[data-field="sell"]')!;
    expect(eurBuy.disabled).toBe(false);
    expect(eurSell.disabled).toBe(false);
    eurBuy.value = '38.10';
    eurBuy.dispatchEvent(new Event('input', { bubbles: true }));

    viewHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: [makeOrgStartingPriceItem({ currency: 'EUR', version: 1, buy: '38.00000000', sell: '38.50000000' })],
      });
    retryBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('.xc-sp-error')).toBeNull();
    });
    expect(eurBuy.value).toBe('38.10');
  });
});
