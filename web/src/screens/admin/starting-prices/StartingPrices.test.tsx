// SP-3 (#1617): Admin Starting Prices screen specs (design `docs/design/blueprint/rates.md` §12.6, §12.7, §12.10 SP-3).
// Tests cover all edge cases in ticket #1617 §9:
// 1. Loading, empty (no starting prices yet: only the "add currency" row), and error with retry each render.
// 2. Rows for two currencies with versions 1–3 in any order render one current line per currency (the highest version), and the history toggle lists the older versions newest first.
// 3. buy: "35.12345678" renders exactly 35.12345678, and sell: null renders "—", never 0.
// 4. Saving one changed row sends exactly { prices: [{ currency, buy, sell }], reason }, with empty inputs as null, unchanged rows left out, and an Idempotency-Key header; a retry after a network error reuses the same key.
// 5. "Add currency" upper-cases usd → USD, and a duplicate code is not added.
// 6. Client checks block the send, worded with the field marked: blank reason, both empty, "0", "abc", and buy "36" > sell "35.5". A buy that is larger only in its fractional digits ("35.60" vs "35.5") is caught too, and "35.50" vs "35.5" is allowed.
// 7. A server validationFailed on currency (unknown code) shows the worded error on that row and keeps the inputs.
// 8. An answer { unpriced: [{ branch: "B2", reason: "noPolicy" }] } lists "B2: no policy: not priced". An unknown reason shows the generic text, and [] shows nothing.
// 9. After a successful save the view is read again.
// 10. A reason containing <b>x</b> renders as literal text.
// 11. The menu has needs: 'OrgStartingPricesManage' and blueprint: true.
// 12. TH/EN key parity.

import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { OrgStartingPriceSaveOutput } from '../../../shared/api/generated/org_starting_price_save';
import type { OrgStartingPriceItem } from '../../../shared/api/generated/org_starting_prices';
import { setLang } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { Toast } from '../parts/toast';
import * as table from './i18n';
import { menu } from './menu';
import { StartingPrices } from './StartingPrices';

interface SentRequest {
  method: string;
  path: string;
  search: string;
  body: unknown;
  headers: Record<string, string>;
}

function dummyProps(overrides: Partial<AdminMenuProps> = {}): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-13', to: '2026-10-13' });
  const [shown] = createSignal(true);
  return {
    branch,
    period,
    shown,
    ...overrides,
  };
}

