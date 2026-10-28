import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { th as sharedTh } from '../../../../shared/i18n/th';
import { ReceiptFrame } from '../../../../shared/receipt/ReceiptFrame';
import {
  makeMe,
  makeReceiptDocumentDto,
  makeReceiptDocumentViewData,
} from '../../../../shared/testing/fixtures';
import type { DeskStore, SaveOutcome } from '../../desk-store';
import * as texts from './i18n';
import { step } from './step';

const DOC = 'XC-20261028-000001';
const RECEIPT_HTML = '<article>stored receipt</article>';
const PRINT_INITIAL = 'พิมพ์ใบเสร็จ NEOCAL 80 มม.';
const PRINT_AGAIN = 'พิมพ์อีกครั้ง';
const PRINTING_TEXT = 'กำลังพิมพ์ใบเสร็จ…';
const PRINTED_TEXT = 'ส่งใบเสร็จไปพิมพ์แล้ว';
const FAILED_PREFIX = 'บันทึกรายการแล้ว แต่พิมพ์ใบเสร็จไม่สำเร็จ — ';

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let receiptRequests: URL[];
let onReceiptRead: () => Response | Promise<Response>;

function makeStore(initialOutcome: SaveOutcome = 'saved') {
  const [outcome, setOutcome] = createSignal<SaveOutcome>(initialOutcome);
  const fetchFn: Fetch = vi.fn(async (input: string) => {
    const u = new URL(input, 'http://localhost');
    if (u.pathname === '/api/v3/views/me') {
      return jsonResponse(200, {
        version: 1,
        changed: true,
        data: makeMe(),
      });
    }
    if (u.pathname === '/api/v3/views/receipt_document') {
      receiptRequests.push(u);
      return onReceiptRead();
    }
    return jsonResponse(404, { error: { code: 'notFound', message: 'not found' } });
  });

  const store = {
    api: new ApiClient(fetchFn),
    path: { kind: 'owner', devicesLink: true },
    step: () => 4 as const,
    branch: () => 'MAIN',
    config: () => ({ shiftOpen: true, adminUrl: '/?branch=MAIN', paymentMethods: [] }),
    outcome,
    transactionState: () => (outcome() === 'saved' ? 'COMPLETED' : undefined),
    saveError: () => undefined,
    saved: () =>
      outcome() === 'saved'
        ? { id: 'tx-1', status: 'COMPLETED' as const, documentRef: DOC }
        : undefined,
    submit: vi.fn(async () => 'saved' as const),
    backToEdit: vi.fn(),
    startNew: vi.fn(() => {
      setOutcome('idle');
      return true;
    }),
    go: vi.fn(),
    _setOutcome: setOutcome,
  };

  return store;
}

const findButton = (label: string) =>
  [...host.querySelectorAll<HTMLButtonElement>('button')].find(
    (b) => (b.textContent ?? '').trim() === label,
  );

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  receiptRequests = [];
  onReceiptRead = () =>
    jsonResponse(200, {
      version: 1,
      changed: true,
      data: makeReceiptDocumentViewData({
        receipt: makeReceiptDocumentDto({ number: DOC, html: RECEIPT_HTML }),
      }),
    });
  vi.spyOn(ReceiptFrame, 'print').mockResolvedValue(undefined);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.restoreAllMocks();
  setLang('th');
});

