// KC-R (#kiosk-cancel FE, docs/design/blueprint/kiosk-cancel.md §2, §3.5, §5 row KC-R): the shared cancel-reason
// dialog, the shared cancel call, and the `kioskCancel.*` texts. Written before the implementation by the
// sub-controller claude/36d9dc6a. Do not edit, delete or weaken these tests; add tests in a file of your own.
//
// The API these tests pin:
//
//   web/src/shared/ui/CancelReasonDialog.tsx
//     export type StaffCancelReason = 'CUSTOMER_LEFT' | 'CUSTOMER_CHANGED_MIND' | 'DUPLICATE' | 'OTHER';
//     export type KioskCancelReason = StaffCancelReason | 'CUSTOMER_CANCELLED' | 'DAY_CLOSED';
//     export const STAFF_CANCEL_REASONS: readonly StaffCancelReason[]   // the four, in this order
//     export interface CancelReasonDialogProps {
//       open: boolean;
//       onConfirm: (choice: { reason: StaffCancelReason; note?: string }) => void;
//       onCancel: () => void;
//     }
//     export function CancelReasonDialog(props): JSX.Element
//     Labels: `kioskCancel.reason.<code in camelCase>` (customerLeft, customerChangedMind, duplicate, other,
//     customerCancelled, dayClosed), because shared keys are lowercase-first camelCase (i18n.test.ts:259).
//       DOM: <dialog open class="xc-dialog"> with select[name="reason"], textarea[name="note"],
//            button[data-action="confirm-cancel-request"] and button[data-action="dismiss"].
//
//   web/src/shared/api/kiosk-cancel.ts
//     export interface KioskIdentityAnswer { id: string; queue: string; orderNo: string | null;
//       transactionNo: string | null; status: string }
//     export function cancelKioskRequest(api: Pick<ApiClient, 'post'>,
//       input: { id: string; branchId: string; reason: StaffCancelReason; note?: string }): Promise<KioskIdentityAnswer>
//       → api.post('/api/kiosk', { action: 'cancel', payload: { id, branchId, reason, note? } })
//
// The wire shape and refusal sentences are AG controller KC's (#564 issuecomment-6307996084): the answer is
// `identity_json` (`status: "CANCELLED"`), and a refusal is `400 {"error": "<sentence>"}`, which ApiClient turns into
// ApiError code `refused` with the sentence as its message (client.post.test.ts). The dialog never words a refusal
// itself; the caller shows the server's sentence.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError } from '../api/client';
import { cancelKioskRequest } from '../api/kiosk-cancel';
import { setLang } from '../i18n';
import { en } from '../i18n/en';
import { th } from '../i18n/th';
import { zh } from '../i18n/zh';
import { CancelReasonDialog, STAFF_CANCEL_REASONS, type StaffCancelReason } from './CancelReasonDialog';

const STAFF_CODES: readonly StaffCancelReason[] = ['CUSTOMER_LEFT', 'CUSTOMER_CHANGED_MIND', 'DUPLICATE', 'OTHER'];
const HISTORY_ONLY = ['CUSTOMER_CANCELLED', 'DAY_CLOSED'] as const;
const ALL_CODES = [...STAFF_CODES, ...HISTORY_ONLY];

/** The i18n key of a reason code: shared keys are lowercase-first camelCase segments (i18n.test.ts:259). */
function reasonKey(code: string): string {
  const camel = code.toLowerCase().replace(/_([a-z])/g, (_m, c: string) => c.toUpperCase());
  return `kioskCancel.reason.${camel}`;
}

function table(lang: Record<string, string>): Record<string, string> {
  return lang as unknown as Record<string, string>;
}

function select(root: ParentNode): HTMLSelectElement {
  const el = root.querySelector<HTMLSelectElement>('dialog.xc-dialog select[name="reason"]');
  if (!el) throw new Error('select[name="reason"] inside dialog.xc-dialog not found');
  return el;
}

function note(root: ParentNode): HTMLTextAreaElement {
  const el = root.querySelector<HTMLTextAreaElement>('dialog.xc-dialog textarea[name="note"]');
  if (!el) throw new Error('textarea[name="note"] inside dialog.xc-dialog not found');
  return el;
}

