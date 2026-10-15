import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { en as sharedEn } from '../../shared/i18n/en';
import { th as sharedTh } from '../../shared/i18n/th';
import type { StaffMeView } from '../../shared/api/generated/staff_me';
import type { TransactionDetailDto } from '../../shared/api/generated/transaction';
import type { TransactionAuditViewData } from '../../shared/api/generated/transaction_audit';
import { TransactionScreen } from './TransactionScreen';

const TX_ID = '00000000-0000-4000-8000-000000000002';
const PATH = `/desk/transactions/${TX_ID}`;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function sampleStaffMe(): StaffMeView {
  return {
    userId: '00000000-0000-4000-8000-000000000001',
    name: 'คุณทดสอบ',
    roles: [],
    branchId: 'MAIN',
    branchName: 'สาขาทดสอบ',
    permissions: ['TransactionRead', 'ReceiptReprint', 'ReversalRequest'],
  };
}

function sampleTx(over: Partial<TransactionDetailDto> = {}): TransactionDetailDto {
  return {
    id: TX_ID,
    status: 'COMPLETED',
    documentRef: 'XC-0001',
    branchId: 'MAIN',
    customerId: 'cust-1',
    quoteId: null,
    quoteExpiresAt: null,
    lines: [
      {
        lineNo: 1,
        currency: 'USD',
        side: 'BUY',
        quantity: '1000.0000',
        unitSize: '1.0000',
        rateSnapshot: '35.12000000',
        thbAmount: '35120.00',
        feeThb: '0.00',
        settleThb: '35120.00',
      },
    ],
    payment: {
      method: 'CASH',
      reference: '',
      account: '',
      slipFileKey: null,
      amountThb: '35120.00',
      bankFeeThb: '0.00',
      paidAt: '2026-10-11T03:00:00Z',
    },
    receipts: [{ number: 'XC-0001', revision: 1, issuedAt: '2026-10-11T03:00:05Z' }],
    approvalRequest: null,
    completedAt: '2026-10-11T03:00:05Z',
    cancelledAt: null,
    cancelReason: null,
    reversal: null,
    ...over,
  };
}

const sampleAudit: TransactionAuditViewData = {
  events: [
    { at: '2026-10-11T02:59:30Z', actorId: 'u-mgr', actorRole: 'ROLE_A', action: 'approval.approved' },
  ],
};

