// Tests for ticket #2597: Admin starting prices reason reset on save success
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { OrgStartingPriceSaveOutput } from '../../../shared/api/generated/org_starting_price_save';
import type { OrgStartingPriceItem } from '../../../shared/api/generated/org_starting_prices';
import { setLang } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { Toast } from '../parts/toast';
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

describe('Ticket #2597: StartingPrices reason reset on successful save', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];
  let viewItems: OrgStartingPriceItem[] = [];
  let viewStatus = 200;
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
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: viewItems,
          }),
          { status: viewStatus, headers: { 'content-type': 'application/json' } },
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

  // Row 1: Save succeeds -> Reason field empty
  it('row 1: Save succeeds -> Reason field empty', async () => {
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
    reasonInput.value = 'Morning desk adjustment';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    const saveBtn = container.querySelector('.xc-sp-save-btn') as HTMLButtonElement;
    saveBtn.click();

    // Wait for save and subsequent loadView reload to complete
    await vi.waitFor(() => {
      const viewReads = sentRequests.filter((r) => r.path === '/api/v3/views/org_starting_prices');
      expect(viewReads.length).toBe(2);
    });

    // Reason field must be reset to empty
    expect(reasonInput.value).toBe('');
    expect(container.querySelector('.xc-sp-reason-error')).toBeNull();
  });

  // Row 2: Then another change without typing a reason -> Refused with the reason-required message
  it('row 2: Then another change without typing a reason -> Refused with the reason-required message', async () => {
    viewItems = [
      { currency: 'USD', version: 1, buy: '34.00', sell: '34.50', enteredBy: 'a', enteredAt: '2026-10-12T10:00:00Z', reason: 'r1' },
      { currency: 'EUR', version: 1, buy: '38.00', sell: '38.50', enteredBy: 'a', enteredAt: '2026-10-12T10:00:00Z', reason: 'r2' },
    ];
    mount();

    await vi.waitFor(() => {
      expect(container.querySelectorAll('.xc-sp-currency-row').length).toBe(2);
    });

    // First save: change USD with a valid reason
    const usdRow = container.querySelector('[data-currency="USD"]')!;
    const usdBuy = usdRow.querySelector('input[data-field="buy"]') as HTMLInputElement;
    usdBuy.value = '34.20';
    usdBuy.dispatchEvent(new Event('input', { bubbles: true }));

    const reasonInput = container.querySelector('#xc-sp-reason, input[name="reason"]') as HTMLInputElement;
    reasonInput.value = 'First reason';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    // Prepare updated view data so after save, USD version 2 is loaded
    viewItems = [
      { currency: 'USD', version: 2, buy: '34.20', sell: '34.50', enteredBy: 'a', enteredAt: '2026-10-12T11:00:00Z', reason: 'First reason' },
      { currency: 'EUR', version: 1, buy: '38.00', sell: '38.50', enteredBy: 'a', enteredAt: '2026-10-12T10:00:00Z', reason: 'r2' },
    ];

    const saveBtn = container.querySelector('.xc-sp-save-btn') as HTMLButtonElement;
    saveBtn.click();

    // Wait for save and reload to finish
    await vi.waitFor(() => {
      const viewReads = sentRequests.filter((r) => r.path === '/api/v3/views/org_starting_prices');
      expect(viewReads.length).toBe(2);
    });

    // Second change: change EUR without re-typing a reason
    const eurRow = container.querySelector('[data-currency="EUR"]')!;
    const eurBuy = eurRow.querySelector('input[data-field="buy"]') as HTMLInputElement;
    eurBuy.value = '38.60';
    eurBuy.dispatchEvent(new Event('input', { bubbles: true }));

    saveBtn.click();

    // Must be refused with reason-required message, and no 2nd save request sent
    await vi.waitFor(() => {
      const reasonErr = container.querySelector('.xc-sp-reason-error');
      expect(reasonErr).not.toBeNull();
      expect(reasonErr?.textContent).toContain('กรุณาระบุเหตุผล');
    });

    const saves = sentRequests.filter((r) => r.method === 'POST' && r.path === '/api/v3/commands/org_starting_price_save');
    expect(saves.length).toBe(1);
  });

  // Row 3: Refused save keeps the reason (kept behaviour)
  it('row 3: Refused save keeps the reason', async () => {
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
    reasonInput.value = 'Kept reason on error';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    // Simulate server rejection
    saveStatus = 400;
    saveErrorCode = 'validationFailed';
    saveErrorField = 'currency';

    const saveBtn = container.querySelector('.xc-sp-save-btn') as HTMLButtonElement;
    saveBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('.xc-sp-currency-error, .xc-sp-row-error')).not.toBeNull();
    });

    // Reason must NOT be cleared when save fails
    expect(reasonInput.value).toBe('Kept reason on error');
  });

  // Edge case §9: Nothing else is reset beyond today's behaviour
  it('edge case §9: Nothing else is reset beyond today\'s behaviour', async () => {
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
    reasonInput.value = 'Unpriced test';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));

    saveStatus = 200;
    saveOutput = {
      unpriced: [{ branch: 'B2', reason: 'noPolicy' }],
    };

    const saveBtn = container.querySelector('.xc-sp-save-btn') as HTMLButtonElement;
    saveBtn.click();

    // Verify unpriced list and toast are preserved as today's behaviour
    await vi.waitFor(() => {
      const unpricedSection = container.querySelector('.xc-sp-unpriced-list');
      expect(unpricedSection).not.toBeNull();
      expect(unpricedSection?.textContent).toContain('B2: no policy: not priced');
    });

    // Reason reset
    expect(reasonInput.value).toBe('');
  });
});
