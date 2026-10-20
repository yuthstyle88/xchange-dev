// #2737: Desk refunds — when receipt_document answers { receipt: null } or { receipt: { html: '' } },
// "Print refund slip" must show t('error.receiptMissing') in .desk-refunds-alert, not call ReceiptFrame.print,
// keep the document number and Print on screen with no Retry, and clear the alert on the next successful print.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient } from '../../../../shared/api/client';
import { makeDeskRefundRow, makeReversalSummaryDto } from '../../../../shared/testing/fixtures';
import { setLang } from '../../../../shared/i18n';
import { th as sharedTh } from '../../../../shared/i18n/th';
import { ReceiptFrame } from '../../../../shared/receipt/ReceiptFrame';

vi.mock('./post', () => ({
  createPoster: () => ({
    async post(): Promise<string> {
      return 'XC-0001-R1';
    },
  }),
}));

const { createRefundsStore } = await import('./refunds-store');
const { RefundsStep } = await import('./RefundsStep');

const DOC = 'XC-0001-R1';
const RECEIPT_HTML = '<p>refund receipt</p>';
const RECEIPT_MISSING_TH = 'ไม่พบใบคืนเงินของรายการนี้ กรุณาลองใหม่หรือแจ้งผู้ดูแล';

function row(id: string) {
  return makeDeskRefundRow({
    reversal: makeReversalSummaryDto({
      id,
      originalTransactionId: `tx-${id}`,
      status: 'APPROVED',
      reason: 'r',
      amount: '1.00',
      requestedBy: 'u',
      requestedAt: '2026-10-11T04:00:00Z',
      approvedAt: '2026-10-11T04:10:00Z',
      postedAt: null,
      documentNumber: null,
    }),
    receiptNumber: `XC-${id}`,
    lines: [],
    payments: [],
  });
}

type ReceiptReply = Error | { html: string } | null;

let receiptReplies: ReceiptReply[] = [];
let receiptReads: unknown[] = [];
let dispose: (() => void) | undefined;
let host: HTMLDivElement;

function fakeApi(): ApiClient {
  return {
    async view(name: string, opts?: unknown) {
      if (name === 'desk_refunds') return { data: { rows: [row('rv-1')] } };
      if (name === 'receipt_document') {
        receiptReads.push(opts);
        const r = receiptReplies.shift();
        if (r instanceof Error) throw r;
        return { data: { receipt: r === undefined ? { html: RECEIPT_HTML } : r } };
      }
      return { data: null };
    },
  } as unknown as ApiClient;
}

async function setupPosted() {
  const store = createRefundsStore(fakeApi());
  store.setOpen(true);
  await store.readRefunds();
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <RefundsStep store={store} />, host);
  store.startConfirm('rv-1');
  await store.submitPost();
  expect(store.lastPostedDoc()).toBe(DOC);
  return store;
}

const alertText = () =>
  (host.querySelector('.desk-refunds-alert')?.textContent ?? '').replace(/\s+/g, ' ').trim();
const printButton = () => host.querySelector('button[data-action="print"]');
const retryButton = () => host.querySelector('button[data-action="retry"]');
const shown = () => (host.textContent ?? '').replace(/\s+/g, ' ');

beforeEach(() => {
  setLang('th');
  receiptReplies = [];
  receiptReads = [];
  vi.spyOn(ReceiptFrame, 'print').mockResolvedValue(undefined);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

describe('Desk refunds: Print refund slip when receipt is missing (#2737 §8)', () => {
  it('R1: receipt_document answers { receipt: null } -> shows error.receiptMissing alert; no ReceiptFrame.print; doc and Print still shown; no Retry', async () => {
    const store = await setupPosted();
    receiptReplies = [null];

    await expect(store.printReceipt(), 'Print does not reject').resolves.toBeUndefined();

    expect(receiptReads).toHaveLength(1);
    expect(alertText()).toBe(RECEIPT_MISSING_TH);
    expect(shown(), 'document number still shown').toContain(DOC);
    expect(printButton(), 'Print still shown').not.toBeNull();
    expect(retryButton(), 'Retry not shown').toBeNull();
    expect(store.canRetry()).toBe(false);
    expect(store.lastPostedDoc()).toBe(DOC);
    expect(ReceiptFrame.print).not.toHaveBeenCalled();
  });

  it('R2: after R1 alert is shown, press Print again and read answers html -> ReceiptFrame.print called once with html and alert is gone', async () => {
    const store = await setupPosted();
    receiptReplies = [null, { html: RECEIPT_HTML }];

    await expect(store.printReceipt()).resolves.toBeUndefined();
    expect(alertText()).toBe(RECEIPT_MISSING_TH);
    expect(ReceiptFrame.print).not.toHaveBeenCalled();

    (printButton() as HTMLButtonElement).click();
    await vi.waitFor(() => expect(ReceiptFrame.print).toHaveBeenCalledTimes(1));
    expect(ReceiptFrame.print).toHaveBeenCalledWith(RECEIPT_HTML);
    expect(receiptReads).toHaveLength(2);
    expect(store.postError()).toBeNull();
    expect(host.querySelector('.desk-refunds-alert'), 'alert gone').toBeNull();
  });

  it('R3: a failed read showed the #2633 worded error, then Print answers { receipt: null } -> alert shows error.receiptMissing (does not go blank)', async () => {
    const store = await setupPosted();
    receiptReplies = [
      new ApiError({ code: 'serverError', message: 'boom', status: 500 }),
      null,
    ];

    await expect(store.printReceipt()).resolves.toBeUndefined();
    expect(alertText()).toBe(sharedTh['error.serverError']);

    await expect(store.printReceipt()).resolves.toBeUndefined();
    expect(alertText()).toBe(RECEIPT_MISSING_TH);
    expect(ReceiptFrame.print).not.toHaveBeenCalled();
    expect(store.canRetry()).toBe(false);
  });

  it("R4: receipt_document answers { receipt: { html: '' } } (empty html) -> same as R1", async () => {
    const store = await setupPosted();
    receiptReplies = [{ html: '' }];

    await expect(store.printReceipt(), 'Print does not reject').resolves.toBeUndefined();

    expect(receiptReads).toHaveLength(1);
    expect(alertText()).toBe(RECEIPT_MISSING_TH);
    expect(shown(), 'document number still shown').toContain(DOC);
    expect(printButton(), 'Print still shown').not.toBeNull();
    expect(retryButton(), 'Retry not shown').toBeNull();
    expect(store.canRetry()).toBe(false);
    expect(store.lastPostedDoc()).toBe(DOC);
    expect(ReceiptFrame.print).not.toHaveBeenCalled();
  });
});
