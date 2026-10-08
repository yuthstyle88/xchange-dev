// V2-parts (#476): the page's one dialog (ticket §7 `Dialog.tsx`, design §12.2). Written before the
// implementation by the lane controller (claude/7ca80c). Do not edit, delete or weaken; add tests
// in a file of your own.
//
// v2 (`admin-html:22`, `:75-76`): one `<dialog>` per page with a heading and a close button `×`
// labelled `ปิดหน้าต่าง`; `modal(title, body)` replaces its title and body and opens it,
// `closeModal()` closes it.
//
// The ticket names the export `Dialog` only; these tests use the shape the shell and the menus
// need, as in v2 [question on the ticket]: `<Dialog/>` is the page's host, rendered once (as
// `<Toast/>` is); `openDialog(title, body)` shows `title` and the body `body()` in it, replacing
// whatever it showed; `closeDialog()` closes it. "Open" is read as the host's `<dialog>` element
// having `open` — jsdom has no `showModal()`, so the host sets the attribute (or calls
// `showModal()` only where it exists).
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import { setLang } from '../../../shared/i18n';
import { Dialog, closeDialog, openDialog } from './Dialog';

const THAI = /[฀-๿]/;

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

function closeButton(el: HTMLElement): HTMLButtonElement {
  const button = dialogEl(el).querySelector('button[aria-label]') as HTMLButtonElement | null;
  expect(button, 'the close button').not.toBeNull();
  return button!;
}

describe('Dialog (#476)', () => {
  it('the host is closed until something is opened in it', () => {
    const el = mount(() => <Dialog />);
    expect(dialogEl(el).open).toBe(false);
  });

  it('openDialog shows the title and the body; the close × is labelled ปิดหน้าต่าง', () => {
    const el = mount(() => <Dialog />);
    openDialog('รายละเอียดรายการ', () => <p class="probe">เนื้อหา</p>);
    const dialog = dialogEl(el);
    expect(dialog.open).toBe(true);
    expect(dialog.textContent).toContain('รายละเอียดรายการ');
    expect(dialog.querySelector('.probe')?.textContent).toBe('เนื้อหา');
    const close = closeButton(el);
    expect(close.getAttribute('aria-label')).toBe('ปิดหน้าต่าง');
    expect(close.textContent).toBe('×');
  });

  it('the close × closes it', () => {
    const el = mount(() => <Dialog />);
    openDialog('หัวข้อ', () => <p>body</p>);
    closeButton(el).click();
    expect(dialogEl(el).open).toBe(false);
  });

  it('one dialog per page: opening again replaces the title and the body', () => {
    const el = mount(() => <Dialog />);
    openDialog('แรก', () => <p class="probe">หนึ่ง</p>);
    openDialog('สอง', () => <p class="probe">สอง-เนื้อหา</p>);
    const dialog = dialogEl(el);
    expect(dialog.open).toBe(true);
    expect(dialog.textContent).not.toContain('แรก');
    expect(dialog.textContent).not.toContain('หนึ่ง');
    expect([...dialog.querySelectorAll('.probe')].map((p) => p.textContent)).toEqual(['สอง-เนื้อหา']);
  });

  it('closeDialog closes it', () => {
    const el = mount(() => <Dialog />);
    openDialog('หัวข้อ', () => <p>body</p>);
    closeDialog();
    expect(dialogEl(el).open).toBe(false);
  });

  it('in English the close label switches (W10)', () => {
    setLang('en');
    const el = mount(() => <Dialog />);
    openDialog('Title', () => <p>Body</p>);
    const label = closeButton(el).getAttribute('aria-label') ?? '';
    expect(label.length).toBeGreaterThan(0);
    expect(THAI.test(label)).toBe(false);
  });
});