function confirmBtn(root: ParentNode): HTMLButtonElement {
  const el = root.querySelector<HTMLButtonElement>('button[data-action="confirm-cancel-request"]');
  if (!el) throw new Error('button[data-action="confirm-cancel-request"] not found');
  return el;
}

function dismissBtn(root: ParentNode): HTMLButtonElement {
  const el = root.querySelector<HTMLButtonElement>('button[data-action="dismiss"]');
  if (!el) throw new Error('button[data-action="dismiss"] not found');
  return el;
}

function pick(root: ParentNode, code: string): void {
  const s = select(root);
  s.value = code;
  s.dispatchEvent(new Event('change', { bubbles: true }));
}

function typeNote(root: ParentNode, value: string): void {
  const n = note(root);
  n.value = value;
  n.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('CancelReasonDialog (KC-R, kiosk-cancel.md §2 reason table, §3.5)', () => {
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
  });

  function mount() {
    const [open, setOpen] = createSignal(true);
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    dispose = render(
      () => (
        <CancelReasonDialog
          open={open()}
          onConfirm={(c) => {
            onConfirm(c);
            setOpen(false);
          }}
          onCancel={() => {
            onCancel();
            setOpen(false);
          }}
        />
      ),
      host,
    );
    return { setOpen, onConfirm, onCancel };
  }

  it('row 1: lists exactly the four staff codes in order, defaulting to CUSTOMER_LEFT, each with its kioskCancel label', () => {
    mount();
    const options = Array.from(select(host).querySelectorAll('option'));
    expect(options.map((o) => o.value)).toEqual([...STAFF_CODES]);
    expect(STAFF_CANCEL_REASONS).toEqual(STAFF_CODES);
    expect(select(host).value).toBe('CUSTOMER_LEFT');
    for (const o of options) {
      expect(o.textContent?.trim()).toBe(table(th)[reasonKey(o.value)]);
    }
    // CUSTOMER_CANCELLED and DAY_CLOSED are never offered to staff (§2: "not offered to staff").
    for (const code of HISTORY_ONLY) {
      expect(options.some((o) => o.value === code)).toBe(false);
    }
  });

  it('row 2: confirm with the default sends CUSTOMER_LEFT and no note when the note is blank', () => {
    const { onConfirm } = mount();
    expect(confirmBtn(host).disabled).toBe(false);
    confirmBtn(host).click();
    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onConfirm).toHaveBeenCalledWith({ reason: 'CUSTOMER_LEFT' });
  });

  it('row 3: OTHER needs a note of 1-200 chars after trimming; confirm stays disabled until then and the note is sent trimmed', () => {
    const { onConfirm } = mount();
    pick(host, 'OTHER');
    expect(confirmBtn(host).disabled).toBe(true);
    typeNote(host, '   ');
    expect(confirmBtn(host).disabled).toBe(true);
    typeNote(host, '  left in a taxi  ');
    expect(confirmBtn(host).disabled).toBe(false);
    confirmBtn(host).click();
    expect(onConfirm).toHaveBeenCalledWith({ reason: 'OTHER', note: 'left in a taxi' });
  });

  it('row 4: every code accepts a note of up to 200 chars; 201 chars keeps confirm disabled', () => {
    const { onConfirm } = mount();
    pick(host, 'DUPLICATE');
    typeNote(host, 'x'.repeat(201));
    expect(confirmBtn(host).disabled).toBe(true);
    typeNote(host, 'x'.repeat(200));
    expect(confirmBtn(host).disabled).toBe(false);
    confirmBtn(host).click();
    expect(onConfirm).toHaveBeenCalledWith({ reason: 'DUPLICATE', note: 'x'.repeat(200) });
  });

  it('row 5: each open starts empty: reason back to CUSTOMER_LEFT, note cleared (as ReasonDialog, #2248)', () => {
    const { setOpen } = mount();
    pick(host, 'OTHER');
    typeNote(host, 'something');
    setOpen(false);
    setOpen(true);
    expect(select(host).value).toBe('CUSTOMER_LEFT');
    expect(note(host).value).toBe('');
  });

  it('row 6: dismiss calls onCancel and never onConfirm', () => {
    const { onConfirm, onCancel } = mount();
    dismissBtn(host).click();
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('row 7: the dialog words itself in the current language (en)', () => {
    setLang('en');
    mount();
    const options = Array.from(select(host).querySelectorAll('option'));
    for (const o of options) {
      expect(o.textContent?.trim()).toBe(table(en)[reasonKey(o.value)]);
    }
  });
});

