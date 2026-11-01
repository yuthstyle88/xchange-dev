// FE-T11 #5858 (docs/design/fe-triage-1101.md §3 W8, amending desk-transaction-detail.md D6): the B04 screen says
// "กลับรายการ" for a reversal, never "คืนรายการ" or the English word, in Thai. The request button reads "ขอกลับรายการ".
// Harness trimmed from TransactionScreen.test.tsx. Written before the implementation by the ticket author
// claude/01TxaRJG. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import {
  makeMe,
  makeReversalSummaryDto,
  makeTransactionAuditViewData,
  makeTransactionDetailDto,
  makeTransactionViewData,
  makeTxPaymentDto,
  makeTxReceiptDto,
  type ReversalSummaryDto,
  type TransactionDetailDto,
} from '../../shared/testing/fixtures';
import { en, th } from './i18n';
import { TransactionScreen } from './TransactionScreen';

const TX_ID = '7f0c2b1e-4a5d-4c3b-9e8f-0a1b2c3d4e5f';
const PATH = `/desk/transactions/${TX_ID}`;

const ME = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'คุณทดสอบ',
  branch: 'MAIN',
  branchName: 'สาขาทดสอบ',
  branches: [],
  permissions: ['TransactionRead', 'ReceiptReprint', 'ReversalRequest'],
});

function completed(over: Partial<TransactionDetailDto> = {}) {
  return makeTransactionDetailDto({
    id: TX_ID,
    status: 'COMPLETED',
    documentRef: 'XC-0001',
    branchId: 'MAIN',
    lines: [],
    payment: makeTxPaymentDto({ method: 'CASH', amountThb: '35120.00', bankFeeThb: '0.00', paidAt: '2026-10-11T03:00:00Z' }),
    receipts: [makeTxReceiptDto({ number: 'XC-0001', revision: 1, issuedAt: '2026-10-11T03:00:05Z' })],
    approvalRequest: null,
    completedAt: '2026-10-11T03:00:05Z',
    cancelledAt: null,
    cancelReason: null,
    reversal: null,
    ...over,
  });
}

function reversal(over: Partial<ReversalSummaryDto> = {}) {
  return makeReversalSummaryDto({
    id: 'rv-0001',
    originalTransactionId: TX_ID,
    status: 'REQUESTED',
    reason: 'ลูกค้าขอยกเลิก',
    amount: '35120.00',
    requestedBy: 'u-teller',
    requestedAt: '2026-10-11T04:00:00Z',
    approvedAt: null,
    postedAt: null,
    documentNumber: null,
    ...over,
  });
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let transaction: unknown;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
}

function installFetch() {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      const view = (data: unknown) => json(200, { version: 1, changed: true, data });
      if (method === 'GET' && u.pathname === '/api/v3/views/me') return view(ME);
      if (method === 'GET' && u.pathname === '/api/v3/views/transaction') return view(transaction);
      if (method === 'GET' && u.pathname === '/api/v3/views/transaction_audit') {
        return view(makeTransactionAuditViewData({ events: [] }));
      }
      if (method === 'POST' && u.pathname === `/api/transactions/${TX_ID}/reversal-requests`) {
        return json(200, { id: 'rv-0001', status: 'REQUESTED' });
      }
      return json(404, { error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` } });
    }),
  );
}

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(tx: TransactionDetailDto) {
  transaction = makeTransactionViewData({ transaction: tx });
  installFetch();
  dispose = render(() => <TransactionScreen pathname={PATH} search="" />, host);
  await settle();
}

const requestBtn = () => host.querySelector<HTMLButtonElement>('button[data-action="request-reversal"]');
const text = () => host.textContent ?? '';
const LATIN = /[A-Za-z]/;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('FE-T11 #5858 W8 B04 reversal wording (th)', () => {
  it('R1 the request button reads "ขอกลับรายการ"', async () => {
    await mount(completed());
    expect(requestBtn()?.textContent?.trim()).toBe('ขอกลับรายการ');
  });

  it('R2 the reason dialog is titled "เหตุผลในการขอกลับรายการ", and success says "ส่งคำขอกลับรายการแล้ว"', async () => {
    await mount(completed());
    requestBtn()!.click();
    await settle();
    const dialog = host.querySelector('dialog');
    expect(dialog, 'S-3 ReasonDialog is open').not.toBeNull();
    expect(dialog!.textContent).toContain('เหตุผลในการขอกลับรายการ');
    const input = dialog!.querySelector<HTMLInputElement>('input')!;
    input.value = 'ลูกค้าขอยกเลิก';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    const buttons = dialog!.querySelectorAll('button');
    (buttons[buttons.length - 1] as HTMLButtonElement).click();
    await settle();
    expect(host.querySelector('.xc-dtx-message[role="status"]')?.textContent?.trim()).toBe('ส่งคำขอกลับรายการแล้ว');
  });

  it('R3 the reversal section is headed "การกลับรายการ"', async () => {
    await mount(completed({ reversal: reversal() }));
    expect(text()).toContain('การกลับรายการ');
    expect(text()).not.toContain('คืนรายการ');
  });

  it('R4 no Thai string of desk-transaction/i18n.ts holds a Latin letter or "คืนรายการ"', () => {
    for (const [k, v] of Object.entries(th)) {
      expect(v, `th.${k}`).not.toMatch(LATIN);
      expect(v, `th.${k}`).not.toContain('คืนรายการ');
    }
  });

  it('R5 en keeps its reversal wording', () => {
    expect(en.requestReversal).toBe('Request a reversal');
    expect(en.reversal).toBe('Reversal');
    expect(en.reasonTitle).toBe('Reason for the reversal');
    expect(en.requested).toBe('Reversal requested');
  });
});