describe('steps/receipt print flow (#5323 §8 & §9)', () => {
  it('row 1: not saved -> Print disabled, no print state', () => {
    const store = makeStore('saving');
    dispose = render(() => step.component({ store: store as unknown as DeskStore }), host);

    const btn = findButton(PRINT_INITIAL);
    expect(btn).toBeDefined();
    expect(btn!.disabled).toBe(true);
    expect(host.textContent).not.toContain(PRINTING_TEXT);
    expect(host.textContent).not.toContain(PRINTED_TEXT);
    expect(host.textContent).not.toContain(FAILED_PREFIX);
    expect(host.querySelector('[role="alert"]')).toBeNull();
  });

  it('row 2: saved, click Print, view has html, frame resolves -> printing then printed; one receipt_document request with filter.number = documentRef; ReceiptFrame.print called with that html', async () => {
    let resolveFrame!: () => void;
    vi.spyOn(ReceiptFrame, 'print').mockImplementation(
      () =>
        new Promise<void>((r) => {
          resolveFrame = r;
        }),
    );

    const store = makeStore('saved');
    dispose = render(() => step.component({ store: store as unknown as DeskStore }), host);

    const btn = findButton(PRINT_INITIAL)!;
    expect(btn.disabled).toBe(false);
    btn.click();

    await vi.waitFor(() => {
      expect(host.textContent).toContain(PRINTING_TEXT);
      expect(btn.disabled).toBe(true);
      expect(ReceiptFrame.print).toHaveBeenCalledTimes(1);
    });

    expect(receiptRequests).toHaveLength(1);
    expect(receiptRequests[0]!.searchParams.get('number')).toBe(DOC);
    expect(ReceiptFrame.print).toHaveBeenCalledWith(RECEIPT_HTML);

    resolveFrame();

    await vi.waitFor(() => {
      expect(host.textContent).toContain(PRINTED_TEXT);
      expect(host.textContent).not.toContain(PRINTING_TEXT);
    });
    const againBtn = findButton(PRINT_AGAIN);
    expect(againBtn).toBeDefined();
    expect(againBtn!.disabled).toBe(false);
  });

  it('row 3: saved, view has no html -> failed with receipt-missing text; button "พิมพ์อีกครั้ง"', async () => {
    onReceiptRead = () =>
      jsonResponse(200, {
        version: 1,
        changed: true,
        data: makeReceiptDocumentViewData({ receipt: null }),
      });

    const store = makeStore('saved');
    dispose = render(() => step.component({ store: store as unknown as DeskStore }), host);

    findButton(PRINT_INITIAL)!.click();

    const missingText = (texts.th as Record<string, string>).receiptMissing;
    expect(missingText).toBeTruthy();

    await vi.waitFor(() => {
      const alert = host.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBe(`${FAILED_PREFIX}${missingText}`);
    });
    expect(ReceiptFrame.print).not.toHaveBeenCalled();
    expect(findButton(PRINT_AGAIN)?.disabled).toBe(false);
  });

  it('row 4: saved, view answers 500 -> failed with error.serverError text', async () => {
    onReceiptRead = () =>
      jsonResponse(500, { error: { code: 'serverError', message: 'boom' } });

    const store = makeStore('saved');
    dispose = render(() => step.component({ store: store as unknown as DeskStore }), host);

    findButton(PRINT_INITIAL)!.click();

    await vi.waitFor(() => {
      const alert = host.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBe(`${FAILED_PREFIX}${sharedTh['error.serverError']}`);
    });
    expect(ReceiptFrame.print).not.toHaveBeenCalled();
    expect(findButton(PRINT_AGAIN)?.disabled).toBe(false);
  });

  it('row 5: saved, ReceiptFrame.print rejects -> failed', async () => {
    vi.spyOn(ReceiptFrame, 'print').mockRejectedValue(new TypeError('Failed to fetch'));

    const store = makeStore('saved');
    dispose = render(() => step.component({ store: store as unknown as DeskStore }), host);

    findButton(PRINT_INITIAL)!.click();

    await vi.waitFor(() => {
      const alert = host.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toBe(`${FAILED_PREFIX}${sharedTh['error.network']}`);
    });
    expect(findButton(PRINT_AGAIN)?.disabled).toBe(false);
  });

  it('row 6: after failed, click "พิมพ์อีกครั้ง" and it works -> printed', async () => {
    let calls = 0;
    onReceiptRead = () => {
      calls += 1;
      if (calls === 1) {
        return jsonResponse(500, { error: { code: 'serverError', message: 'boom' } });
      }
      return jsonResponse(200, {
        version: 1,
        changed: true,
        data: makeReceiptDocumentViewData({
          receipt: makeReceiptDocumentDto({ number: DOC, html: RECEIPT_HTML }),
        }),
      });
    };

    const store = makeStore('saved');
    dispose = render(() => step.component({ store: store as unknown as DeskStore }), host);

    findButton(PRINT_INITIAL)!.click();
    await vi.waitFor(() => expect(host.querySelector('[role="alert"]')).not.toBeNull());

    findButton(PRINT_AGAIN)!.click();
    await vi.waitFor(() => {
      expect(host.textContent).toContain(PRINTED_TEXT);
      expect(host.querySelector('[role="alert"]')).toBeNull();
    });
    expect(ReceiptFrame.print).toHaveBeenCalledTimes(1);
    expect(ReceiptFrame.print).toHaveBeenCalledWith(RECEIPT_HTML);
  });

  it('row 7: a print failure keeps the save outcome saved (save texts unchanged)', async () => {
    onReceiptRead = () =>
      jsonResponse(500, { error: { code: 'serverError', message: 'boom' } });

    const store = makeStore('saved');
    dispose = render(() => step.component({ store: store as unknown as DeskStore }), host);

    findButton(PRINT_INITIAL)!.click();
    await vi.waitFor(() => expect(host.querySelector('[role="alert"]')).not.toBeNull());

    expect(store.outcome()).toBe('saved');
    expect(host.textContent).toContain(texts.th.saved);
    expect(host.textContent).toContain(texts.th.documentRef);
    expect(host.textContent).toContain(DOC);
    expect(host.textContent).toContain(texts.th.adminLink);
  });

  it('edge 1: two clicks while printing send one request only because the button is disabled', async () => {
    let resolveRead!: (res: Response) => void;
    onReceiptRead = () =>
      new Promise<Response>((r) => {
        resolveRead = r;
      });

    const store = makeStore('saved');
    dispose = render(() => step.component({ store: store as unknown as DeskStore }), host);

    const btn = findButton(PRINT_INITIAL)!;
    btn.click();
    expect(btn.disabled).toBe(true);
    btn.click();

    expect(receiptRequests).toHaveLength(1);

    resolveRead(
      jsonResponse(200, {
        version: 1,
        changed: true,
        data: makeReceiptDocumentViewData({
          receipt: makeReceiptDocumentDto({ number: DOC, html: RECEIPT_HTML }),
        }),
      }),
    );

    await vi.waitFor(() => expect(host.textContent).toContain(PRINTED_TEXT));
    expect(receiptRequests).toHaveLength(1);
  });

  it('edge 2: "ทำรายการใหม่" after printed resets the next sale to idle', async () => {
    const store = makeStore('saved');
    dispose = render(() => step.component({ store: store as unknown as DeskStore }), host);

    findButton(PRINT_INITIAL)!.click();
    await vi.waitFor(() => expect(host.textContent).toContain(PRINTED_TEXT));

    findButton(texts.th.newExchange)!.click();
    expect(store.startNew).toHaveBeenCalledTimes(1);

    // Next sale reaches saved again
    store._setOutcome('saved');
    expect(host.textContent).not.toContain(PRINTED_TEXT);
    expect(findButton(PRINT_INITIAL)?.disabled).toBe(false);
    expect(findButton(PRINT_AGAIN)).toBeUndefined();
  });
});
