// FE-W #5978: the receipt step while complete waits for a drawer top-up (design xchange-v3-wave3-desk.md §5.4a, as
// amended by PR #5977 after the owner's "ต้องเติมเงินก่อน ห้ามติดลบ"). Restated by claude/01TxaRJG (FE sub-controller) from
// #5932's tests: no request id and no rejection any more, so the old R3 (rejected) is removed. Do not edit, delete or
// weaken; add tests in a file of your own. The store is a stub, as in step.print.test.tsx. One test per row; the id is in the name.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../../shared/i18n';
import type { DeskStore, SaveOutcome } from '../../desk-store';
import * as texts from './i18n';
import { step } from './step';

const SHORT = 'ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: THB ลิ้นชัก ขาด 1711.46';

// §5.4a's words, verbatim.
const WORDS = {
  th: {
    stockWaitTitle: 'เงินในลิ้นชักไม่พอจ่ายรายการนี้',
    stockWaitNext:
      'ให้ผู้จัดการสาขาโอนเงินจากตู้เซฟเข้าลิ้นชักนี้ แล้วกดรับเงินที่หน้ากะ เมื่อเงินเข้าลิ้นชักแล้วหน้านี้จะบันทึกรายการต่อเอง',
    cancelSale: 'ยกเลิกรายการนี้',
  },
  en: {
    stockWaitTitle: 'The drawer does not hold enough to pay this transaction',
    stockWaitNext:
      'Ask the branch manager to move cash from the safe into this drawer, then receive it on the shift page. This page completes the transaction by itself once the cash is in the drawer',
    cancelSale: 'Cancel this transaction',
  },
} as const;

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function makeStore(wait: { message: string } | undefined, outcome: SaveOutcome, state: 'CONFIRMED' | 'CANCELLED' = 'CONFIRMED') {
  const [w, setW] = createSignal(wait);
  const [o, setO] = createSignal<SaveOutcome>(outcome);
  return {
    path: { kind: 'staff', devicesLink: false },
    step: () => 4 as const,
    branch: () => 'MAIN',
    config: () => ({ shiftOpen: true, adminUrl: '/?branch=MAIN', paymentMethods: [] }),
    outcome: o,
    transactionState: () => state,
    saveError: () => undefined,
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

describe('§5.4a the receipt step while complete waits for a drawer top-up', () => {
  it('R0: the keys exist in th and en with the design wording; stockRejectedCancelled is gone', () => {
    for (const lang of ['th', 'en'] as const) {
      for (const [k, v] of Object.entries(WORDS[lang])) {
        expect((texts[lang] as Record<string, string>)[k], `desk.steps.receipt ${lang}.${k}`).toBe(v);
      }
      expect((texts[lang] as Record<string, string>).stockRejectedCancelled, `${lang}.stockRejectedCancelled removed`).toBeUndefined();
    }
  });

  it('R1 (S1): waiting shows the title, the server message as sent, then what to do, in that order; no code, no request id', () => {
    show(makeStore({ message: SHORT }, 'saving'));
    const t = text();
    const i = t.indexOf(WORDS.th.stockWaitTitle);
    const j = t.indexOf(SHORT);
    const k = t.indexOf(WORDS.th.stockWaitNext);
    expect(i, 'title').toBeGreaterThanOrEqual(0);
    expect(j, 'the server message after the title').toBeGreaterThan(i);
    expect(k, 'what to do after the message').toBeGreaterThan(j);
    expect(t).not.toContain('stockInsufficient');
    expect(t).not.toContain('{ref}');
    expect(host.querySelector('[title]')?.textContent?.startsWith('#') ?? false, 'no request-id element').toBe(false);
  });

  it('R2 (S1/S5): while waiting, print is disabled and cancelSale calls cancelStockWait once', async () => {
    const store = makeStore({ message: SHORT }, 'saving');
    show(store);
    expect(button('พิมพ์ใบเสร็จ NEOCAL 80 มม.')?.disabled).toBe(true);
    const cancel = button(WORDS.th.cancelSale);
    expect(cancel, 'a cancel button while waiting').toBeDefined();
    cancel!.click();
    await vi.waitFor(() => expect(store.cancelStockWait).toHaveBeenCalledTimes(1));
  });

  it('R3: no wait, no wait words and no cancelSale button', () => {
    show(makeStore(undefined, 'saving'));
    const t = text();
    expect(t).not.toContain(WORDS.th.stockWaitTitle);
    expect(button(WORDS.th.cancelSale)).toBeUndefined();
  });

  it('R4: the words follow setLang (en); the server message stays as sent', () => {
    setLang('en');
    show(makeStore({ message: SHORT }, 'saving'));
    const t = text();
    expect(t).toContain(WORDS.en.stockWaitTitle);
    expect(t).toContain(SHORT);
    expect(t).toContain(WORDS.en.stockWaitNext);
    expect(button(WORDS.en.cancelSale)).toBeDefined();
  });
});
