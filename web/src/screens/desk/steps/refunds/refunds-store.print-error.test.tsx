// #2633: Desk refunds — when the refund receipt read fails, "Print refund slip" must say so (worded error, no Retry),
// keep the document number and Print on screen, and print normally when pressed again and the read succeeds.
// Renders the real RefundsStep over the real store; the poster and the api client are stubbed (as refunds-store.lastdoc).
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

/** Each receipt_document read takes the next outcome; an Error is thrown, anything else is the html. */
let receiptReplies: Array<Error | string> = [];
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
        return { data: { receipt: { html: r ?? RECEIPT_HTML } } };
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

const FAILURES: Array<{ name: string; make: () => Error; worded: string }> = [
  { name: 'network', make: () => new TypeError('Failed to fetch'), worded: sharedTh['error.network'] },
  {
    name: '500',
    make: () => new ApiError({ code: 'serverError', message: 'boom', status: 500 }),
    worded: sharedTh['error.serverError'],
  },
  {
    name: '4xx',
    make: () => new ApiError({ code: 'roleNotAllowedHere', message: 'no', status: 403 }),
    worded: sharedTh['error.roleNotAllowedHere'],
  },
];

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

describe('Desk refunds: Print refund slip when the receipt read fails (#2633 §8 & §9)', () => {
  it.each(FAILURES)(
    'row 1: receipt_document read fails ($name) -> worded error in the refunds alert; doc and Print still shown; no Retry',
    async ({ make, worded }) => {
      const store = await setupPosted();
      receiptReplies = [make()];

      await expect(store.printReceipt(), 'Print does not reject').resolves.toBeUndefined();

      expect(receiptReads).toHaveLength(1);
      expect(alertText()).toBe(worded);
      expect(shown(), 'document number still shown').toContain(DOC);
      expect(printButton(), 'Print still shown').not.toBeNull();
      expect(retryButton(), 'Retry not shown').toBeNull();
      expect(store.canRetry()).toBe(false);
      expect(store.lastPostedDoc()).toBe(DOC);
      expect(ReceiptFrame.print).not.toHaveBeenCalled();
    },
  );

  it('row 2: Print again after a failure, read succeeds -> ReceiptFrame.print called once with the html', async () => {
    const store = await setupPosted();
    receiptReplies = [new TypeError('Failed to fetch'), RECEIPT_HTML];

    await expect(store.printReceipt(), 'the failed press does not reject').resolves.toBeUndefined();
    expect(ReceiptFrame.print).not.toHaveBeenCalled();

    (printButton() as HTMLButtonElement).click();
    await vi.waitFor(() => expect(ReceiptFrame.print).toHaveBeenCalledTimes(1));
    expect(ReceiptFrame.print).toHaveBeenCalledWith(RECEIPT_HTML);
    expect(receiptReads).toHaveLength(2);
  });

  // Row "Read succeeds -> unchanged" is kept and pinned by refunds.test.tsx RF-19 and refunds-store.lastdoc.test.tsx
  // (unchanged); it is not repeated here because a kept row cannot go red on revert.

  it('edge: the print error clears on the next successful print', async () => {
    const store = await setupPosted();
    receiptReplies = [new ApiError({ code: 'serverError', message: 'boom', status: 500 }), RECEIPT_HTML];

    await expect(store.printReceipt()).resolves.toBeUndefined();
    expect(alertText()).toBe(sharedTh['error.serverError']);

    await store.printReceipt();
    expect(ReceiptFrame.print).toHaveBeenCalledTimes(1);
    expect(store.postError()).toBeNull();
    expect(host.querySelector('.desk-refunds-alert'), 'alert gone').toBeNull();
    expect(printButton(), 'Print still shown').not.toBeNull();
  });
});
