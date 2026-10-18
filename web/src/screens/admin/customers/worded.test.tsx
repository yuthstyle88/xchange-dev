import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { closeDialog, Dialog } from '../parts/Dialog';
import { Customers } from './Customers';

interface CustomerRow {
  id: string;
  name: string;
  nationality: string;
  created: string;
  member: boolean;
  completed: number;
  volume: string;
  lastAt: string | null;
}

function customer(n: number, over: Partial<CustomerRow> = {}): CustomerRow {
  return {
    id: `CX-${String(n).padStart(8, '0')}`,
    name: `Customer ${n}`,
    nationality: 'THA',
    created: '2026-10-01T02:03:04.000Z',
    member: false,
    completed: 2,
    volume: '6420.00',
    lastAt: '2026-10-08T03:04:05.000Z',
    ...over,
  };
}

function tx(n: number) {
  return {
    id: `XC-20261008-${String(n).padStart(6, '0')}`,
    date: '2026-10-08',
    created: '2026-10-08T03:04:05.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '32.1000',
    unitSize: '1',
    principal: '3210.00',
    fee: '0.00',
    bankFee: '0.00',
    settlement: '3210.00',
    method: 'Cash',
    customer: 'Customer 1',
    customerId: 'CX-00000001',
    staff: 'front1',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
  };
}

