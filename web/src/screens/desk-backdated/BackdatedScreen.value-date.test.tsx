// F44 up-front tests (docs/design/backdated-keying.md §15.1, §15.3 row F44-5): the form no longer guesses the
// business date. It has no valueDate input and its POST body has no valueDate key; the server dates the deal from
// paidAt under the branch's plan. Written before the implementation by the sub-controller claude/3b81c9d5.
// Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { makeDeskConfig, makeDeskRate, makeStaffCustomerRow, makeTransactionDto } from '../../shared/testing/fixtures';
import * as screen from './BackdatedScreen';

function setInput(el: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

function json(data: unknown): Response {
  return new Response(JSON.stringify(data), { status: 200, headers: { 'content-type': 'application/json' } });
}

describe('desk-backdated F44: the server dates the deal', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  it('exports no calendar-date guess', () => {
    expect((screen as Record<string, unknown>).valueDateFromPaidAt).toBeUndefined();
  });

  it('has no valueDate input, before or after a night paidAt is picked', () => {
    const api = new ApiClient(async () => json({ version: 1, changed: true }));
    dispose = render(() => <screen.BackdatedScreen api={api} />, host);
    expect(host.querySelector('input[name="valueDate"]')).toBeNull();
    setInput(host.querySelector<HTMLInputElement>('input[name="paidAt"]')!, '2026-11-05T02:59');
    expect(host.querySelector('input[name="valueDate"]')).toBeNull();
  });

  it('posts a night row with paidAt and no valueDate key', async () => {
    const bodies: Array<Record<string, unknown>> = [];
    const api = new ApiClient(async (url, init) => {
      if (url.startsWith('/api/v3/views/')) {
        return json({
          version: 1,
          changed: true,
          data: url.includes('staff_customers')
            ? [makeStaffCustomerRow({ id: 'c-108', displayName: 'Somchai' })]
            : url.includes('backdate_currencies')
              ? { currencies: [{ code: 'USD', unitSize: '1', minorUnit: 2 }] }
              : makeDeskConfig({ rates: [makeDeskRate({ code: 'USD', unitSize: '1', buy: '33.53', sell: '33.90' })] }),
        });
      }
      if (url === '/api/transactions/backdated') {
        bodies.push(typeof init?.body === 'string' ? JSON.parse(init.body) : {});
        return json(makeTransactionDto({ id: 'tx-night-1', status: 'COMPLETED', documentRef: 'XC-20261105-000001' }));
      }
      return json({});
    });
    dispose = render(() => <screen.BackdatedScreen api={api} />, host);

    setInput(host.querySelector<HTMLInputElement>('input[name="customerSearch"]')!, 'Somchai');
    host.querySelector<HTMLButtonElement>('button[type="button"]')!.click();
    await vi.waitFor(() => {
      expect(host.querySelector('[data-customer-option="c-108"]')).not.toBeNull();
    });
    host.querySelector<HTMLButtonElement>('[data-customer-option="c-108"]')!.click();
    setInput(host.querySelector<HTMLSelectElement>('select[name="side"]')!, 'BUY');
    setInput(host.querySelector<HTMLInputElement>('input[name="currency"]')!, 'USD');
    setInput(host.querySelector<HTMLInputElement>('input[name="quantity"]')!, '100');
    setInput(host.querySelector<HTMLInputElement>('input[name="rate"]')!, '33.53');
    setInput(host.querySelector<HTMLInputElement>('input[name="paidAt"]')!, '2026-11-05T02:59');
    setInput(host.querySelector<HTMLSelectElement>('select[name="method"]')!, 'Cash');
    host.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();

    await vi.waitFor(() => {
      expect(bodies).toHaveLength(1);
    });
    expect(bodies[0]!).not.toHaveProperty('valueDate');
    expect((bodies[0]!.payment as Record<string, unknown>).paidAt).toBe('2026-11-05T02:59:00+07:00');
  });
});
