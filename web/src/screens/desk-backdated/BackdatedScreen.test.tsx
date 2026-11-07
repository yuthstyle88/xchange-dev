import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import {
  makeDeskConfig,
  makeDeskRate,
  makeStaffCustomerRow,
  makeTransactionDto,
} from '../../shared/testing/fixtures';
import {
  BackdatedScreen,
  computeSettleThb,
  computeThb,
  toBangkokRfc3339,
} from './BackdatedScreen';
import * as i18n from './i18n';

function setInput(el: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('desk-backdated (BK-11 #6146)', () => {
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

  it('i18n exports th and en with matching keys and no zh', () => {
    const mod = i18n as Record<string, unknown>;
    expect(mod.zh).toBeUndefined();
    expect(Object.keys(i18n.th).sort()).toEqual(Object.keys(i18n.en).sort());
  });

  describe('1. computed THB (quantity × rate ÷ unitSize, HALF_UP 2 dp)', () => {
    it('computes THB and settle THB with HALF_UP rounding to 2 decimal places', () => {
      // B1: 500 USD at 32.15 per 1 -> 16075.00
      expect(computeThb('500', '32.15', '1')).toBe('16075.00');
      // B17: 10000 JPY at 0.2155 per 100 -> 21.55
      expect(computeThb('10000', '0.2155', '100')).toBe('21.55');
      // HALF_UP midpoint (.005 rounds away from zero to .01; .004 rounds to .00)
      expect(computeThb('1', '1.005', '1')).toBe('1.01');
      expect(computeThb('1', '1.004', '1')).toBe('1.00');
      expect(computeThb('3', '10.005', '1')).toBe('30.02');
      // Settle THB: BUY subtracts fee, SELL adds fee
      expect(computeSettleThb('16075.00', '25.00', 'BUY')).toBe('16050.00');
      expect(computeSettleThb('16075.00', '25.00', 'SELL')).toBe('16100.00');
    });

    // F46 (docs/design/backdated-keying.md §15.2): units come from backdate_currencies, not desk_config. Restated by the
    // ticket author claude/3b81c9d5; the new rows are in BackdatedScreen.units.test.tsx.
    it('updates live computed THB in the form using unitSize from backdate_currencies', async () => {
      const api = new ApiClient(async (url) => {
        if (url.startsWith('/api/v3/views/backdate_currencies')) {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: {
                currencies: [
                  { code: 'JPY', unitSize: '100', minorUnit: 0 },
                  { code: 'USD', unitSize: '1', minorUnit: 2 },
                ],
              },
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (url.startsWith('/api/v3/views/desk_config')) {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: makeDeskConfig({
                rates: [
                  makeDeskRate({ code: 'USD', unitSize: '1', buy: '32.00', sell: '32.50' }),
                  makeDeskRate({ code: 'JPY', unitSize: '100', buy: '21.00', sell: '22.00' }),
                ],
              }),
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response('{}', { status: 200 });
      });

      dispose = render(() => <BackdatedScreen api={api} />, host);
      await vi.waitFor(() => {
        expect(host.querySelector('datalist option[value="JPY"]')).not.toBeNull();
      });

      const currencyInput = host.querySelector<HTMLInputElement>('input[name="currency"]')!;
      const qtyInput = host.querySelector<HTMLInputElement>('input[name="quantity"]')!;
      const rateInput = host.querySelector<HTMLInputElement>('input[name="rate"]')!;

      setInput(currencyInput, 'JPY');
      setInput(qtyInput, '10000');
      setInput(rateInput, '21.555');

      // 10000 * 21.555 / 100 = 2155.50
      expect(host.querySelector('[data-testid="computed-thb"]')?.textContent).toContain('2155.50 THB');
    });
  });

  // F44 (docs/design/backdated-keying.md §15.1): the form no longer pre-fills valueDate from the calendar date of
  // paidAt, so the pre-fill test is gone and the body below has no valueDate. Restated by the ticket author
  // claude/3b81c9d5; the new behaviour is in BackdatedScreen.value-date.test.tsx.
  describe('2. paidAt formatting', () => {
    it('formats Asia/Bangkok RFC 3339 with +07:00', () => {
      expect(toBangkokRfc3339('2026-11-04T13:23')).toBe('2026-11-04T13:23:00+07:00');
      expect(toBangkokRfc3339('2026-11-04T13:23:45')).toBe('2026-11-04T13:23:45+07:00');
    });
  });

  describe('3. request body sent to /api/transactions/backdated, receipt link, and field errors', () => {
    it('posts TransactionBackdateInput with a fresh Idempotency-Key and shows documentRef + receipt link', async () => {
      const calls: Array<{ url: string; method: string; key: string | null; body: unknown }> = [];

      const api = new ApiClient(async (url, init) => {
        const method = (init?.method ?? 'GET').toUpperCase();
        const headers = new Headers(init?.headers);
        if (url.startsWith('/api/v3/views/desk_config')) {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: makeDeskConfig({
                rates: [makeDeskRate({ code: 'USD', unitSize: '1', buy: '32.15', sell: '32.50' })],
              }),
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (url.startsWith('/api/v3/views/staff_customers')) {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: [makeStaffCustomerRow({ id: 'c-108', displayName: 'Somchai' })],
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (url === '/api/transactions/backdated') {
          calls.push({
            url,
            method,
            key: headers.get('idempotency-key'),
            body: typeof init?.body === 'string' ? JSON.parse(init.body) : null,
          });
          return new Response(
            JSON.stringify(
              makeTransactionDto({
                id: 'tx-backdated-1',
                status: 'COMPLETED',
                documentRef: 'XC-20261104-000001',
              }),
            ),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response('{}', { status: 200 });
      });

      dispose = render(() => <BackdatedScreen api={api} />, host);

      // Pick customer via customer search
      setInput(host.querySelector<HTMLInputElement>('input[name="customerSearch"]')!, 'Somchai');
      host.querySelector<HTMLButtonElement>('button[type="button"]')!.click();
      await vi.waitFor(() => {
        expect(host.querySelector<HTMLButtonElement>('[data-customer-option="c-108"]')).not.toBeNull();
      });
      host.querySelector<HTMLButtonElement>('[data-customer-option="c-108"]')!.click();

      setInput(host.querySelector<HTMLSelectElement>('select[name="side"]')!, 'BUY');
      setInput(host.querySelector<HTMLInputElement>('input[name="currency"]')!, 'USD');
      setInput(host.querySelector<HTMLInputElement>('input[name="quantity"]')!, '500');
      setInput(host.querySelector<HTMLInputElement>('input[name="rate"]')!, '32.15');
      setInput(host.querySelector<HTMLInputElement>('input[name="expectedSettleThb"]')!, '16075.00');
      setInput(host.querySelector<HTMLInputElement>('input[name="paidAt"]')!, '2026-11-04T13:23');
      setInput(host.querySelector<HTMLSelectElement>('select[name="method"]')!, 'Cash');
      setInput(host.querySelector<HTMLTextAreaElement>('textarea[name="notes"]')!, 'Telegram order #1');

      host.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();

      await vi.waitFor(() => {
        expect(calls).toHaveLength(1);
      });

      expect(calls[0]!.method).toBe('POST');
      expect(calls[0]!.key).toBeTruthy();
      expect(calls[0]!.body).toEqual({
        customerId: 'c-108',
        side: 'BUY',
        currency: 'USD',
        quantity: '500',
        rate: '32.15',
        feeThb: '0',
        expectedSettleThb: '16075.00',
        payment: {
          method: 'Cash',
          reference: '',
          account: '',
          slip: null,
          bankFeeThb: '0',
          paidAt: '2026-11-04T13:23:00+07:00',
        },
        passport: null,
        notes: 'Telegram order #1',
      });

      await vi.waitFor(() => {
        expect(host.querySelector('[data-testid="document-ref"]')?.textContent).toBe('XC-20261104-000001');
      });
      expect(host.querySelector<HTMLAnchorElement>('[data-testid="receipt-link"]')?.getAttribute('href')).toBe(
        '/desk/transactions/tx-backdated-1',
      );

      // F51 (docs/design/backdated-keying.md §16.1), restated by the ticket author claude/3b81c9d5: the next deal,
      // keyed into the cleared form, goes with a fresh Idempotency-Key. Before F51 a second click on the same filled
      // form did this, which booked the same deal twice.
      setInput(host.querySelector<HTMLInputElement>('input[name="quantity"]')!, '200');
      setInput(host.querySelector<HTMLInputElement>('input[name="rate"]')!, '32.15');
      setInput(host.querySelector<HTMLInputElement>('input[name="paidAt"]')!, '2026-11-04T14:05');
      host.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();
      await vi.waitFor(() => {
        expect(calls).toHaveLength(2);
      });
      expect(calls[1]!.key).toBeTruthy();
      expect(calls[1]!.key).not.toBe(calls[0]!.key);
    });

    it('shows the server message by field on validation error', async () => {
      const api = new ApiClient(async (url) => {
        if (url === '/api/transactions/backdated') {
          return new Response(
            JSON.stringify({
              error: {
                code: 'validationFailed',
                field: 'expectedSettleThb',
                message: 'expected 16075.00',
              },
            }),
            { status: 400, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(JSON.stringify({ version: 1, changed: true }), { status: 200 });
      });

      dispose = render(() => <BackdatedScreen api={api} />, host);
      setInput(host.querySelector<HTMLInputElement>('input[name="customerId"]')!, 'c-108');
      setInput(host.querySelector<HTMLInputElement>('input[name="quantity"]')!, '500');
      setInput(host.querySelector<HTMLInputElement>('input[name="rate"]')!, '32.15');
      setInput(host.querySelector<HTMLInputElement>('input[name="expectedSettleThb"]')!, '16075.01');
      setInput(host.querySelector<HTMLInputElement>('input[name="paidAt"]')!, '2026-11-04T13:23');

      host.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();

      await vi.waitFor(() => {
        expect(host.querySelector('[data-field-error="expectedSettleThb"]')?.textContent).toContain('16075.00');
      });
    });
  });
});