describe('WE-13: screens/admin/customers/worded.test.tsx', () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;

  let customersHandler: (params: URLSearchParams) => Promise<Response>;
  let historyHandler: (params: URLSearchParams) => Promise<Response>;

  function json(status: number, body: unknown): Response {
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });
  }

  beforeEach(() => {
    setLang('th');

    customersHandler = async () =>
      json(200, { version: 3, changed: true, data: [customer(1)] });
    historyHandler = async () =>
      json(200, { version: 3, changed: true, data: [tx(1)] });

    vi.stubGlobal(
      'fetch',
      vi.fn((input: RequestInfo | URL) => {
        const url = new URL(String(input), 'http://desk.test');
        const view = url.pathname.startsWith('/api/v3/views/')
          ? url.pathname.replace(/^\/api\/v3\/views\//, '')
          : null;
        if (view === 'admin_customers') {
          return customersHandler(url.searchParams);
        }
        if (view === 'admin_customer_history') {
          return historyHandler(url.searchParams);
        }
        if (view === 'admin_branches') {
          return Promise.resolve(
            json(200, {
              version: 3,
              changed: true,
              data: {
                branches: [
                  { id: 'MAIN', name: 'Main Branch' },
                  { id: 'B02', name: 'Branch 2' },
                ],
              },
            }),
          );
        }
        return Promise.resolve(json(200, { version: 3, changed: false }));
      }),
    );
  });

  afterEach(() => {
    closeDialog();
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 6; i++) {
      for (let j = 0; j < 5; j++) await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function until(ok: () => boolean, what: string): Promise<void> {
    const start = Date.now();
    while (!ok()) {
      if (Date.now() - start > 2000) throw new Error(`timed out waiting for: ${what}`);
      await settle();
    }
    await settle();
  }

  function mount(initialBranch = 'MAIN') {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period, setPeriod] = createSignal({ from: '2026-10-01', to: '2026-10-08' });
    const [shown, setShown] = createSignal(true);

    root = document.createElement('div');
    document.body.appendChild(root);

    dispose = render(
      (): JSX.Element => (
        <>
          <Customers branch={branch} period={period} shown={shown} />
          <Dialog />
        </>
      ),
      root,
    );

    return { setBranch, setPeriod, setShown };
  }

  async function openCustomerHistory(): Promise<void> {
    await until(() => root.querySelectorAll('table tbody tr').length > 0, 'customers loaded');
    const tr = root.querySelector('table tbody tr')!;
    const btn = [...tr.querySelectorAll('button')].find(
      (b) => (b.textContent ?? '').includes('ดูประวัติ') || (b.textContent ?? '').includes('View History'),
    )!;
    btn.click();
    await settle();
  }

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 1: readCustomers (admin_customers view call)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 1: readCustomers failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customersHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();
      await until(() => root.querySelector('.refusal-message') !== null, 'refusal message');

      const msgTh = root.querySelector('.refusal-message')?.textContent;
      expect(msgTh).toBe(th['error.network']);
      expect(msgTh).not.toContain('TypeError');
      expect(msgTh).not.toContain('Failed to fetch');

      // In EN
      dispose?.();
      setLang('en');
      mount();
      await until(() => root.querySelector('.refusal-message') !== null, 'refusal message in en');

      const msgEn = root.querySelector('.refusal-message')?.textContent;
      expect(msgEn).toBe(en['error.network']);
      expect(msgEn).not.toContain('TypeError');
      expect(msgEn).not.toContain('Failed to fetch');
    });

    it('(2) 500 server error shows error.serverError in TH', async () => {
      customersHandler = () =>
        Promise.resolve(
          json(500, {
            error: { code: 'roleNotAllowedHere', message: 'Internal postgres error' },
          }),
        );
      mount();
      await until(() => root.querySelector('.refusal-message') !== null, 'refusal message');

      const msg = root.querySelector('.refusal-message')?.textContent;
      expect(msg).toBe(th['error.serverError']);
      expect(msg).not.toContain('Internal postgres error');
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      customersHandler = () =>
        Promise.resolve(
          json(400, {
            error: {
              code: 'customCustomersRefusal',
              message: 'ไม่อนุญาตให้เข้าถึงรายชื่อลูกค้านี้',
            },
          }),
        );
      mount();
      await until(() => root.querySelector('.refusal-message') !== null, 'refusal message');

      const msg = root.querySelector('.refusal-message')?.textContent;
      expect(msg).toBe('ไม่อนุญาตให้เข้าถึงรายชื่อลูกค้านี้');
      expect(msg).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 2: loadHistory (admin_customer_history view call)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 2: loadHistory failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      historyHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();
      await openCustomerHistory();
      await until(() => root.querySelector('dialog .banner.error') !== null, 'history error banner');

      const msgTh = root.querySelector('dialog .banner.error')?.textContent;
      expect(msgTh).toBe(th['error.network']);
      expect(msgTh).not.toContain('TypeError');

      // In EN
      dispose?.();
      setLang('en');
      mount();
      await openCustomerHistory();
      await until(() => root.querySelector('dialog .banner.error') !== null, 'history error banner in en');

      const msgEn = root.querySelector('dialog .banner.error')?.textContent;
      expect(msgEn).toBe(en['error.network']);
      expect(msgEn).not.toContain('TypeError');
    });

    it('(2) 500 server error shows error.serverError in TH', async () => {
      historyHandler = () =>
        Promise.resolve(
          json(500, {
            error: { code: 'roleNotAllowedHere', message: 'History partition corrupt' },
          }),
        );
      mount();
      await openCustomerHistory();
      await until(() => root.querySelector('dialog .banner.error') !== null, 'history error banner');

      const msg = root.querySelector('dialog .banner.error')?.textContent;
      expect(msg).toBe(th['error.serverError']);
      expect(msg).not.toContain('History partition corrupt');
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      historyHandler = () =>
        Promise.resolve(
          json(400, {
            error: {
              code: 'customHistoryRefusal',
              message: 'ไม่สามารถดูประวัติลูกค้ารายนี้ได้ในขณะนี้',
            },
          }),
        );
      mount();
      await openCustomerHistory();
      await until(() => root.querySelector('dialog .banner.error') !== null, 'history error banner');

      const msg = root.querySelector('dialog .banner.error')?.textContent;
      expect(msg).toBe('ไม่สามารถดูประวัติลูกค้ารายนี้ได้ในขณะนี้');
      expect(msg).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // §4.3: Branch switch stale data isolation
  // ──────────────────────────────────────────────────────────────────────────
  describe('§4.3: Branch switch stale data handling', () => {
    it('after branch switch, a failed read does not leave the previous branch data showing', async () => {
      const { setBranch } = mount('MAIN');
      await until(() => root.querySelectorAll('table tbody tr').length === 1, '1 customer row');

      // Previous branch MAIN has 1 row showing
      expect(root.querySelectorAll('table tbody tr').length).toBe(1);

      // Now switch branch to B02, where read fails
      customersHandler = () =>
        Promise.resolve(
          json(500, {
            error: { code: 'serverError', message: 'Failed to read B02' },
          }),
        );
      setBranch('B02');
      await until(() => root.querySelector('.refusal-message') !== null, 'refusal on B02');

      // The refusal message must be shown
      expect(root.querySelector('.refusal-message')?.textContent).toBe(th['error.serverError']);

      // Stale data from previous branch must NOT be shown
      expect(root.querySelectorAll('table tbody tr').length).toBe(0);
      expect(root.querySelector('.summary-status')).toBeNull();
    });
  });
});
