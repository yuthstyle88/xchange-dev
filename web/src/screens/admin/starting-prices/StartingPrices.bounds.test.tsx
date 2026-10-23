// T14 (#4913): Admin starting prices bounds and row-level refusal tests.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import {
  makeOrgStartingPriceItem,
  makeOrgStartingPriceSaveOutput,
  type OrgStartingPriceItem,
} from '../../../shared/testing/fixtures';
import type { AdminMenuProps } from '../menus';
import { Toast } from '../parts/toast';
import * as table from './i18n';
import { StartingPrices } from './StartingPrices';

const VIEW_PATH = '/api/v3/views/org_starting_prices';
const SAVE_PATH = '/api/v3/commands/org_starting_price_save';

interface SentSave {
  body: unknown;
}

describe('T14 (#4913): StartingPrices bounds and field refusals', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let saves: SentSave[] = [];
  let viewItems: OrgStartingPriceItem[];
  let saveReply: () => Response = () =>
    new Response(JSON.stringify(makeOrgStartingPriceSaveOutput({ unpriced: [] })), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    saves = [];
    viewItems = [
      makeOrgStartingPriceItem({
        currency: 'USD',
        version: 1,
        buy: '32.00',
        sell: '33.00',
        enteredBy: 'admin',
        enteredAt: '2026-10-23T01:00:00Z',
        reason: 'init',
      }),
      makeOrgStartingPriceItem({
        currency: 'EUR',
        version: 1,
        buy: '38.00',
        sell: '39.00',
        enteredBy: 'admin',
        enteredAt: '2026-10-23T01:00:00Z',
        reason: 'init',
      }),
    ];
    saveReply = () =>
      new Response(JSON.stringify(makeOrgStartingPriceSaveOutput({ unpriced: [] })), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && u.pathname === VIEW_PATH) {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: viewItems }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (method === 'POST' && u.pathname === SAVE_PATH) {
        const body = typeof init?.body === 'string' ? JSON.parse(init.body) : null;
        saves.push({ body });
        return saveReply();
      }

      return new Response(JSON.stringify({ error: { code: 'notFound', message: 'not found' } }), { status: 404 });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = null;
    container.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function mountAndWait() {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-23', to: '2026-10-23' });
    const [shown] = createSignal(true);
    const props: AdminMenuProps = { branch, period, shown };
    dispose = render(
      () => (
        <>
          <StartingPrices {...props} />
          <Toast />
        </>
      ),
      container,
    );
    await vi.waitFor(() => {
      expect(container.querySelector('[data-currency="USD"]')).not.toBeNull();
      expect(container.querySelector('.xc-sp-loading')).toBeNull();
    });
  }

  function typeInput(el: HTMLInputElement, val: string) {
    el.value = val;
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }

  it('SP1: USD buy 32.123456789 -> worded line at USD; no request', async () => {
    await mountAndWait();

    const usdRow = container.querySelector('[data-currency="USD"]')!;
    const eurRow = container.querySelector('[data-currency="EUR"]')!;
    typeInput(usdRow.querySelector<HTMLInputElement>('input[data-field="buy"]')!, '32.123456789');
    typeInput(container.querySelector<HTMLInputElement>('#xc-sp-reason')!, 'rate update');

    container.querySelector<HTMLButtonElement>('.xc-sp-save-btn')!.click();

    const buyErr = usdRow.querySelector('[data-error-for="buy"]');
    expect(buyErr).not.toBeNull();
    expect(buyErr?.textContent).toBe(table.th['validation.positiveDecimal']);
    expect(eurRow.querySelector('[data-error-for="buy"]')).toBeNull();
    expect(saves.length).toBe(0);
  });

  it('SP2: USD sell 100000001 -> worded line at USD; no request', async () => {
    await mountAndWait();

    const usdRow = container.querySelector('[data-currency="USD"]')!;
    const eurRow = container.querySelector('[data-currency="EUR"]')!;
    typeInput(usdRow.querySelector<HTMLInputElement>('input[data-field="sell"]')!, '100000001');
    typeInput(container.querySelector<HTMLInputElement>('#xc-sp-reason')!, 'rate update');

    container.querySelector<HTMLButtonElement>('.xc-sp-save-btn')!.click();

    const sellErr = usdRow.querySelector('[data-error-for="sell"]');
    expect(sellErr).not.toBeNull();
    expect(sellErr?.textContent).toBe(table.th['validation.positiveDecimal']);
    expect(eurRow.querySelector('[data-error-for="sell"]')).toBeNull();
    expect(saves.length).toBe(0);
  });

  it('SP3: add U1 -> invalidCurrencyCode at the add field', async () => {
    await mountAndWait();

    const addInput = container.querySelector<HTMLInputElement>('.xc-sp-add-code-input')!;
    const addBtn = container.querySelector<HTMLButtonElement>('.xc-sp-add-code-btn')!;
    typeInput(addInput, 'U1');
    addBtn.click();

    const addErr = container.querySelector('.xc-sp-add-error');
    expect(addErr).not.toBeNull();
    expect(addErr?.textContent).toBe(table.th['validation.invalidCurrencyCode']);
    expect(container.querySelector('[data-currency="U1"]')).toBeNull();
  });

  it('SP4: add jpy -> accepted as today', async () => {
    await mountAndWait();

    const addInput = container.querySelector<HTMLInputElement>('.xc-sp-add-code-input')!;
    const addBtn = container.querySelector<HTMLButtonElement>('.xc-sp-add-code-btn')!;
    typeInput(addInput, 'jpy');
    addBtn.click();

    expect(container.querySelector('.xc-sp-add-error')).toBeNull();
    expect(container.querySelector('[data-currency="JPY"]')).not.toBeNull();
    expect(addInput.value).toBe('');
  });

  it('SP5: USD buy 32.10 -> saved as today, and server refusal on buy/sell marks only the row being saved', async () => {
    await mountAndWait();

    const usdRow = container.querySelector('[data-currency="USD"]')!;
    const eurRow = container.querySelector('[data-currency="EUR"]')!;
    const usdBuy = usdRow.querySelector<HTMLInputElement>('input[data-field="buy"]')!;
    const reasonInput = container.querySelector<HTMLInputElement>('#xc-sp-reason')!;
    const saveBtn = container.querySelector<HTMLButtonElement>('.xc-sp-save-btn')!;

    typeInput(usdBuy, '32.10');
    typeInput(reasonInput, 'morning adjustment');

    // First verify server refusal on buy marks only USD (the row being saved) and keeps the line at that row
    saveReply = () =>
      new Response(
        JSON.stringify({ error: { code: 'validationFailed', field: 'buy', message: 'invalid buy rate' } }),
        { status: 400, headers: { 'content-type': 'application/json' } },
      );
    saveBtn.click();

    await vi.waitFor(() => {
      expect(saves.length).toBe(1);
      expect(usdRow.querySelector('[data-error-for="buy"]')?.textContent).toBe(
        table.th['validation.positiveDecimal'],
      );
    });
    expect(eurRow.querySelector('[data-error-for="buy"]')).toBeNull();
    expect(container.querySelector('.xc-sp-form-error')).toBeNull();
    expect(usdBuy.value).toBe('32.10');

    // Now let save succeed -> saved as today
    saveReply = () =>
      new Response(JSON.stringify(makeOrgStartingPriceSaveOutput({ unpriced: [] })), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    saveBtn.click();

    await vi.waitFor(() => {
      expect(saves.length).toBe(2);
    });
    expect(saves[1]!.body).toEqual({
      prices: [{ currency: 'USD', buy: '32.10', sell: '33.00' }],
      reason: 'morning adjustment',
    });
  });
});
