// TD-3 #1291: the page itself, mounted through its entry `main.tsx` as `web/pages/desk-transaction.html` loads it.
// Written before the implementation by the up-front test writer (claude/709ec749, on the xchange FE lane controller
// claude/5edf7bb5's order). Do not edit, delete or weaken; add tests in a file of your own.
//
// LK-1b (`docs/design/xchange-v3-look.md`): a page must render the shared shell, not only import it. This test goes one
// step past `shell-look.test.ts`, which reads the source: it runs the entry and checks the screen is drawn inside
// `AppShell`, taking its id from the page's own URL (`/desk/transactions/{id}`).
import { afterEach, expect, it, vi } from 'vitest';
import {
  makeStaffMeView,
  makeTransactionAuditViewData,
  makeTransactionDetailDto,
  makeTransactionViewData,
  makeTxLineDto,
  makeTxReceiptDto,
} from '../../shared/testing/fixtures';

const TX_ID = '3c1d9a7e-2b4f-4e6a-8c0d-1f2e3a4b5c6d';

const ME = makeStaffMeView({
  userId: '00000000-0000-4000-8000-000000000001',
  name: 'คุณทดสอบ',
  roles: [],
  branchId: 'MAIN',
  branchName: 'สาขาทดสอบ',
  permissions: ['TransactionRead', 'ReceiptReprint', 'ReversalRequest'],
});

const TRANSACTION = makeTransactionViewData({
  transaction: makeTransactionDetailDto({
    id: TX_ID,
    status: 'COMPLETED',
    documentRef: 'XC-ENTRY-0042',
    branchId: 'MAIN',
    customerId: null,
    quoteId: null,
    quoteExpiresAt: null,
    lines: [
      makeTxLineDto({
        lineNo: 1,
        currency: 'USD',
        side: 'BUY',
        quantity: '100.0000',
        unitSize: '1.0000',
        rateSnapshot: '32.10000000',
        thbAmount: '3210.00',
        feeThb: '0.00',
        settleThb: '3210.00',
      }),
    ],
    payment: null,
    receipts: [makeTxReceiptDto({ number: 'XC-ENTRY-0042', revision: 1, issuedAt: '2026-10-11T03:00:05Z' })],
    approvalRequest: null,
    completedAt: '2026-10-11T03:00:05Z',
    cancelledAt: null,
    cancelReason: null,
    reversal: null,
  }),
});

const AUDIT = makeTransactionAuditViewData({ events: [] });

function view(data: unknown): Response {
  return new Response(JSON.stringify({ version: 1, changed: true, data }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

afterEach(() => {
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it("main.tsx draws the screen inside AppShell, reading the transaction named by the page's URL", async () => {
  history.replaceState(null, '', `/desk/transactions/${TX_ID}`);
  const host = document.createElement('div');
  host.id = 'app';
  document.body.appendChild(host);
  const asked: URL[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://xc.test');
      asked.push(u);
      if (u.pathname === '/api/v3/views/staff_me') return view(ME);
      if (u.pathname === '/api/v3/views/transaction') return view(TRANSACTION);
      if (u.pathname === '/api/v3/views/transaction_audit') return view(AUDIT);
      return new Response(JSON.stringify({ error: { code: 'notFound', message: u.pathname } }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' },
      });
    }),
  );

  await import('./main');
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));

  const shell = host.querySelector('.xc-shell');
  expect(shell, 'the entry renders the shared AppShell').not.toBeNull();
  const screen = shell!.querySelector('[data-state]');
  expect(screen, 'the screen is drawn inside the shell').not.toBeNull();
  const read = asked.find((u) => u.pathname === '/api/v3/views/transaction');
  expect(read?.searchParams.get('id'), "the id comes from the page's URL").toBe(TX_ID);
  expect(screen!.getAttribute('data-state')).toBe('ready');
  expect(shell!.textContent).toContain('XC-ENTRY-0042');
});
