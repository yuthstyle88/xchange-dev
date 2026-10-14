// WE-26 (#1775): screens/admin/transactions/ — show failed calls through wordedError
// Tests for §4.2 (three tests per changed catch: loadFirstPage, handleLoadMore, handleExportCsv)
// and §4.3 (stale data cleared on branch switch before failed read).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { sharedSession } from '../../../shared/session/session';
import { meOf } from '../../../test-support/me';
import { Dialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { Transactions } from './Transactions';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';

interface RefusalDef {
  status: number;
  code: string;
  message: string;
}

const EXPORTER_ME = (() => {
  const m = meOf({ id: 'owner', role: 'OWNER', branch: 'B1', name: 'Owner' }) as unknown as {
    permissions: string[];
  };
  return { ...m, permissions: [...m.permissions, 'ReportsExport'].sort() };
})();

function makeTx(id: string): TransactionRow {
  return {
    id,
    date: '2026-10-15',
    created: '2026-10-15T03:00:00.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '32.1000',
    unitSize: '1',
    principal: '3210.00',
    fee: '0.00',
    bankFee: '0.00',
    settlement: '3209.99',
    method: 'Cash',
    customer: 'Customer 1',
    customerId: 'CUS-1',
    staff: 'front',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'SH-1',
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: null,
    slipUrl: null,
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('Transactions worded error handling (#1775)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;

  let firstPageFailure: 'network' | RefusalDef | null = null;
  let loadMoreFailure: 'network' | RefusalDef | null = null;
  let exportFailure: 'network' | RefusalDef | null = null;
  let firstPageRowsCount = 1;

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);

    firstPageFailure = null;
    loadMoreFailure = null;
    exportFailure = null;
    firstPageRowsCount = 1;

    sharedSession().updateMe(EXPORTER_ME as any);

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;

        if (PAGE_WIDE.some((p) => url.includes(p))) {
          return jsonResponse({ version: 1, changed: false });
        }

        if (url.includes('/api/v3/views/me')) {
          return jsonResponse({ version: 1, changed: true, data: EXPORTER_ME });
        }

        if (url.includes('/api/v3/views/admin_branches')) {
          return jsonResponse({
            version: 1,
            changed: true,
            data: { branches: [{ id: 'B1', name: 'Silom #2' }] },
          });
        }

        if (url.includes('/api/v3/views/admin_transactions')) {
          const u = new URL(url, 'http://localhost');
          const isExport = u.searchParams.get('purpose') === 'export';
          const isLoadMore = u.searchParams.has('before');

          if (isExport) {
            if (exportFailure === 'network') {
              return Promise.reject(new TypeError('Failed to fetch'));
            }
            if (exportFailure) {
              return jsonResponse(
                { error: { code: exportFailure.code, message: exportFailure.message } },
                exportFailure.status,
              );
            }
            return jsonResponse([makeTx('TX-EXP-1')]);
          }

          if (isLoadMore) {
            if (loadMoreFailure === 'network') {
              return Promise.reject(new TypeError('Failed to fetch'));
            }
            if (loadMoreFailure) {
              return jsonResponse(
                { error: { code: loadMoreFailure.code, message: loadMoreFailure.message } },
                loadMoreFailure.status,
              );
            }
            return jsonResponse([makeTx('TX-MORE-1')]);
          }

          // Initial / first page read
          if (firstPageFailure === 'network') {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
          if (firstPageFailure) {
            return jsonResponse(
              { error: { code: firstPageFailure.code, message: firstPageFailure.message } },
              firstPageFailure.status,
            );
          }

          const rows = Array.from({ length: firstPageRowsCount }, (_, i) => makeTx(`TX-${i + 1}`));
          return jsonResponse({
            version: 1,
            changed: true,
            data: rows,
          });
        }

        return jsonResponse({ error: { code: 'notFound', message: 'not found' } }, 404);
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    container.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(initialBranch = 'B1') {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-15' });
    const [shown] = createSignal(true);

    dispose = render(
      () => (
        <>
          <Transactions branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ),
      container,
    );

    return { branch, setBranch };
  }

  function getErrorText(): string | null {
    const el = container.querySelector('[role="alert"]');
    return el ? (el.textContent ?? '').trim() : null;
  }

  function getExportButton(): HTMLButtonElement {
    const buttons = [...container.querySelectorAll('button')];
    const btn = buttons.find(
      (b) => b.textContent?.includes('ส่งออก') || b.textContent?.includes('Export CSV'),
    );
    expect(btn, 'export button should exist').toBeDefined();
    return btn!;
  }

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 1: loadFirstPage (initial view read)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 1: loadFirstPage view failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      firstPageFailure = 'network';
      mount();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(th['error.network']);
      });
      expect(getErrorText()).not.toContain('Failed to fetch');

      // In EN
      dispose?.();
      setLang('en');
      mount();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(en['error.network']);
      });
      expect(getErrorText()).not.toContain('Failed to fetch');
    });

    it('(2) 500 server error shows error.serverError in TH', async () => {
      firstPageFailure = {
        status: 500,
        code: 'roleNotAllowedHere',
        message: 'RAW-server-500-dead',
      };
      mount();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(th['error.serverError']);
      });
      expect(getErrorText()).not.toContain('RAW-server-500-dead');
      expect(getErrorText()).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      firstPageFailure = {
        status: 400,
        code: 'customTransactionsRefusal',
        message: 'ข้อความปฏิเสธเฉพาะของธุรกรรม',
      };
      mount();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe('ข้อความปฏิเสธเฉพาะของธุรกรรม');
      });
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 2: handleLoadMore (page read)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 2: handleLoadMore page failures', () => {
    it('(1) network TypeError on loadMore shows error.network in TH and EN', async () => {
      firstPageRowsCount = 100;
      mount();

      await vi.waitFor(() => {
        expect(container.querySelector('.load-more-wrapper button')).not.toBeNull();
      });

      loadMoreFailure = 'network';
      const loadMoreBtn = container.querySelector<HTMLButtonElement>('.load-more-wrapper button')!;
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(th['error.network']);
      });
      expect(getErrorText()).not.toContain('Failed to fetch');

      // In EN
      setLang('en');
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(en['error.network']);
      });
      expect(getErrorText()).not.toContain('Failed to fetch');
    });

    it('(2) 500 server error on loadMore shows error.serverError in TH', async () => {
      firstPageRowsCount = 100;
      mount();

      await vi.waitFor(() => {
        expect(container.querySelector('.load-more-wrapper button')).not.toBeNull();
      });

      loadMoreFailure = {
        status: 500,
        code: 'roleNotAllowedHere',
        message: 'RAW-more-500-crash',
      };
      const loadMoreBtn = container.querySelector<HTMLButtonElement>('.load-more-wrapper button')!;
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(th['error.serverError']);
      });
      expect(getErrorText()).not.toContain('RAW-more-500-crash');
      expect(getErrorText()).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal on loadMore with unshared code and server message shows that message', async () => {
      firstPageRowsCount = 100;
      mount();

      await vi.waitFor(() => {
        expect(container.querySelector('.load-more-wrapper button')).not.toBeNull();
      });

      loadMoreFailure = {
        status: 400,
        code: 'customLoadMoreRefusal',
        message: 'โหลดข้อมูลต่อไม่ได้เนื่องจากติดเงื่อนไข',
      };
      const loadMoreBtn = container.querySelector<HTMLButtonElement>('.load-more-wrapper button')!;
      loadMoreBtn.click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe('โหลดข้อมูลต่อไม่ได้เนื่องจากติดเงื่อนไข');
      });
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 3: handleExportCsv (walkPages / export call)
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 3: handleExportCsv export failures', () => {
    it('(1) network TypeError on export shows error.network in TH and EN', async () => {
      mount();

      await vi.waitFor(() => {
        expect(container.querySelector('tbody tr')).not.toBeNull();
      });

      exportFailure = 'network';
      getExportButton().click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(th['error.network']);
      });
      expect(getErrorText()).not.toContain('Failed to fetch');

      // In EN
      setLang('en');
      getExportButton().click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(en['error.network']);
      });
      expect(getErrorText()).not.toContain('Failed to fetch');
    });

    it('(2) 500 server error on export shows error.serverError in TH', async () => {
      mount();

      await vi.waitFor(() => {
        expect(container.querySelector('tbody tr')).not.toBeNull();
      });

      exportFailure = {
        status: 500,
        code: 'roleNotAllowedHere',
        message: 'RAW-export-500-abort',
      };
      getExportButton().click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(th['error.serverError']);
      });
      expect(getErrorText()).not.toContain('RAW-export-500-abort');
      expect(getErrorText()).not.toContain(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal on export with unshared code and server message shows that message', async () => {
      mount();

      await vi.waitFor(() => {
        expect(container.querySelector('tbody tr')).not.toBeNull();
      });

      exportFailure = {
        status: 400,
        code: 'customExportRefusal',
        message: 'ไม่อนุญาตให้ดาวน์โหลดรายงานนี้',
      };
      getExportButton().click();

      await vi.waitFor(() => {
        expect(getErrorText()).toBe('ไม่อนุญาตให้ดาวน์โหลดรายงานนี้');
      });
      expect(getErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // §4.3: Branch switch stale data handling
  // ──────────────────────────────────────────────────────────────────────────
  describe('§4.3: Branch switch stale data handling', () => {
    it('after branch switch, a failed read does not leave the previous branch data showing', async () => {
      const { setBranch } = mount('B1');

      await vi.waitFor(() => {
        expect(container.querySelector('tbody tr')).not.toBeNull();
      });
      expect(container.querySelectorAll('tbody tr').length).toBe(1);

      // Now set branch to B2 and make the read fail
      firstPageFailure = {
        status: 500,
        code: 'serverError',
        message: 'Failed to read B2',
      };
      setBranch('B2');

      await vi.waitFor(() => {
        expect(getErrorText()).toBe(th['error.serverError']);
      });
      // Previous branch's rows must be absent
      expect(container.querySelectorAll('tbody tr').length).toBe(0);
    });
  });
});