describe('kioskCancel.* texts (KC-R: web/src/shared/i18n/{th,en,zh}.ts)', () => {
  it('row 8: all six reason codes have a label in th, en and zh, including the two history-only codes', () => {
    for (const lang of [th, en, zh]) {
      for (const code of ALL_CODES) {
        const label = table(lang)[reasonKey(code)];
        expect(label, reasonKey(code)).toBeTruthy();
      }
    }
  });

  it('row 9: th labels match the design table (kiosk-cancel.md §2)', () => {
    expect(table(th)['kioskCancel.reason.customerLeft']).toBe('ลูกค้าไม่อยู่ / เดินออกไปแล้ว');
    expect(table(th)['kioskCancel.reason.customerCancelled']).toBe('ลูกค้ายกเลิกเองที่ตู้');
    expect(table(th)['kioskCancel.reason.dayClosed']).toBe('ปิดวันทำการ');
    expect(table(th)['kioskCancel.reason.customerChangedMind']).toBe('ลูกค้าเปลี่ยนใจ');
    expect(table(th)['kioskCancel.reason.duplicate']).toBe('คำขอซ้ำ');
    expect(table(th)['kioskCancel.reason.other']).toBe('อื่น ๆ');
  });

  it('row 10: every kioskCancel.* key in th exists in en and zh', () => {
    const keys = Object.keys(th).filter((k) => k.startsWith('kioskCancel.'));
    expect(keys.length).toBeGreaterThanOrEqual(ALL_CODES.length);
    for (const k of keys) {
      expect(table(en)[k], `en ${k}`).toBeTruthy();
      expect(table(zh)[k], `zh ${k}`).toBeTruthy();
    }
  });
});

describe('cancelKioskRequest (KC-R: web/src/shared/api/kiosk-cancel.ts, wire shape #564 issuecomment-6307996084)', () => {
  const IDENTITY = {
    id: 'K-1',
    queue: 'K007',
    orderNo: null,
    transactionNo: null,
    status: 'CANCELLED',
  };

  it('row 11: posts action cancel with id, branchId and reason, and no note key when none is given', async () => {
    const post = vi.fn(async () => IDENTITY);
    const answer = await cancelKioskRequest({ post } as never, {
      id: 'K-1',
      branchId: 'MAIN',
      reason: 'CUSTOMER_LEFT',
    });
    expect(post).toHaveBeenCalledTimes(1);
    const [path, body] = post.mock.calls[0] as unknown as [string, { action: string; payload: Record<string, unknown> }];
    expect(path).toBe('/api/kiosk');
    expect(body).toEqual({ action: 'cancel', payload: { id: 'K-1', branchId: 'MAIN', reason: 'CUSTOMER_LEFT' } });
    expect('note' in body.payload).toBe(false);
    expect(answer.status).toBe('CANCELLED');
  });

  it('row 12: sends the note when given', async () => {
    const post = vi.fn(async () => IDENTITY);
    await cancelKioskRequest({ post } as never, { id: 'K-1', branchId: 'MAIN', reason: 'OTHER', note: 'left' });
    const [, body] = post.mock.calls[0] as unknown as [string, { payload: Record<string, unknown> }];
    expect(body.payload).toEqual({ id: 'K-1', branchId: 'MAIN', reason: 'OTHER', note: 'left' });
  });

  it('row 13: a server refusal reaches the caller unchanged, with the server sentence as its message', async () => {
    const refusal = new ApiError({ code: 'refused', message: 'คำขอนี้ยกเลิกไม่ได้', status: 400 });
    const post = vi.fn(async () => {
      throw refusal;
    });
    await expect(
      cancelKioskRequest({ post } as never, { id: 'K-1', branchId: 'MAIN', reason: 'DUPLICATE' }),
    ).rejects.toBe(refusal);
  });
});
