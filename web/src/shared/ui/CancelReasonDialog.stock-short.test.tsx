// #6094 (F5, rehearsal finding on #564): the cancel-reason dialog offers `STOCK_SHORT` ("เงินสดในลิ้นชักไม่พอ")
// right after `CUSTOMER_LEFT`, with an optional note, so a teller can record that the drawer could not cover the
// request (docs/design/blueprint/kiosk-cancel.md §2 as amended for #6094; the owner approved the migration,
// #6094 issuecomment-6627165045). Written before the implementation by the ticket author claude/01TxaRJG. Do not
// edit, delete or weaken these tests; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../i18n';
import { en } from '../i18n/en';
import { th } from '../i18n/th';
import { zh } from '../i18n/zh';
import { CancelReasonDialog, STAFF_CANCEL_REASONS } from './CancelReasonDialog';

const LABELS = {
  th: 'เงินสดในลิ้นชักไม่พอ',
  en: 'Not enough cash in the drawer',
  zh: '钱箱现金不足',
} as const;

function table(lang: Record<string, string>): Record<string, string> {
  return lang as unknown as Record<string, string>;
}

function select(root: ParentNode): HTMLSelectElement {
  const el = root.querySelector<HTMLSelectElement>('dialog.xc-dialog select[name="reason"]');
  if (!el) throw new Error('select[name="reason"] inside dialog.xc-dialog not found');
  return el;
}

function confirmBtn(root: ParentNode): HTMLButtonElement {
  const el = root.querySelector<HTMLButtonElement>('button[data-action="confirm-cancel-request"]');
  if (!el) throw new Error('button[data-action="confirm-cancel-request"] not found');
  return el;
}

function pick(root: ParentNode, code: string): void {
  const s = select(root);
  s.value = code;
  s.dispatchEvent(new Event('change', { bubbles: true }));
}

function typeNote(root: ParentNode, value: string): void {
  const n = root.querySelector<HTMLTextAreaElement>('dialog.xc-dialog textarea[name="note"]');
  if (!n) throw new Error('textarea[name="note"] inside dialog.xc-dialog not found');
  n.value = value;
  n.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('CancelReasonDialog: STOCK_SHORT (#6094)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount() {
    const [open, setOpen] = createSignal(true);
    const onConfirm = vi.fn();
    dispose = render(
      () => (
        <CancelReasonDialog
          open={open()}
          onConfirm={(c) => {
            onConfirm(c);
            setOpen(false);
          }}
          onCancel={() => setOpen(false)}
        />
      ),
      host,
    );
    return { onConfirm };
  }

  it('F5-1: STOCK_SHORT is offered second, right after the default CUSTOMER_LEFT', () => {
    mount();
    const values = Array.from(select(host).querySelectorAll('option')).map((o) => o.value);
    expect(values.slice(0, 2)).toEqual(['CUSTOMER_LEFT', 'STOCK_SHORT']);
    expect(STAFF_CANCEL_REASONS.indexOf('STOCK_SHORT')).toBe(1);
    expect(select(host).value).toBe('CUSTOMER_LEFT');
  });

  it('F5-2: STOCK_SHORT with a blank note confirms and sends no note', () => {
    const { onConfirm } = mount();
    pick(host, 'STOCK_SHORT');
    typeNote(host, '   ');
    expect(confirmBtn(host).disabled).toBe(false);
    confirmBtn(host).click();
    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onConfirm).toHaveBeenCalledWith({ reason: 'STOCK_SHORT' });
  });

  it('F5-3: STOCK_SHORT sends its note trimmed; 201 characters keeps confirm disabled', () => {
    const { onConfirm } = mount();
    pick(host, 'STOCK_SHORT');
    typeNote(host, 'x'.repeat(201));
    expect(confirmBtn(host).disabled).toBe(true);
    typeNote(host, '  USD ขาด 7  ');
    expect(confirmBtn(host).disabled).toBe(false);
    confirmBtn(host).click();
    expect(onConfirm).toHaveBeenCalledWith({ reason: 'STOCK_SHORT', note: 'USD ขาด 7' });
  });

  // Staff screens speak th and en only (`SPOKEN`, i18n/index.ts), so zh is checked through its table (F5-5).
  it('F5-4: the option is worded in the current language (th, en)', () => {
    for (const lang of ['th', 'en'] as const) {
      setLang(lang);
      mount();
      const option = Array.from(select(host).querySelectorAll('option')).find((o) => o.value === 'STOCK_SHORT');
      expect(option?.textContent?.trim(), lang).toBe(LABELS[lang]);
      dispose?.();
      dispose = undefined;
    }
  });

  it('F5-5: kioskCancel.reason.stockShort reads as the design table says, in th, en and zh', () => {
    expect(table(th)['kioskCancel.reason.stockShort']).toBe(LABELS.th);
    expect(table(en)['kioskCancel.reason.stockShort']).toBe(LABELS.en);
    expect(table(zh)['kioskCancel.reason.stockShort']).toBe(LABELS.zh);
  });
});
