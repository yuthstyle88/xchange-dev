import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { Transactions } from './Transactions';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';

function tx(n: number): TransactionRow {
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
    staff: 'alice',
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

describe('Admin Transactions export in-flight guard (#2894)', () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;

  let branchesHandler: () => Promise<Response>;
  let transactionsViewHandler: (params: URLSearchParams) => Promise<Response>;
  let transactionsExportHandler: (params: URLSearchParams) => Promise<Response>;
  let transactionsExportCalls: URLSearchParams[] = [];

  let blobs: Blob[] = [];
  let names: string[] = [];
  const originalCreate = URL.createObjectURL;
  const originalRevoke = URL.revokeObjectURL;

  function json(status: number, body: unknown): Response {
    return new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });
  }

  beforeEach(() => {
    setLang('th');
    sharedSession().updateMe?.({ id: 'owner', permissions: ['ReportsExport'] } as any);

    blobs = [];
    names = [];
    transactionsExportCalls = [];

    URL.createObjectURL = ((b: Blob) => {
      blobs.push(b);
      return `blob:test/${blobs.length}`;
    }) as typeof URL.createObjectURL;
    URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL;
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      names.push(this.download);
    });

    branchesHandler = async () =>
      json(200, {
        version: 1,
        changed: true,
        data: {
          branches: [
            { id: 'B1', name: 'Silom #2' },
          ],
        },
      });

    transactionsViewHandler = async () =>
      json(200, {
        version: 1,
        changed: true,
        data: [tx(1)],
      });

    transactionsExportHandler = async () =>
      json(200, [tx(1)]);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: RequestInfo | URL) => {
        const url = new URL(String(input), 'http://desk.test');
        const view = url.pathname.startsWith('/api/v3/views/')
          ? url.pathname.replace(/^\/api\/v3\/views\//, '')
          : null;
        if (view === 'admin_branches') {
          return branchesHandler();
        }
        if (view === 'admin_transactions') {
          if (url.searchParams.get('purpose') === 'export') {
            transactionsExportCalls.push(url.searchParams);
            return transactionsExportHandler(url.searchParams);
          }
          return transactionsViewHandler(url.searchParams);
        }
        return Promise.resolve(json(200, { version: 1, changed: false }));
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    closeDialog();
    toast('');
    URL.createObjectURL = originalCreate;
    URL.revokeObjectURL = originalRevoke;
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

  function mount(initialBranch = 'B1') {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period, setPeriod] = createSignal({ from: '2026-10-01', to: '2026-10-08' });
    const [shown, setShown] = createSignal(true);

    root = document.createElement('div');
    document.body.appendChild(root);

    dispose = render(
      (): JSX.Element => (
        <>
          <Transactions branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );

    return { setBranch, setPeriod, setShown };
  }

  function alertText(): string | null {
    const el = root.querySelector('[role="alert"]');
    return el ? (el.textContent ?? '').trim() : null;
  }

  function exportButton(): HTMLButtonElement {
    const buttons = [...root.querySelectorAll<HTMLButtonElement>('button')];
    const btn = buttons.find(
      (b) => b.textContent?.includes('ส่งออก') || b.textContent?.includes('Export CSV'),
    );
    expect(btn, 'the export button').toBeDefined();
    return btn!;
  }

  it('1: click Export CSV twice before the first export ends: exactly one walk starts and downloadCsv is called once', async () => {
    mount();
    await until(() => root.querySelector('tbody tr') !== null, 'table rows loaded');
    const btn = exportButton();

    let finishExport!: (res: Response) => void;
    const exportPromise = new Promise<Response>((resolve) => {
      finishExport = resolve;
    });
    transactionsExportHandler = () => exportPromise;

    // First click
    btn.click();
    await until(() => transactionsExportCalls.length === 1, 'first export walk started');

    // Second click while first export is in flight
    btn.click();
    await settle();

    // Still only one walk started
    expect(transactionsExportCalls.length).toBe(1);

    // Complete the first export
    finishExport(json(200, [tx(1)]));
    await until(() => names.length === 1, 'download completed');

    expect(transactionsExportCalls.length).toBe(1);
    expect(names.length).toBe(1);
  });

  it('2: an export is in flight: Export CSV button is disabled, enabled again after export succeeds or fails', async () => {
    mount();
    await until(() => root.querySelector('tbody tr') !== null, 'table rows loaded');
    const btn = exportButton();
    expect(btn.disabled).toBe(false);

    let finishExport!: (res: Response) => void;
    let exportPromise = new Promise<Response>((resolve) => {
      finishExport = resolve;
    });
    transactionsExportHandler = () => exportPromise;

    // Start export
    btn.click();
    await until(() => transactionsExportCalls.length === 1, 'export started');
    expect(btn.disabled).toBe(true);

    // Resolve export successfully
    finishExport(json(200, [tx(1)]));
    await until(() => names.length === 1, 'download completed');
    expect(btn.disabled).toBe(false);

    // Start second export, which fails
    exportPromise = new Promise<Response>((resolve) => {
      finishExport = resolve;
    });
    transactionsExportHandler = () => exportPromise;

    btn.click();
    await until(() => transactionsExportCalls.length === 2, 'second export started');
    expect(btn.disabled).toBe(true);

    finishExport(json(500, { error: { code: 'internal', message: 'server error' } }));
    await until(() => alertText() !== null, 'error banner displayed');
    expect(btn.disabled).toBe(false);
  });

  it('3: an export fails (banner shown), then a retry succeeds: banner is gone after the retry', async () => {
    mount();
    await until(() => root.querySelector('tbody tr') !== null, 'table rows loaded');
    const btn = exportButton();

    // First export fails
    transactionsExportHandler = async () =>
      json(500, { error: { code: 'serverError', message: 'Fail' } });

    btn.click();
    await until(() => alertText() !== null, 'error banner shown');
    expect(alertText()).not.toBeNull();

    // Retry export succeeds
    transactionsExportHandler = async () => json(200, [tx(1)]);
    btn.click();
    await until(() => names.length === 1, 'retry download completed');

    expect(alertText()).toBeNull();
  });

  it('4 (kept): an export fails: banner shows the worded error', async () => {
    mount();
    await until(() => root.querySelector('tbody tr') !== null, 'table rows loaded');
    const btn = exportButton();

    transactionsExportHandler = async () =>
      json(500, { error: { code: 'serverError', message: 'Fail' } });

    btn.click();
    await until(() => alertText() !== null, 'error banner shown');

    expect(alertText()).toBe('เซิร์ฟเวอร์ขัดข้อง');
  });

  it('5 (kept): an export finds no rows: noTransactionsToExport toast is shown and no file is downloaded', async () => {
    mount();
    await until(() => root.querySelector('tbody tr') !== null, 'table rows loaded');
    const btn = exportButton();

    transactionsExportHandler = async () => json(200, []);

    btn.click();
    await until(() => {
      const t = root.querySelector('.toast');
      return t !== null && !t.hasAttribute('hidden') && (t.textContent ?? '').length > 0;
    }, 'toast shown');

    const toastEl = root.querySelector('.toast');
    expect(toastEl?.textContent).toBe('ไม่มีรายการให้ส่งออก');
    expect(names.length).toBe(0);
  });
});
