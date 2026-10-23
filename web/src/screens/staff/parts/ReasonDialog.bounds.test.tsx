import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang, t as sharedT } from '../../../shared/i18n';
import { th as staffTh } from '../i18n';
import { th as holdsTh } from '../holds/i18n';
import {
  makeMe,
  makeStaffCustomerRow,
  makeStaffFloorView,
  makeStaffMeView,
  makeTransactionDetailDto,
  makeTransactionViewData,
  makeReceiptDocumentDto,
  makeReceiptDocumentViewData,
  makeTransactionAuditViewData,
  makeTxPaymentDto,
  makeTxReceiptDto,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { ReasonDialog } from './ReasonDialog';
import { HoldsCard } from '../holds/HoldsCard';
import { CancelReasonDialog } from '../../../shared/ui/CancelReasonDialog';
import { TransactionScreen } from '../../desk-transaction/TransactionScreen';

const TX_ID = '7f0c2b1e-4a5d-4c3b-9e8f-0a1b2c3d4e5f';
const PATH = `/desk/transactions/${TX_ID}`;

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function viewResponse(data: unknown): Response {
  return jsonResponse(200, { version: 1, changed: true, data });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function typeInto(el: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  const max = el.maxLength;
  el.value = max !== undefined && max > 0 && value.length > max ? value.slice(0, max) : value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('ReasonDialog and bounds validation tests (#4903)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let posts: { url: string; body: unknown }[];

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    posts = [];
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function installTransactionFetch() {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(rawUrl, 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && u.pathname === '/api/v3/views/me') {
          return viewResponse(
            makeMe({
              id: '00000000-0000-4000-8000-000000000001',
              name: 'Staff',
              permissions: ['TransactionRead', 'ReversalRequest', 'ReceiptReprint'],
            }),
          );
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/staff_me') {
          return jsonResponse(401, { error: { code: 'employeeAuthRequired', message: 'employee auth required' } });
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/transaction') {
          return viewResponse(
            makeTransactionViewData({
              transaction: makeTransactionDetailDto({
                id: TX_ID,
                status: 'COMPLETED',
                documentRef: 'XC-0001',
                branchId: 'MAIN',
                customerId: 'cust-1',
                quoteId: null,
                quoteExpiresAt: null,
                lines: [],
                payment: makeTxPaymentDto({
                  method: 'CASH',
                  reference: '',
                  account: '',
                  slipFileKey: null,
                  amountThb: '35120.00',
                  bankFeeThb: '0.00',
                  paidAt: '2026-10-11T03:00:00Z',
                }),
                receipts: [makeTxReceiptDto({ number: 'XC-0001', revision: 1, issuedAt: '2026-10-11T03:00:05Z' })],
                approvalRequest: null,
                completedAt: '2026-10-11T03:00:05Z',
                cancelledAt: null,
                cancelReason: null,
                reversal: null,
                benefits: [],
                booking: null,
              }),
            }),
          );
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/transaction_audit') {
          return viewResponse(makeTransactionAuditViewData({ events: [] }));
        }
        if (method === 'GET' && u.pathname === '/api/v3/views/receipt_document') {
          return viewResponse(
            makeReceiptDocumentViewData({
              receipt: makeReceiptDocumentDto({ number: 'XC-0001', html: '<p>receipt</p>' }),
            }),
          );
        }
        if (method === 'POST' && u.pathname === `/api/transactions/${TX_ID}/reversal-requests`) {
          posts.push({ url: u.pathname, body: init?.body ? JSON.parse(String(init.body)) : null });
          return jsonResponse(200, { id: 'rv-0001', status: 'REQUESTED' });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );
  }

  it('R1: reversal with blank reason -> worded required alert shown, no request sent, dialog open', async () => {
    installTransactionFetch();
    dispose = render(() => <TransactionScreen pathname={PATH} search="" />, host);
    await flush();

    const reqBtn = host.querySelector<HTMLButtonElement>('button[data-action="request-reversal"]');
    expect(reqBtn, 'request-reversal button exists').not.toBeNull();
    reqBtn!.click();
    await flush();

    const dialog = host.querySelector('dialog');
    expect(dialog, 'ReasonDialog is open').not.toBeNull();

    const input = dialog!.querySelector<HTMLInputElement>('input');
    expect(input, 'dialog input').not.toBeNull();
    typeInto(input!, '   ');

    const confirmBtn = Array.from(dialog!.querySelectorAll('button')).find((b) => b.textContent === staffTh.confirm);
    expect(confirmBtn, 'confirm button').not.toBeNull();
    confirmBtn!.click();
    await flush();

    expect(posts).toHaveLength(0);
    const alert = dialog!.querySelector('[role="alert"]');
    expect(alert, 'role="alert" element').not.toBeNull();
    expect(alert!.textContent).toContain(staffTh.reasonRequired);
    expect(host.querySelector('dialog'), 'dialog stays open').not.toBeNull();
  });

  it('R2: reversal textarea/input carries maxLength=500 and holds at most 500 characters', async () => {
    installTransactionFetch();
    dispose = render(() => <TransactionScreen pathname={PATH} search="" />, host);
    await flush();

    const reqBtn = host.querySelector<HTMLButtonElement>('button[data-action="request-reversal"]');
    reqBtn!.click();
    await flush();

    const dialog = host.querySelector('dialog');
    const input = dialog!.querySelector<HTMLInputElement>('input');
    expect(input!.maxLength).toBe(500);

    const longText = 'x'.repeat(501);
    typeInto(input!, longText);
    expect(input!.value.length).toBe(500);

    expect(dialog!.textContent).toContain('500 / 500');
  });

  it('R3: reversal with valid reason -> request is sent, dialog closes after onConfirm resolves', async () => {
    installTransactionFetch();
    dispose = render(() => <TransactionScreen pathname={PATH} search="" />, host);
    await flush();

    const reqBtn = host.querySelector<HTMLButtonElement>('button[data-action="request-reversal"]');
    reqBtn!.click();
    await flush();

    const dialog = host.querySelector('dialog');
    const input = dialog!.querySelector<HTMLInputElement>('input');
    typeInto(input!, 'wrong amount');

    const confirmBtn = Array.from(dialog!.querySelectorAll('button')).find((b) => b.textContent === staffTh.confirm);
    confirmBtn!.click();
    await flush();

    expect(posts).toHaveLength(1);
    expect(posts[0]!.url).toBe(`/api/transactions/${TX_ID}/reversal-requests`);
    expect(posts[0]!.body).toEqual({ reason: 'wrong amount' });
    expect(host.querySelector('dialog'), 'dialog closes after onConfirm resolves').toBeNull();
  });

  it('R4: hold with blank reason -> worded line at the field, no request sent', async () => {
    const cust = makeStaffCustomerRow({ id: 'CX-0001', displayName: 'Customer A' });
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET' && rawUrl.includes('/api/v3/views/staff_holds')) {
          return viewResponse([]);
        }
        if (method === 'GET' && rawUrl.includes('/api/v3/views/staff_customers')) {
          return viewResponse([cust]);
        }
        if (method === 'POST' && rawUrl.includes('/api/customer-holds')) {
          posts.push({ url: rawUrl, body: init?.body ? JSON.parse(String(init.body)) : null });
          return jsonResponse(200, { id: 'h-1' });
        }
        return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
      }),
    );

    const ctx: StaffCtx = {
      me: makeStaffMeView({
        userId: 'u-1',
        branchId: 'BKK',
        permissions: ['CustomerHoldPlace'],
      }),
      floor: makeStaffFloorView({ phase: 'OPEN' }),
      refresh: vi.fn(async () => {}),
    } as unknown as StaffCtx;

    dispose = render(() => <HoldsCard ctx={ctx} />, host);
    await flush();

    const keyInput = host.querySelector<HTMLInputElement>('[data-testid="holds-key"]')!;
    const findBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-find"]')!;
    typeInto(keyInput, 'Customer');
    findBtn.click();
    await flush();

    const custRow = host.querySelector<HTMLElement>(`[data-customer-id="${cust.id}"]`)!;
    custRow.click();
    await flush();

    const reasonInput = host.querySelector<HTMLInputElement>('[data-testid="holds-reason"]')!;
    expect(reasonInput.maxLength).toBe(500);
    typeInto(reasonInput, '   ');

    const placeBtn = host.querySelector<HTMLButtonElement>('[data-testid="holds-place"]')!;
    placeBtn.click();
    await flush();

    expect(posts).toHaveLength(0);
    const alert = host.querySelector('[role="alert"]');
    expect(alert, 'worded line at the field').not.toBeNull();
    expect(alert!.textContent).toContain(holdsTh.reasonRequired);
  });

  it('R5: kiosk cancel with OTHER and blank note -> noteRequired error shows, confirm disabled', async () => {
    const onConfirm = vi.fn();
    dispose = render(
      () => (
        <CancelReasonDialog
          open={true}
          onConfirm={onConfirm}
          onCancel={vi.fn()}
        />
      ),
      host,
    );
    await flush();

    const select = host.querySelector<HTMLSelectElement>('select[name="reason"]')!;
    select.value = 'OTHER';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();

    const noteTextarea = host.querySelector<HTMLTextAreaElement>('textarea[name="note"]')!;
    expect(noteTextarea.maxLength).toBe(200);
    typeInto(noteTextarea, '   ');
    await flush();

    const confirmBtn = host.querySelector<HTMLButtonElement>('button[data-action="confirm-cancel-request"]')!;
    expect(confirmBtn.disabled).toBe(true);

    const alert = host.querySelector('[role="alert"]');
    expect(alert, 'noteRequired alert is shown').not.toBeNull();
    expect(alert!.textContent).toContain(sharedT('kioskCancel.noteRequired'));
  });

  it('R6: ReasonDialog without props with blank reason -> onConfirm is called', async () => {
    const onConfirm = vi.fn();
    const [open, setOpen] = createSignal(true);
    dispose = render(
      () => (
        <ReasonDialog
          open={open()}
          title="Unconstrained"
          onConfirm={(r) => {
            onConfirm(r);
            setOpen(false);
          }}
          onCancel={() => setOpen(false)}
        />
      ),
      host,
    );
    await flush();

    const dialog = host.querySelector('dialog')!;
    const input = dialog.querySelector<HTMLInputElement>('input')!;
    expect(input.maxLength).toBe(-1); // default / no maxLength prop
    expect(dialog.textContent).not.toContain('/');

    const confirmBtn = Array.from(dialog.querySelectorAll('button')).find((b) => b.textContent === staffTh.confirm)!;
    confirmBtn.click();
    await flush();

    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onConfirm).toHaveBeenCalledWith('');
  });
});