describe('WE-31 screens/desk-transaction/ worded error (#1780)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let meHandler: () => Promise<Response>;
  let txHandler: () => Promise<Response>;
  let auditHandler: () => Promise<Response>;
  let receiptHandler: () => Promise<Response>;
  let reversalHandler: () => Promise<Response>;

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);

    meHandler = async () => json(200, { version: 1, changed: true, data: sampleStaffMe() });
    txHandler = async () => json(200, { version: 1, changed: true, data: { transaction: sampleTx() } });
    auditHandler = async () => json(200, { version: 1, changed: true, data: sampleAudit });
    receiptHandler = async () =>
      json(200, {
        version: 1,
        changed: true,
        data: { receipt: { number: 'XC-0001', revision: 1, issuedAt: '2026-10-11T03:00:05Z', html: '<p>ok</p>' } },
      });
    reversalHandler = async () => json(200, { id: 'rv-0001', status: 'REQUESTED' });

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(url, 'http://xc.test');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') return meHandler();
        if (method === 'GET' && u.pathname === '/api/v3/views/transaction') return txHandler();
        if (method === 'GET' && u.pathname === '/api/v3/views/transaction_audit') return auditHandler();
        if (method === 'GET' && u.pathname === '/api/v3/views/receipt_document') return receiptHandler();
        if (method === 'POST' && u.pathname.includes('/reversal-requests')) return reversalHandler();

        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(initialSearch = '') {
    const [search, setSearch] = createSignal(initialSearch);
    dispose = render(() => <TransactionScreen pathname={PATH} search={search()} />, root);
    return { search, setSearch };
  }

  function messageText(): string {
    const el = root.querySelector('.xc-dtx-message');
    return (el?.textContent ?? '').trim();
  }

  async function requestReversal(reason = 'ลูกค้าขอยกเลิก') {
    const btn = root.querySelector('button[data-action="request-reversal"]') as HTMLButtonElement;
    expect(btn, 'request-reversal button exists').not.toBeNull();
    btn.click();
    await vi.waitFor(() => {
      expect(root.querySelector('dialog')).not.toBeNull();
    });
    const input = root.querySelector('dialog input') as HTMLInputElement;
    input.value = reason;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    const buttons = root.querySelectorAll('dialog button');
    const confirmBtn = buttons[buttons.length - 1] as HTMLButtonElement;
    confirmBtn.click();
  }

  // ── Catch 1: load (transaction and transaction_audit) ──────────────────────

  describe('Catch 1: load (transaction read)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      setLang('th');
      txHandler = async () => {
        throw new TypeError('Failed to fetch');
      };
      mount();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.network']);
      });

      setLang('en');
      dispose?.();
      dispose = undefined;
      root.innerHTML = '';
      mount();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows th["error.serverError"]', async () => {
      setLang('th');
      txHandler = async () =>
        json(500, { error: { code: 'roleNotAllowedHere', message: 'internal server error' } });
      mount();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      setLang('th');
      const serverMessage = 'เซิร์ฟเวอร์ปฏิเสธการเข้าถึงรายการธุรกรรมนี้เป็นการเฉพาะ';
      txHandler = async () =>
        json(400, { error: { code: 'unmappedTxLoadError999', message: serverMessage } });
      mount();

      await vi.waitFor(() => {
        expect(messageText()).toBe(serverMessage);
      });
    });
  });

  // ── Catch 2: reprint (receipt_document) ───────────────────────────────────

  describe('Catch 2: reprint (receipt_document)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      setLang('th');
      mount();

      await vi.waitFor(() => {
        expect(root.querySelector('button[data-action="reprint"]')).not.toBeNull();
      });

      receiptHandler = async () => {
        throw new TypeError('Failed to fetch');
      };

      const reprintBtn = root.querySelector('button[data-action="reprint"]') as HTMLButtonElement;
      reprintBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.network']);
      });

      setLang('en');
      reprintBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows th["error.serverError"]', async () => {
      setLang('th');
      mount();

      await vi.waitFor(() => {
        expect(root.querySelector('button[data-action="reprint"]')).not.toBeNull();
      });

      receiptHandler = async () =>
        json(500, { error: { code: 'roleNotAllowedHere', message: 'internal reprint failure' } });

      const reprintBtn = root.querySelector('button[data-action="reprint"]') as HTMLButtonElement;
      reprintBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      setLang('th');
      const serverMessage = 'เครื่องพิมพ์ไม่สามารถพิมพ์เอกสารนี้ได้ในขณะนี้';
      mount();

      await vi.waitFor(() => {
        expect(root.querySelector('button[data-action="reprint"]')).not.toBeNull();
      });

      receiptHandler = async () =>
        json(400, { error: { code: 'unmappedReprintError777', message: serverMessage } });

      const reprintBtn = root.querySelector('button[data-action="reprint"]') as HTMLButtonElement;
      reprintBtn.click();

      await vi.waitFor(() => {
        expect(messageText()).toBe(serverMessage);
      });
    });
  });

  // ── Catch 3: sendRequest (reversal-requests) ───────────────────────────────

  describe('Catch 3: sendRequest (reversal-requests)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      setLang('th');
      mount();

      await vi.waitFor(() => {
        expect(root.querySelector('button[data-action="request-reversal"]')).not.toBeNull();
      });

      reversalHandler = async () => {
        throw new TypeError('Failed to fetch');
      };

      await requestReversal();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.network']);
      });

      setLang('en');
      await requestReversal();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows th["error.serverError"]', async () => {
      setLang('th');
      mount();

      await vi.waitFor(() => {
        expect(root.querySelector('button[data-action="request-reversal"]')).not.toBeNull();
      });

      reversalHandler = async () =>
        json(500, { error: { code: 'roleNotAllowedHere', message: 'reversal post server error' } });

      await requestReversal();

      await vi.waitFor(() => {
        expect(messageText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      setLang('th');
      const serverMessage = 'ธุรกรรมนี้ไม่สามารถขอยกเลิกได้เนื่องจากเกินกำหนดเวลาเฉพาะ';
      mount();

      await vi.waitFor(() => {
        expect(root.querySelector('button[data-action="request-reversal"]')).not.toBeNull();
      });

      reversalHandler = async () =>
        json(409, { error: { code: 'unmappedReversalError555', message: serverMessage } });

      await requestReversal();

      await vi.waitFor(() => {
        expect(messageText()).toBe(serverMessage);
      });
    });
  });

  // ── Criterion 3: stale data on branch switch ───────────────────────────────

  describe('Criterion 3: stale data on branch switch', () => {
    it('clears stale data from previous branch when subsequent branch read fails', async () => {
      const { setSearch } = mount('?branch=B1');

      // Verify B1 loaded successfully with transaction data
      await vi.waitFor(() => {
        expect(root.textContent).toContain('XC-0001');
        expect(root.textContent).toContain('35120.00');
        expect(root.querySelector('[data-state="ready"]')).not.toBeNull();
      });

      // Next read for B2 fails
      txHandler = async () =>
        json(400, { error: { code: 'branchNotAllowed', message: 'ไม่อนุญาตให้เข้าถึงสาขานี้' } });

      setSearch('?branch=B2');

      await vi.waitFor(() => {
        expect(messageText()).toBe('ไม่อนุญาตให้เข้าถึงสาขานี้');
      });

      // Previous branch's transaction data must not be displayed
      expect(root.textContent).not.toContain('35120.00');
      expect(root.textContent).not.toContain('XC-0001');
      expect(root.querySelector('[data-state="ready"]')).toBeNull();
      expect(root.querySelector('[data-state="error"]')).not.toBeNull();
    });
  });
});
