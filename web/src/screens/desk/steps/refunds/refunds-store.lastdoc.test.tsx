// #2456: Desk refunds — the posted document number and Print button of refund A must not stay on screen after the
// next post (B) is refused, or after the panel is closed and reopened; Print must never print A's receipt then.
// Renders the real RefundsStep over the real store; the poster and the api client are stubbed.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient } from '../../../../shared/api/client';
import type { DeskRefundRow } from '../../../../shared/api/generated/desk_refunds';
import { setLang } from '../../../../shared/i18n';
import { ReceiptFrame } from '../../../../shared/receipt/ReceiptFrame';
import * as texts from './i18n';

type Reply = { doc: string } | { refuse: string } | 'network';

let replies: Reply[] = [];
const postedIds: string[] = [];

vi.mock('./post', () => ({
  createPoster: (_api: unknown, id: string) => ({
    async post(): Promise<string> {
      postedIds.push(id);
      const r = replies.shift();
      if (!r || r === 'network') throw new TypeError('Failed to fetch');
      if ('refuse' in r) throw new ApiError({ code: r.refuse, message: r.refuse, status: 409 });
      return r.doc;
    },
  }),
}));

const { createRefundsStore } = await import('./refunds-store');
const { RefundsStep } = await import('./RefundsStep');

function row(id: string): DeskRefundRow {
  return {
    reversal: {
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
    },
    receiptNumber: `XC-${id}`,
    lines: [],
    payments: [],
  } as DeskRefundRow;
}

const DOC_A = 'XC-0001-R1';
const DOC_B = 'XC-0002-R1';

let viewCalls: Array<{ name: string; opts: unknown }> = [];
let dispose: (() => void) | undefined;
let host: HTMLDivElement;

function fakeApi(): ApiClient {
  return {
    async view(name: string, opts?: unknown) {
      viewCalls.push({ name, opts });
      if (name === 'desk_refunds') return { data: { rows: [row('rv-1'), row('rv-2')] } };
      if (name === 'receipt_document') return { data: { receipt: { html: '<p>receipt</p>' } } };
      return { data: null };
    },
  } as unknown as ApiClient;
}

async function setup() {
  const store = createRefundsStore(fakeApi());
  store.setOpen(true);
  await store.readRefunds();
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <RefundsStep store={store} />, host);
  return store;
}

async function post(store: Awaited<ReturnType<typeof setup>>, id: string): Promise<void> {
  store.startConfirm(id);
  await store.submitPost();
}

const shown = () => (host.textContent ?? '').replace(/\s+/g, ' ');
const printButton = () => host.querySelector('button[data-action="print"]');
const receiptPrints = () => viewCalls.filter((c) => c.name === 'receipt_document');

beforeEach(() => {
  setLang('th');
  replies = [];
  postedIds.length = 0;
  viewCalls = [];
  vi.spyOn(ReceiptFrame, 'print').mockResolvedValue(undefined);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

describe("Desk refunds: refund A's document and Print do not outlive A (#2456 §8 & §9)", () => {
  it("row 1: post A ok, then post B refused -> only B's error shows; no A document or Print; Print prints nothing", async () => {
    const store = await setup();
    replies = [{ doc: DOC_A }, { refuse: 'reversalWindowClosed' }];

    await post(store, 'rv-1');
    expect(shown()).toContain(DOC_A);

    await post(store, 'rv-2');
    expect(postedIds).toEqual(['rv-1', 'rv-2']);
    expect(shown()).toContain(texts.th['error.reversalWindowClosed']);
    expect(shown(), "A's document is gone").not.toContain(DOC_A);
    expect(printButton(), 'no Print button').toBeNull();
    expect(store.lastPostedDoc()).toBeNull();

    await store.printReceipt();
    expect(receiptPrints(), "Print does not fetch A's receipt").toEqual([]);
    expect(ReceiptFrame.print).not.toHaveBeenCalled();
  });

  it('row 2: post A ok, Close, reopen -> no A document or Print', async () => {
    const store = await setup();
    replies = [{ doc: DOC_A }];

    await post(store, 'rv-1');
    expect(shown()).toContain(DOC_A);
    expect(printButton()).not.toBeNull();

    (host.querySelector('button[data-action="close"]') as HTMLButtonElement).click();
    expect(store.open()).toBe(false);
    store.setOpen(true);
    await store.readRefunds();

    expect(shown(), "A's document is gone after reopen").not.toContain(DOC_A);
    expect(printButton(), 'no Print button after reopen').toBeNull();
    expect(store.lastPostedDoc()).toBeNull();

    await store.printReceipt();
    expect(receiptPrints()).toEqual([]);
  });

  // Row "Post A ok -> A's document and Print show" is kept and pinned by refunds.test.tsx RF-15/RF-18 (unchanged).

  it('edge: a network-error retry keeps no stale document; a successful retry shows only the new one', async () => {
    const store = await setup();
    replies = [{ doc: DOC_A }, 'network', 'network', { doc: DOC_B }];

    await post(store, 'rv-1');
    expect(shown()).toContain(DOC_A);

    await post(store, 'rv-2');
    expect(store.canRetry()).toBe(true);
    expect(shown()).toContain(texts.th['error.unknown']);
    expect(shown(), "A's document is gone once B starts").not.toContain(DOC_A);
    expect(printButton()).toBeNull();

    await store.retry();
    expect(store.canRetry()).toBe(true);
    expect(shown()).not.toContain(DOC_A);
    expect(printButton()).toBeNull();

    await store.retry();
    expect(postedIds).toEqual(['rv-1', 'rv-2', 'rv-2', 'rv-2']);
    expect(shown()).toContain(DOC_B);
    expect(shown()).not.toContain(DOC_A);
    expect(store.lastPostedDoc()).toBe(DOC_B);
  });
});
