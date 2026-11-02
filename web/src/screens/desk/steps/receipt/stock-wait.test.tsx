// G5-FE #5932: the receipt step while complete waits for a negative-stock approval (design xchange-v3-wave3-desk.md §5.4a,
// PR #5931). Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken;
// add tests in a file of your own. The store is a stub, as in step.print.test.tsx. One test per row; the id is in the name.
// S4 is the owner's option A (2026-11-02, "A ตามเสนอ"): a rejection has already cancelled the sale.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import type { DeskStore, SaveOutcome } from '../../desk-store';
import * as texts from './i18n';
import { step } from './step';

const REQ = '167d8c93-7648-405c-b015-8fc53f176e4a';

// §5.4a's words, verbatim.
const WORDS = {
  th: {
    stockWaitTitle: 'ยอดเงินในลิ้นชักไม่พอสำหรับรายการนี้ ต้องให้ผู้จัดการสาขาอนุมัติก่อน',
    stockWaitNext: 'ให้ผู้จัดการสาขาอนุมัติคำขอ {ref} ที่หน้าอนุมัติ เมื่ออนุมัติแล้วหน้านี้จะบันทึกรายการต่อเอง',
    stockRejectedCancelled: 'ผู้จัดการสาขาไม่อนุมัติ รายการนี้ถูกยกเลิกแล้ว เริ่มรายการใหม่ได้',
    cancelSale: 'ยกเลิกรายการนี้',
  },
  en: {
    stockWaitTitle: 'The drawer does not hold enough for this transaction. The branch manager must approve it first',
    stockWaitNext:
      'Ask the branch manager to approve request {ref} on the Approvals page. This page completes the transaction by itself once it is approved',
    stockRejectedCancelled: 'The branch manager rejected it. This transaction has been cancelled; you can start a new one',
    cancelSale: 'Cancel this transaction',
  },
} as const;

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function makeStore(
  wait: { requestId: string; rejected: boolean } | undefined,
  outcome: SaveOutcome,
  state: 'CONFIRMED' | 'CANCELLED' = 'CONFIRMED',
) {
  const [w, setW] = createSignal(wait);
  const [o, setO] = createSignal<SaveOutcome>(outcome);
  return {
    path: { kind: 'staff', devicesLink: false },
    step: () => 4 as const,
    branch: () => 'MAIN',
    config: () => ({ shiftOpen: true, adminUrl: '/?branch=MAIN', paymentMethods: [] }),
    outcome: o,
    transactionState: () => state,
    saveError: () => (state === 'CANCELLED' ? 'เงินในลิ้นชักไม่พอ ไม่อนุมัติ' : undefined),
    saved: () => undefined,
    stockWait: w,
    cancelStockWait: vi.fn(async () => {
      setW(undefined);
      setO('refused');
    }),
    submit: vi.fn(async () => 'saving' as const),
    backToEdit: vi.fn(),
    startNew: vi.fn(() => true),
    go: vi.fn(),
  };
}

function show(store: ReturnType<typeof makeStore>): void {
  dispose = render(() => step.component({ store: store as unknown as DeskStore }), host);
}

function text(): string {
  return (host.textContent ?? '').replace(/\s+/g, ' ').trim();
}

const button = (label: string) =>
  [...host.querySelectorAll<HTMLButtonElement>('button')].find((b) => (b.textContent ?? '').trim() === label);

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  setLang('th');
});

describe('§5.4a the receipt step while complete waits for a NEGATIVE_STOCK approval', () => {
  it('R0: the four keys exist in th and en with the design wording', () => {
    for (const lang of ['th', 'en'] as const) {
      for (const [k, v] of Object.entries(WORDS[lang])) {
        expect((texts[lang] as Record<string, string>)[k], `desk.steps.receipt ${lang}.${k}`).toBe(v);
      }
    }
  });

  it('R1 (S1): waiting shows who decides and what to do, the request as #<8> with the full id in title; no raw code', () => {
    show(makeStore({ requestId: REQ, rejected: false }, 'saving'));
    const t = text();
    expect(t).toContain(WORDS.th.stockWaitTitle);
    expect(t).toContain(WORDS.th.stockWaitNext.replace('{ref}', '#167d8c93'));
    expect(host.querySelector(`[title="${REQ}"]`)?.textContent?.trim()).toBe('#167d8c93');
    expect(t).not.toContain('approvalRequired');
    expect(t).not.toContain(REQ);
  });

  it('R2 (S1/S5): while waiting, print is disabled and cancelSale calls cancelStockWait once', async () => {
    const store = makeStore({ requestId: REQ, rejected: false }, 'saving');
    show(store);
    expect(button('พิมพ์ใบเสร็จ NEOCAL 80 มม.')?.disabled).toBe(true);
    const cancel = button(WORDS.th.cancelSale);
    expect(cancel, 'a cancel button while waiting').toBeDefined();
    cancel!.click();
    await vi.waitFor(() => expect(store.cancelStockWait).toHaveBeenCalledTimes(1));
  });

  it('R3 (S4, option A): rejected and cancelled shows stockRejectedCancelled and the manager\'s reason, offers a new exchange, no cancel button', () => {
    show(makeStore({ requestId: REQ, rejected: true }, 'refused', 'CANCELLED'));
    const t = text();
    expect(t).toContain(WORDS.th.stockRejectedCancelled);
    expect(button(WORDS.th.cancelSale), 'nothing left to cancel').toBeUndefined();
    expect(button(texts.th.newExchange)?.disabled, 'a new exchange is offered').toBe(false);
    expect(t, "the manager's reason, as sent").toContain('เงินในลิ้นชักไม่พอ ไม่อนุมัติ');
    expect(t).not.toContain('staleVersion');
    expect(t).not.toContain(REQ);
  });

  it('R4: the words follow setLang (en)', () => {
    setLang('en');
    show(makeStore({ requestId: REQ, rejected: false }, 'saving'));
    const t = text();
    expect(t).toContain(WORDS.en.stockWaitTitle);
    expect(t).toContain(WORDS.en.stockWaitNext.replace('{ref}', '#167d8c93'));
    expect(button(WORDS.en.cancelSale)).toBeDefined();
  });
});
