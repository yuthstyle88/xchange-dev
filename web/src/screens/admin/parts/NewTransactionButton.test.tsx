// V2-parts (#476): the new-transaction button, LIVE (ticket §7 `NewTransactionButton.tsx`, design
// §12.2, behaviour P5). Written before the implementation by the lane controller (claude/7ca80c).
// Do not edit, delete or weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`, decoded admin page): the LIVE text `＋ ทำรายการแลกเงิน`
// (`admin-html:100`), disabled while a write is pending; on click (`:131`) with no open shift →
// toast `เปิดกะก่อนทำรายการแลกเงิน` and `location.hash = 'cash'`; with one → the dialog
// `ทำรายการแลกเงินจริง` with its sentence, `ปิด`, and the link `เปิดหน้าทำรายการ ↗` to `deskUrl` in a
// new tab. v3 reads the open shift and `deskUrl` from the view `admin_shift_state` on the button's
// branch when clicked (never `fetch`, done criterion 3).
//
// Uses the dialog host and `openDialog` of `Dialog.test.tsx` [question on the ticket], and
// `pendingWrite.run(api, branch, action, payload, onSaved)` of `pending-write.test.tsx`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import type { ApiClient, ViewAnswer, ViewParams } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { Dialog, closeDialog } from './Dialog';
import { NewTransactionButton } from './NewTransactionButton';
import { pendingWrite } from './pending-write';
import { Toast } from './toast';

const THAI = /[฀-๿]/;
const LABEL = '＋ ทำรายการแลกเงิน';
const OPEN_SHIFT_FIRST = 'เปิดกะก่อนทำรายการแลกเงิน';
const TITLE = 'ทำรายการแลกเงินจริง';
const SENTENCE =
  'เปิดหน้าทำรายการเดิมเพื่อยืนยันลูกค้า เอกสาร และการรับ–จ่ายเงิน เมื่อ เซิร์ฟเวอร์ ยืนยันการบันทึก สต็อกในกะนี้จะปรับตามรายการ';
const DESK_LINK = 'เปิดหน้าทำรายการ ↗';

interface Read {
  name: string;
  q: ViewParams;
}

function fakeApi(answer: ViewAnswer<{ shiftOpen: boolean; deskUrl: string }>) {
  const reads: Read[] = [];
  const posts: Array<{ path: string; resolve: (v: unknown) => void }> = [];
  const client = {
    view: async (name: string, q: ViewParams = {}) => {
      reads.push({ name, q });
      return answer;
    },
    page: async () => {
      throw new Error('the button reads no page');
    },
    post: (path: string) =>
      new Promise((resolve) => {
        posts.push({ path, resolve });
      }),
  };
  return { api: client as unknown as ApiClient, reads, posts };
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

let dispose: (() => void) | undefined;
let root: HTMLElement;
let fetchSpy: ReturnType<typeof vi.fn>;

function mount(api: ApiClient, branch = 'B1'): HTMLElement {
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <NewTransactionButton api={api} branch={branch} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  return root;
}

beforeEach(() => {
  fetchSpy = vi.fn(() => Promise.reject(new Error('the button never calls fetch')));
  vi.stubGlobal('fetch', fetchSpy);
  location.hash = '';
});

afterEach(async () => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
  await settle();
});

function theButton(el: HTMLElement): HTMLButtonElement {
  const found = [...el.querySelectorAll('button')].filter((b) => (b.textContent ?? '').trim() === LABEL);
  expect(found.length, 'one new-transaction button').toBe(1);
  return found[0] as HTMLButtonElement;
}

function dialog(el: HTMLElement): HTMLDialogElement {
  return el.querySelector('dialog') as HTMLDialogElement;
}

function toastText(el: HTMLElement): string | null {
  const status = el.querySelector('[role="status"]') as HTMLElement | null;
  if (!status || status.hidden || status.closest('[hidden]')) return null;
  const t = (status.textContent ?? '').trim();
  return t === '' ? null : t;
}

