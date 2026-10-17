import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import {
  Dialog,
  closeDialog,
  isDialogLocked,
  openDialog,
  setDialogLocked,
} from './Dialog';

let dispose: (() => void) | undefined;
let root: HTMLElement;

function mount(fn: () => JSX.Element): HTMLElement {
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(fn, root);
  return root;
}

afterEach(() => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  setLang('th');
});

function dialogEl(el: HTMLElement): HTMLDialogElement {
  const found = el.querySelectorAll('dialog');
  expect(found.length, 'the page has exactly one dialog').toBe(1);
  return found[0] as HTMLDialogElement;
}

describe('Dialog native close reconciliation (#2371)', () => {
  it('Locked dialog closed natively (dialog.open=false + close event) re-opens with the same title and body', () => {
    const el = mount(() => <Dialog />);
    openDialog('ยังไม่ยืนยันผลการบันทึก', () => (
      <p class="probe">กรุณาตรวจสอบก่อนทำรายการต่อ</p>
    ));
    setDialogLocked(true);

    const dialog = dialogEl(el);
    expect(dialog.open).toBe(true);
    expect(isDialogLocked()).toBe(true);

    // Simulate native browser close (e.g. Chrome close watcher on second Esc)
    dialog.open = false;
    dialog.removeAttribute('open');
    dialog.dispatchEvent(new Event('close'));

    expect(dialog.open).toBe(true);
    expect(dialog.hasAttribute('open')).toBe(true);
    expect(isDialogLocked()).toBe(true);
    expect(dialog.querySelector('h2')?.textContent).toBe('ยังไม่ยืนยันผลการบันทึก');
    expect(dialog.querySelector('.probe')?.textContent).toBe('กรุณาตรวจสอบก่อนทำรายการต่อ');
  });

  it('Locked dialog closed natively twice stays open each time with exactly one .dialog-content', () => {
    const el = mount(() => <Dialog />);
    openDialog('รายการไม่พบ', () => <p class="probe">เนื้อหาเดิม</p>);
    setDialogLocked(true);

    const dialog = dialogEl(el);
    const contentBefore = dialog.querySelector('.dialog-content');
    expect(contentBefore).not.toBeNull();

    // First native close
    dialog.open = false;
    dialog.removeAttribute('open');
    dialog.dispatchEvent(new Event('close'));
    expect(dialog.open).toBe(true);
    expect(dialog.querySelectorAll('.dialog-content').length).toBe(1);

    // Second native close
    dialog.open = false;
    dialog.removeAttribute('open');
    dialog.dispatchEvent(new Event('close'));
    expect(dialog.open).toBe(true);
    expect(dialog.querySelectorAll('.dialog-content').length).toBe(1);
    expect(dialog.querySelector('.dialog-content')).toBe(contentBefore);
    expect(dialog.querySelector('.probe')?.textContent).toBe('เนื้อหาเดิม');
  });

  it('Unlocked dialog closed natively (close event only) unmounts content and keeps isDialogLocked() false', () => {
    const el = mount(() => <Dialog />);
    openDialog('รายละเอียด', () => <p class="probe">เนื้อหา</p>);

    const dialog = dialogEl(el);
    expect(dialog.open).toBe(true);
    expect(dialog.querySelector('.dialog-content')).not.toBeNull();

    // Native close event on an unlocked dialog reconciles state via closeDialog()
    dialog.dispatchEvent(new Event('close'));

    expect(dialog.open).toBe(false);
    expect(dialog.querySelector('.dialog-content')).toBeNull();
    expect(isDialogLocked()).toBe(false);
  });

  it('Edge cases: showModal vs attribute fallback, effect close() with state() null is a no-op, and unlocked cancel closes', () => {
    const el = mount(() => <Dialog />);
    const dialog = dialogEl(el);

    // 1. When state() is null, a close event (even if locked is true) is a no-op and never opens the dialog
    setDialogLocked(true);
    dialog.dispatchEvent(new Event('close'));
    expect(dialog.open).toBe(false);
    setDialogLocked(false);

    // 2. Fallback path without showModal re-shows through the open attribute
    const origShowModal = dialog.showModal;
    Reflect.deleteProperty(dialog, 'showModal');
    openDialog('Fallback Title', () => <p class="probe">Fallback Body</p>);
    setDialogLocked(true);
    expect(dialog.open).toBe(true);

    dialog.open = false;
    dialog.removeAttribute('open');
    dialog.dispatchEvent(new Event('close'));
    expect(dialog.open).toBe(true);
    expect(dialog.hasAttribute('open')).toBe(true);

    // 3. When showModal is present, locked native close calls showModal()
    const showModalSpy = vi.fn(() => {
      dialog.open = true;
      dialog.setAttribute('open', '');
    });
    dialog.showModal = showModalSpy;
    dialog.open = false;
    dialog.removeAttribute('open');
    dialog.dispatchEvent(new Event('close'));
    expect(showModalSpy).toHaveBeenCalledTimes(1);
    expect(dialog.open).toBe(true);

    // 4. Effect's own close() firing 'close' with state() null is a no-op (no loop)
    let closeCalls = 0;
    dialog.close = vi.fn(() => {
      closeCalls++;
      dialog.open = false;
      dialog.removeAttribute('open');
      dialog.dispatchEvent(new Event('close'));
    });
    closeDialog();
    expect(closeCalls).toBe(1);
    expect(dialog.open).toBe(false);
    expect(dialog.querySelector('.dialog-content')).toBeNull();

    // 5. Unlocked cancel still closes via onCancel
    openDialog('Unlocked Cancel', () => <p class="probe">Cancelable</p>);
    expect(dialog.open).toBe(true);
    const cancelEv = new Event('cancel', { cancelable: true });
    dialog.dispatchEvent(cancelEv);
    expect(cancelEv.defaultPrevented).toBe(false);
    expect(dialog.open).toBe(false);
    expect(dialog.querySelector('.dialog-content')).toBeNull();

    if (origShowModal !== undefined) {
      dialog.showModal = origShowModal;
    }
  });

  it("Effect's own close() queueing a close event asynchronously does not close a second dialog opened in the same task", async () => {
    const el = mount(() => <Dialog />);
    const dialog = dialogEl(el);

    dialog.showModal = vi.fn(() => {
      dialog.open = true;
      dialog.setAttribute('open', '');
    });
    dialog.close = vi.fn(() => {
      dialog.open = false;
      dialog.removeAttribute('open');
      setTimeout(() => {
        dialog.dispatchEvent(new Event('close'));
      }, 0);
    });

    openDialog('first', () => <p class="probe">เนื้อหาแรก</p>);
    expect(dialog.open).toBe(true);

    // Chained closeDialog() + openDialog() in the same task
    closeDialog();
    openDialog('second', () => <p class="probe">เนื้อหาที่สอง</p>);
    expect(dialog.open).toBe(true);

    // Let the queued close event from the first dialog's close() fire
    await new Promise((r) => setTimeout(r, 0));

    expect(dialog.open).toBe(true);
    expect(dialog.querySelector('h2')?.textContent).toBe('second');
    expect(dialog.querySelector('.probe')?.textContent).toBe('เนื้อหาที่สอง');

    // A subsequent genuine native close on the second dialog still closes it
    dialog.open = false;
    dialog.removeAttribute('open');
    dialog.dispatchEvent(new Event('close'));
    expect(dialog.open).toBe(false);
    expect(dialog.querySelector('.dialog-content')).toBeNull();
  });
});