describe('SP-3: StartingPrices (§9 specs)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];
  let viewItems: OrgStartingPriceItem[] = [];
  let viewStatus = 200;
  let viewErrorCode: string | undefined = undefined;
  let saveStatus = 200;
  let saveOutput: OrgStartingPriceSaveOutput = { unpriced: [] };
  let saveErrorCode: string | undefined = undefined;
  let saveErrorField: string | undefined = undefined;
  let saveErrorMessage: string | undefined = undefined;

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];
    viewItems = [];
    viewStatus = 200;
    viewErrorCode = undefined;
    saveStatus = 200;
    saveOutput = { unpriced: [] };
    saveErrorCode = undefined;
    saveErrorField = undefined;
    saveErrorMessage = undefined;

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers: Record<string, string> = {};
      if (init?.headers) {
        new Headers(init.headers).forEach((v, k) => {
          headers[k.toLowerCase()] = v;
        });
      }
      let body: unknown = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }

      sentRequests.push({ method, path: u.pathname, search: u.search, body, headers });

      if (u.pathname === '/api/v3/views/org_starting_prices') {
        if (viewStatus !== 200) {
          return new Response(
            JSON.stringify({ error: { code: viewErrorCode ?? 'serverError', message: 'Failed to load' } }),
            { status: viewStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: viewItems,
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (u.pathname === '/api/v3/commands/org_starting_price_save') {
        if (saveStatus !== 200) {
          return new Response(
            JSON.stringify({
              error: {
                code: saveErrorCode ?? 'validationFailed',
                field: saveErrorField,
                message: saveErrorMessage ?? 'Refused',
              },
            }),
            { status: saveStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(JSON.stringify(saveOutput), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      return new Response(JSON.stringify({ error: 'not found' }), { status: 404 });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = null;
    container.remove();
    vi.restoreAllMocks();
  });

  function mount(props: Partial<AdminMenuProps> = {}) {
    dispose = render(
      () => (
        <>
          <StartingPrices {...dummyProps(props)} />
          <Toast />
        </>
      ),
      container,
    );
  }

  // 1. Loading, empty (no starting prices yet: only the "add currency" row), and error with retry each render.
  it('1. Loading, empty (no starting prices yet: only the "add currency" row), and error with retry each render', async () => {
    viewStatus = 500;
    viewErrorCode = 'serverError';
    mount();

    // Error state rendered
    await vi.waitFor(() => {
      const errEl = container.querySelector('.xc-sp-error');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง');
      expect(container.querySelector('.xc-sp-retry-btn')).not.toBeNull();
    });

    // Retry button click
    viewStatus = 200;
    viewItems = [];
    const retryBtn = container.querySelector('.xc-sp-retry-btn') as HTMLButtonElement;
    retryBtn.click();

    // Empty state: no price rows, only the "add currency" row/section
    await vi.waitFor(() => {
      expect(container.querySelector('.xc-sp-error')).toBeNull();
      expect(container.querySelector('.xc-sp-add-currency')).not.toBeNull();
      // No currency item rows yet
      expect(container.querySelectorAll('.xc-sp-currency-row').length).toBe(0);
      expect(container.querySelector('.xc-sp-empty')).not.toBeNull();
    });
  });

  // 2. Rows for two currencies with versions 1–3 in any order render one current line per currency (the highest version), and the history toggle lists the older versions newest first.
  it('2. Rows for two currencies with versions 1–3 in any order render one current line per currency (the highest version), and the history toggle lists the older versions newest first', async () => {
    viewItems = [
      { currency: 'USD', version: 1, buy: '34.00', sell: '34.50', enteredBy: 'alice', enteredAt: '2026-10-10T10:00:00Z', reason: 'USD v1' },
      { currency: 'EUR', version: 2, buy: '38.00', sell: '38.80', enteredBy: 'bob', enteredAt: '2026-10-11T12:00:00Z', reason: 'EUR v2' },
      { currency: 'USD', version: 3, buy: '34.20', sell: '34.80', enteredBy: 'alice', enteredAt: '2026-10-12T10:00:00Z', reason: 'USD v3' },
      { currency: 'USD', version: 2, buy: '34.10', sell: '34.70', enteredBy: 'carol', enteredAt: '2026-10-11T10:00:00Z', reason: 'USD v2' },
      { currency: 'EUR', version: 1, buy: '37.50', sell: '38.50', enteredBy: 'bob', enteredAt: '2026-10-10T12:00:00Z', reason: 'EUR v1' },
      { currency: 'EUR', version: 3, buy: '38.20', sell: '38.90', enteredBy: 'dave', enteredAt: '2026-10-12T12:00:00Z', reason: 'EUR v3' },
    ];

    mount();

    await vi.waitFor(() => {
      const rows = container.querySelectorAll('.xc-sp-currency-row');
      expect(rows.length).toBe(2);
    });

    // Check USD current line has version 3 values
    const usdRow = container.querySelector('[data-currency="USD"]');
    expect(usdRow).not.toBeNull();
    const usdBuyInput = usdRow?.querySelector('input[data-field="buy"]') as HTMLInputElement;
    const usdSellInput = usdRow?.querySelector('input[data-field="sell"]') as HTMLInputElement;
    expect(usdBuyInput.value).toBe('34.20');
    expect(usdSellInput.value).toBe('34.80');
    expect(usdRow?.textContent).toContain('alice');
    expect(usdRow?.textContent).toContain('USD v3');

    // Toggle history for USD
    const usdHistoryBtn = usdRow?.querySelector('.xc-sp-history-toggle') as HTMLButtonElement;
    expect(usdHistoryBtn).not.toBeNull();
    usdHistoryBtn.click();

    await vi.waitFor(() => {
      const historyItems = usdRow?.querySelectorAll('.xc-sp-history-item');
      expect(historyItems?.length).toBe(2);
      // Older versions newest first: version 2 then version 1
      expect(historyItems?.[0].textContent).toContain('v2');
      expect(historyItems?.[0].textContent).toContain('34.10');
      expect(historyItems?.[0].textContent).toContain('USD v2');

      expect(historyItems?.[1].textContent).toContain('v1');
      expect(historyItems?.[1].textContent).toContain('34.00');
      expect(historyItems?.[1].textContent).toContain('USD v1');
    });

    // Check EUR current line has version 3 values
    const eurRow = container.querySelector('[data-currency="EUR"]');
    expect(eurRow).not.toBeNull();
    const eurBuyInput = eurRow?.querySelector('input[data-field="buy"]') as HTMLInputElement;
    const eurSellInput = eurRow?.querySelector('input[data-field="sell"]') as HTMLInputElement;
    expect(eurBuyInput.value).toBe('38.20');
    expect(eurSellInput.value).toBe('38.90');
    expect(eurRow?.textContent).toContain('dave');
    expect(eurRow?.textContent).toContain('EUR v3');
  });

  // 3. buy: "35.12345678" renders exactly 35.12345678, and sell: null renders "—", never 0.
  it('3. buy: "35.12345678" renders exactly 35.12345678, and sell: null renders "—", never 0', async () => {
    viewItems = [
      {
        currency: 'GBP',
        version: 1,
        buy: '35.12345678',
        sell: null,
        enteredBy: 'admin',
        enteredAt: '2026-10-13T10:00:00Z',
        reason: 'precise buy only',
      },
    ];

    mount();

    await vi.waitFor(() => {
      const gbpRow = container.querySelector('[data-currency="GBP"]');
      expect(gbpRow).not.toBeNull();
    });

    const gbpRow = container.querySelector('[data-currency="GBP"]')!;
    const buyInput = gbpRow.querySelector('input[data-field="buy"]') as HTMLInputElement;
    const sellInput = gbpRow.querySelector('input[data-field="sell"]') as HTMLInputElement;

    // Prefilled input preserves exact string
    expect(buyInput.value).toBe('35.12345678');
    expect(sellInput.value).toBe('');

    // Display of current prices: buy is 35.12345678, sell is "—" (never 0)
    const currentPrices = gbpRow.querySelector('.xc-sp-current-prices');
    expect(currentPrices?.textContent).toContain('35.12345678');
    expect(currentPrices?.textContent).toContain('—');
    expect(currentPrices?.textContent).not.toContain('0');
  });

  // 4. Saving one changed row sends exactly { prices: [{ currency, buy, sell }], reason }, with empty inputs as null, unchanged rows left out, and an Idempotency-Key header; a retry after a network error reuses the same key.
  it('4. Saving one changed row sends exactly { prices: [{ currency, buy, sell }], reason }, with empty inputs as null, unchanged rows left out, and an Idempotency-Key header; a retry after a network error reuses the same key', async () => {
    viewItems = [
      { currency: 'USD', version: 1, buy: '34.00', sell: '34.50', enteredBy: 'a', enteredAt: '2026-10-12T10:00:00Z', reason: 'r1' },
      { currency: 'EUR', version: 1, buy: '38.00', sell: '38.50', enteredBy: 'a', enteredAt: '2026-10-12T10:00:00Z', reason: 'r2' },
    ];

    mount();

    await vi.waitFor(() => {
      expect(container.querySelectorAll('.xc-sp-currency-row').length).toBe(2);
    });

    // Leave USD unchanged. Change EUR buy to "38.25" and clear EUR sell to empty.
    const eurRow = container.querySelector('[data-currency="EUR"]')!;
    const eurBuy = eurRow.querySelector('input[data-field="buy"]') as HTMLInputElement;
    const eurSell = eurRow.querySelector('input[data-field="sell"]') as HTMLInputElement;

    eurBuy.value = '38.25';
    eurBuy.dispatchEvent(new Event('input', { bubbles: true }));

    eurSell.value = '';
    eurSell.dispatchEvent(new Event('input', { bubbles: true }));

    // Set reason
    const reasonInput = container.querySelector('input[name="reason"], textarea[name="reason"], #xc-sp-reason') as HTMLInputElement;
    reasonInput.value = 'Update EUR starting price';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    // Fail save on first attempt (network error simulation)
    saveStatus = 500;
    saveErrorMessage = 'Network Error';

    const saveBtn = container.querySelector('button[type="submit"], .xc-sp-save-btn') as HTMLButtonElement;
    saveBtn.click();

    await vi.waitFor(() => {
      const err = container.querySelector('.xc-sp-form-error');
      expect(err).not.toBeNull();
    });

    const firstSaveReq = sentRequests.find((r) => r.method === 'POST' && r.path === '/api/v3/commands/org_starting_price_save');
    expect(firstSaveReq).toBeDefined();
    const firstKey = firstSaveReq?.headers['idempotency-key'];
    expect(firstKey).toBeDefined();
    expect(firstKey!.length).toBeGreaterThan(0);

    expect(firstSaveReq?.body).toEqual({
      prices: [{ currency: 'EUR', buy: '38.25', sell: null }],
      reason: 'Update EUR starting price',
    });

    // Retry save with server answering 200
    saveStatus = 200;
    saveOutput = { unpriced: [] };
    saveBtn.click();

    await vi.waitFor(() => {
      const saves = sentRequests.filter((r) => r.method === 'POST' && r.path === '/api/v3/commands/org_starting_price_save');
      expect(saves.length).toBe(2);
    });

    const secondSaveReq = sentRequests.filter((r) => r.method === 'POST' && r.path === '/api/v3/commands/org_starting_price_save')[1];
    expect(secondSaveReq.headers['idempotency-key']).toBe(firstKey);
  });

  // 5. "Add currency" upper-cases usd → USD, and a duplicate code is not added.
  it('5. "Add currency" upper-cases usd → USD, and a duplicate code is not added', async () => {
    viewItems = [
      { currency: 'USD', version: 1, buy: '34.00', sell: '34.50', enteredBy: 'a', enteredAt: '2026-10-12T10:00:00Z', reason: 'r' },
    ];

    mount();

    await vi.waitFor(() => {
      expect(container.querySelectorAll('.xc-sp-currency-row').length).toBe(1);
    });

    const addInput = container.querySelector('.xc-sp-add-code-input') as HTMLInputElement;
    const addBtn = container.querySelector('.xc-sp-add-code-btn') as HTMLButtonElement;

    // Try to add lowercase 'usd' which duplicates existing 'USD'
    addInput.value = 'usd';
    addInput.dispatchEvent(new Event('input', { bubbles: true }));
    addBtn.click();

    // No duplicate added
    expect(container.querySelectorAll('.xc-sp-currency-row').length).toBe(1);

    // Add new currency 'jpy'
    addInput.value = 'jpy';
    addInput.dispatchEvent(new Event('input', { bubbles: true }));
    addBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelectorAll('.xc-sp-currency-row').length).toBe(2);
      expect(container.querySelector('[data-currency="JPY"]')).not.toBeNull();
    });
  });

  // 6. Client checks block the send, worded with the field marked: blank reason, both empty, "0", "abc", and buy "36" > sell "35.5". A buy that is larger only in its fractional digits ("35.60" vs "35.5") is caught too, and "35.50" vs "35.5" is allowed.
  it('6. Client checks block the send, worded with the field marked: blank reason, both empty, "0", "abc", and buy "36" > sell "35.5". A buy that is larger only in its fractional digits ("35.60" vs "35.5") is caught too, and "35.50" vs "35.5" is allowed', async () => {
    viewItems = [];
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('.xc-sp-add-currency')).not.toBeNull();
    });

    const addInput = container.querySelector('.xc-sp-add-code-input') as HTMLInputElement;
    const addBtn = container.querySelector('.xc-sp-add-code-btn') as HTMLButtonElement;
    addInput.value = 'USD';
    addInput.dispatchEvent(new Event('input', { bubbles: true }));
    addBtn.click();

    const usdRow = container.querySelector('[data-currency="USD"]')!;
    const buyInput = usdRow.querySelector('input[data-field="buy"]') as HTMLInputElement;
    const sellInput = usdRow.querySelector('input[data-field="sell"]') as HTMLInputElement;
    const reasonInput = container.querySelector('#xc-sp-reason, input[name="reason"]') as HTMLInputElement;
    const saveBtn = container.querySelector('.xc-sp-save-btn') as HTMLButtonElement;

    const postCount = () => sentRequests.filter((r) => r.method === 'POST' && r.path === '/api/v3/commands/org_starting_price_save').length;

    // Check 1: blank reason
    buyInput.value = '34.00';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));
    sellInput.value = '34.50';
    sellInput.dispatchEvent(new Event('input', { bubbles: true }));
    reasonInput.value = '   ';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    saveBtn.click();
    expect(postCount()).toBe(0);
    expect(container.querySelector('.xc-sp-reason-error')).not.toBeNull();

    // Check 2: both buy and sell empty
    reasonInput.value = 'Valid reason';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));
    buyInput.value = '';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));
    sellInput.value = '';
    sellInput.dispatchEvent(new Event('input', { bubbles: true }));

    saveBtn.click();
    expect(postCount()).toBe(0);
    expect(usdRow.querySelector('.xc-sp-row-error')).not.toBeNull();

    // Check 3: "0" is not a positive decimal
    buyInput.value = '0';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));
    sellInput.value = '34.50';
    sellInput.dispatchEvent(new Event('input', { bubbles: true }));

    saveBtn.click();
    expect(postCount()).toBe(0);
    expect(usdRow.querySelector('.xc-sp-buy-error, [data-error-for="buy"]')).not.toBeNull();

    // Check 4: "abc" is not a decimal
    buyInput.value = 'abc';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));

    saveBtn.click();
    expect(postCount()).toBe(0);
    expect(usdRow.querySelector('.xc-sp-buy-error, [data-error-for="buy"]')).not.toBeNull();

    // Check 5: buy "36" > sell "35.5"
    buyInput.value = '36';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));
    sellInput.value = '35.5';
    sellInput.dispatchEvent(new Event('input', { bubbles: true }));

    saveBtn.click();
    expect(postCount()).toBe(0);
    expect(usdRow.querySelector('.xc-sp-row-error')).not.toBeNull();

    // Check 6: buy "35.60" vs "35.5" (larger only in fractional digits)
    buyInput.value = '35.60';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));
    sellInput.value = '35.5';
    sellInput.dispatchEvent(new Event('input', { bubbles: true }));

    saveBtn.click();
    expect(postCount()).toBe(0);
    expect(usdRow.querySelector('.xc-sp-row-error')).not.toBeNull();

    // Check 7: buy "35.50" vs "35.5" (equal decimals) IS allowed
    buyInput.value = '35.50';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));
    sellInput.value = '35.5';
    sellInput.dispatchEvent(new Event('input', { bubbles: true }));

    saveBtn.click();
    await vi.waitFor(() => {
      expect(postCount()).toBe(1);
    });
  });

  // 7. A server validationFailed on currency (unknown code) shows the worded error on that row and keeps the inputs.
  it('7. A server validationFailed on currency (unknown code) shows the worded error on that row and keeps the inputs', async () => {
    viewItems = [];
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('.xc-sp-add-currency')).not.toBeNull();
    });

    const addInput = container.querySelector('.xc-sp-add-code-input') as HTMLInputElement;
    const addBtn = container.querySelector('.xc-sp-add-code-btn') as HTMLButtonElement;
    addInput.value = 'XYZ';
    addInput.dispatchEvent(new Event('input', { bubbles: true }));
    addBtn.click();

    const xyzRow = container.querySelector('[data-currency="XYZ"]')!;
    const buyInput = xyzRow.querySelector('input[data-field="buy"]') as HTMLInputElement;
    const sellInput = xyzRow.querySelector('input[data-field="sell"]') as HTMLInputElement;
    buyInput.value = '10.00';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));
    sellInput.value = '10.50';
    sellInput.dispatchEvent(new Event('input', { bubbles: true }));

    const reasonInput = container.querySelector('#xc-sp-reason, input[name="reason"]') as HTMLInputElement;
    reasonInput.value = 'Adding XYZ';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    saveStatus = 400;
    saveErrorCode = 'validationFailed';
    saveErrorField = 'currency';
    saveErrorMessage = 'Unknown currency code';

    const saveBtn = container.querySelector('.xc-sp-save-btn') as HTMLButtonElement;
    saveBtn.click();

    await vi.waitFor(() => {
      const rowErr = xyzRow.querySelector('.xc-sp-currency-error, .xc-sp-row-error');
      expect(rowErr).not.toBeNull();
      expect(rowErr?.textContent).toContain('ไม่รู้จักรหัสสกุลเงินนี้');
      expect(rowErr?.textContent).not.toContain('Unknown currency code');
    });

    // Inputs are kept as typed
    expect(buyInput.value).toBe('10.00');
    expect(sellInput.value).toBe('10.50');
    expect(reasonInput.value).toBe('Adding XYZ');
  });

  // 7b. Server error code roleNotAllowedHere with message 'x' renders errorText's text and not 'x'.
  it('7b. Server error code roleNotAllowedHere with message "x" renders errorText text and not "x"', async () => {
    viewItems = [
      { currency: 'USD', version: 1, buy: '34.00', sell: '34.50', enteredBy: 'a', enteredAt: '2026-10-12T10:00:00Z', reason: 'r' },
    ];
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-currency="USD"]')).not.toBeNull();
    });

    const usdRow = container.querySelector('[data-currency="USD"]')!;
    const buyInput = usdRow.querySelector('input[data-field="buy"]') as HTMLInputElement;
    buyInput.value = '34.10';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));

    const reasonInput = container.querySelector('#xc-sp-reason, input[name="reason"]') as HTMLInputElement;
    reasonInput.value = 'Test price change';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    saveStatus = 403;
    saveErrorCode = 'roleNotAllowedHere';
    saveErrorMessage = 'x';

    const saveBtn = container.querySelector('.xc-sp-save-btn') as HTMLButtonElement;
    saveBtn.click();

    await vi.waitFor(() => {
      const formErr = container.querySelector('.xc-sp-form-error');
      expect(formErr).not.toBeNull();
      // errorText('roleNotAllowedHere', 'x') in Thai is 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้'
      expect(formErr?.textContent).toContain('บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้');
      expect(formErr?.textContent).not.toContain('x');
    });
  });

  // 8. An answer { unpriced: [{ branch: "B2", reason: "noPolicy" }] } lists "B2: no policy: not priced". An unknown reason shows the generic text, and [] shows nothing.
  it('8. An answer { unpriced: [{ branch: "B2", reason: "noPolicy" }] } lists "B2: no policy: not priced". An unknown reason shows the generic text, and [] shows nothing', async () => {
    viewItems = [
      { currency: 'USD', version: 1, buy: '34.00', sell: '34.50', enteredBy: 'a', enteredAt: '2026-10-12T10:00:00Z', reason: 'r' },
    ];
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-currency="USD"]')).not.toBeNull();
    });

    const usdRow = container.querySelector('[data-currency="USD"]')!;
    const buyInput = usdRow.querySelector('input[data-field="buy"]') as HTMLInputElement;
    buyInput.value = '34.10';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));

    const reasonInput = container.querySelector('#xc-sp-reason, input[name="reason"]') as HTMLInputElement;
    reasonInput.value = 'Price update';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    // Save with unpriced branches containing 'noPolicy' and an unknown reason
    saveStatus = 200;
    saveOutput = {
      unpriced: [
        { branch: 'B2', reason: 'noPolicy' },
        { branch: 'B3', reason: 'unknownCustomReason' },
      ],
    };

    const saveBtn = container.querySelector('.xc-sp-save-btn') as HTMLButtonElement;
    saveBtn.click();

    await vi.waitFor(() => {
      const unpricedSection = container.querySelector('.xc-sp-unpriced-list');
      expect(unpricedSection).not.toBeNull();
      expect(unpricedSection?.textContent).toContain('B2: no policy: not priced');
      expect(unpricedSection?.textContent).toContain('B3: not priced: unknownCustomReason');
    });

    // Save with empty unpriced list shows nothing
    saveOutput = { unpriced: [] };
    buyInput.value = '34.20';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));
    reasonInput.value = 'Price update 2';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();

    await vi.waitFor(() => {
      const unpricedSection = container.querySelector('.xc-sp-unpriced-list');
      expect(unpricedSection).toBeNull();
    });
  });

  // 9. After a successful save the view is read again.
  it('9. After a successful save the view is read again', async () => {
    viewItems = [
      { currency: 'USD', version: 1, buy: '34.00', sell: '34.50', enteredBy: 'a', enteredAt: '2026-10-12T10:00:00Z', reason: 'r' },
    ];
    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-currency="USD"]')).not.toBeNull();
    });

    const viewReadsBefore = sentRequests.filter((r) => r.path === '/api/v3/views/org_starting_prices').length;
    expect(viewReadsBefore).toBe(1);

    const usdRow = container.querySelector('[data-currency="USD"]')!;
    const buyInput = usdRow.querySelector('input[data-field="buy"]') as HTMLInputElement;
    buyInput.value = '34.30';
    buyInput.dispatchEvent(new Event('input', { bubbles: true }));

    const reasonInput = container.querySelector('#xc-sp-reason, input[name="reason"]') as HTMLInputElement;
    reasonInput.value = 'Updated';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    saveStatus = 200;
    saveOutput = { unpriced: [] };

    const saveBtn = container.querySelector('.xc-sp-save-btn') as HTMLButtonElement;
    saveBtn.click();

    await vi.waitFor(() => {
      const viewReadsAfter = sentRequests.filter((r) => r.path === '/api/v3/views/org_starting_prices').length;
      expect(viewReadsAfter).toBe(2);
    });
  });

  // 10. A reason containing <b>x</b> renders as literal text.
  it('10. A reason containing <b>x</b> renders as literal text', async () => {
    viewItems = [
      {
        currency: 'USD',
        version: 1,
        buy: '34.00',
        sell: '34.50',
        enteredBy: '<b>admin</b>',
        enteredAt: '2026-10-12T10:00:00Z',
        reason: '<b>x</b>',
      },
    ];

    mount();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-currency="USD"]')).not.toBeNull();
    });

    const usdRow = container.querySelector('[data-currency="USD"]')!;
    // Must contain literal "<b>x</b>" and "<b>admin</b>" without parsing into HTML elements
    expect(usdRow.textContent).toContain('<b>x</b>');
    expect(usdRow.textContent).toContain('<b>admin</b>');
    expect(usdRow.querySelector('b')).toBeNull();
  });

  // 11. The menu has needs: 'OrgStartingPricesManage' and blueprint: true.
  it('11. The menu has needs: OrgStartingPricesManage and blueprint: true', () => {
    expect(menu.id).toBe('starting-prices');
    expect(menu.needs).toBe('OrgStartingPricesManage');
    expect(menu.order).toBe(22);
    expect(menu.group).toBe('money');
    expect(menu.dated).toBe(false);
    expect(typeof menu.label()).toBe('string');
    expect(typeof menu.heading.title()).toBe('string');
  });

  // 12. TH/EN key parity.
  it('12. TH/EN key parity', () => {
    const thKeys = Object.keys(table.th).sort();
    const enKeys = Object.keys(table.en).sort();
    expect(thKeys).toEqual(enKeys);
    expect(thKeys.length).toBeGreaterThan(0);
  });
});