describe('NewTransactionButton (#476 P5)', () => {
  it('the LIVE text, enabled while nothing is pending; nothing is read before a click', () => {
    const { api, reads } = fakeApi({ version: 1, changed: true, data: { shiftOpen: true, deskUrl: '/desk?branch=B1' } });
    const el = mount(api);
    const button = theButton(el);
    expect(button.disabled).toBe(false);
    expect(reads).toEqual([]);
  });

  it('no open shift: toast เปิดกะก่อนทำรายการแลกเงิน and the cash menu; no dialog', async () => {
    const { api, reads } = fakeApi({ version: 1, changed: true, data: { shiftOpen: false, deskUrl: '/desk?branch=B1' } });
    const el = mount(api, 'B1');
    theButton(el).click();
    await settle();
    expect(reads.length).toBe(1);
    expect(reads[0]!.name).toBe('admin_shift_state');
    expect(reads[0]!.q.branch).toBe('B1');
    expect(toastText(el)).toBe(OPEN_SHIFT_FIRST);
    expect(location.hash).toBe('#cash');
    expect(dialog(el).open).toBe(false);
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("an open shift: the dialog with v2's sentence, ปิด, and the desk link from the view in a new tab", async () => {
    const { api, reads } = fakeApi({ version: 1, changed: true, data: { shiftOpen: true, deskUrl: '/desk?branch=B%202' } });
    const el = mount(api, 'B 2');
    theButton(el).click();
    await settle();
    expect(reads[0]!.q.branch).toBe('B 2');
    const d = dialog(el);
    expect(d.open).toBe(true);
    expect(d.textContent).toContain(TITLE);
    expect(d.textContent).toContain(SENTENCE);
    const links = [...d.querySelectorAll('a')].filter((a) => (a.textContent ?? '').trim() === DESK_LINK);
    expect(links.length).toBe(1);
    expect(links[0]!.getAttribute('href')).toBe('/desk?branch=B%202');
    expect(links[0]!.target).toBe('_blank');
    expect(links[0]!.rel).toContain('noopener');
    expect(location.hash).toBe('');
    const close = [...d.querySelectorAll('button')].filter((b) => (b.textContent ?? '').trim() === 'ปิด');
    expect(close.length).toBe(1);
    close[0]!.click();
    expect(d.open).toBe(false);
  });

  it('disabled while a write is running, enabled again after', async () => {
    const { api, posts } = fakeApi({ version: 1, changed: true, data: { shiftOpen: true, deskUrl: '/desk?branch=B1' } });
    const el = mount(api);
    void pendingWrite.run(api, 'B1', 'adminSaveRate', { code: 'USD' }, () => {});
    await settle();
    expect(pendingWrite.busy()).toBe(true);
    expect(theButton(el).disabled).toBe(true);
    posts[0]!.resolve({});
    await settle();
    expect(pendingWrite.busy()).toBe(false);
    expect(theButton(el).disabled).toBe(false);
  });

  it('in English the button, the toast and the dialog switch (W10)', async () => {
    setLang('en');
    const closed = fakeApi({ version: 1, changed: true, data: { shiftOpen: false, deskUrl: '/desk?branch=B1' } });
    const el = mount(closed.api);
    const button = el.querySelector('button') as HTMLButtonElement;
    expect(THAI.test(button.textContent ?? '')).toBe(false);
    button.click();
    await settle();
    expect(THAI.test(toastText(el) ?? '')).toBe(false);
    expect((toastText(el) ?? '').length).toBeGreaterThan(0);
    dispose?.();
    root.remove();
    const open = fakeApi({ version: 1, changed: true, data: { shiftOpen: true, deskUrl: '/desk?branch=B1' } });
    const el2 = mount(open.api);
    (el2.querySelector('button') as HTMLButtonElement).click();
    await settle();
    expect(dialog(el2).open).toBe(true);
    expect(THAI.test(dialog(el2).textContent ?? '')).toBe(false);
  });
});
