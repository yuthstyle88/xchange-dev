// V2-parts (#476): the write with retry, inspect and abandon (ticket §7 `pending-write.ts`, §8, §9,
// design §12.2, behaviour P4). Written before the implementation by the lane controller
// (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//
// v2 (`xchange-v2-source@4d0bdbb`, decoded admin page):
// - `liveWrite` (`admin-html:118`): refused while a request is pending or one is running; adds
//   `branchId` and a new `requestId`; `retryWrite` (`:119-122`) posts `{action, payload}` to
//   `/api/rpc`, with the open dialog's inputs, selects, buttons and its close disabled while it
//   runs; success → close, toast `เซิร์ฟเวอร์ ยืนยันการบันทึกแล้ว` (the answer's snapshot is ignored in
//   v3: `requestRefresh()` and `onSaved()` instead); failure → the dialog `ยังไม่ได้รับการยืนยัน`
//   with the error, the note, `ลองคำขอเดิมซ้ำ` (the **same** request id) and `ตรวจสถานะล่าสุด`;
// - `inspectPending` (`:123`) — in v3 the view `admin_request_status` with the filter
//   `{requestId}` on the request's branch: applied → toast `ตรวจพบรายการบันทึกแล้ว`, the request is
//   done; else the dialog `ยังไม่พบคำขอนี้ใน เซิร์ฟเวอร์` with retry and `ยกเลิกคำขอที่ยังไม่บันทึก`;
//   a failed read → toast of its message;
// - `abandonPending` (`:142`): `adminAbandonRequest {requestId, branchId}` of the pending request;
//   toast `เซิร์ฟเวอร์ ยืนยันว่ารายการบันทึกแล้ว` when `requestResolution` is `APPLIED`, else
//   `ยกเลิกคำขอที่ยังไม่บันทึกแล้ว`; a failure → toast of its message, the request stays pending;
// - `:124`: while a request is pending the page asks before it is left (`beforeunload`) and the
//   dialog cannot be dismissed (`cancel` is prevented; its close stays disabled).
//
// The branch reaches the request as an explicit argument, never ambient state (lane controller,
// 2026-10-08): **`pendingWrite.run(api, branch, action, payload, onSaved)`** — the ticket's
// `run(api, action, payload, onSaved)` adds a `branchId` it is never given. `branchId` is that
// argument; the request keeps it, and `inspect()` / `abandon()` use the request's own branch (v2's
// `pendingWrite.payload.branchId`), whatever the caller passes to a later `run`. #460's shell and
// the menus call e.g. `pendingWrite.run(api, branch(), 'adminCreateBranch', {name}, onSaved)`.
// Also used beyond §7 [question on the ticket]: the dialog host and `openDialog` of
// `Dialog.test.tsx`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import { ApiError, type ApiClient, type ViewParams } from '../../../shared/api/client';
import { makeViewAnswer } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { Dialog, closeDialog, openDialog } from './Dialog';
import { pendingWrite } from './pending-write';
import { refreshTick } from './refresh';
import { Toast } from './toast';

const THAI = /[฀-๿]/;
const RPC = '/api/rpc';
const ID_1 = '11111111-2222-4333-8444-555555555555';
const ID_2 = '66666666-7777-4888-9999-aaaaaaaaaaaa';

const SAVED = 'เซิร์ฟเวอร์ ยืนยันการบันทึกแล้ว';
const NOT_CONFIRMED = 'ยังไม่ได้รับการยืนยัน';
const NOTE = 'ยังไม่แสดงว่าบันทึกสำเร็จ การลองซ้ำใช้รหัสเดิมเพื่อป้องกันบันทึกซ้ำ กรุณาอย่าปิดหรือโหลดหน้านี้ใหม่';
const RETRY = 'ลองคำขอเดิมซ้ำ';
const INSPECT = 'ตรวจสถานะล่าสุด';
const FOUND = 'ตรวจพบรายการบันทึกแล้ว';
const NOT_FOUND = 'ยังไม่พบคำขอนี้ใน เซิร์ฟเวอร์';
const NOT_FOUND_TEXT =
  'หากข้อความผิดพลาดระบุว่ากะหรือยอดเปลี่ยน ให้ยกเลิกคำขอนี้และตรวจข้อมูลล่าสุดก่อนทำใหม่ หากการเชื่อมต่อหมดเวลา ให้ลองคำขอเดิมซ้ำ';
const DISCARD = 'ยกเลิกคำขอที่ยังไม่บันทึก';
const APPLIED_TOAST = 'เซิร์ฟเวอร์ ยืนยันว่ารายการบันทึกแล้ว';
const CANCELLED_TOAST = 'ยกเลิกคำขอที่ยังไม่บันทึกแล้ว';
const RATE_REFUSED = 'เรทไม่ตรงกับเรทหน้าร้านล่าสุด กรุณาตรวจสอบเรทก่อนยืนยันรายการ';

// ── a fake ApiClient whose answers the test gives ────────────────────────────

interface Call {
  via: 'post' | 'view';
  path: string;
  body?: { action: string; payload: Record<string, unknown> };
  q?: ViewParams<{ requestId: string }>;
  resolve: (v: unknown) => void;
  reject: (e: unknown) => void;
}

class FakeApi {
  calls: Call[] = [];
  /** When set, every post answers this at once (used to clean up after a test). */
  autoAnswer: unknown = undefined;

  readonly client = {
    post: (path: string, body: { action: string; payload: Record<string, unknown> }) => {
      if (this.autoAnswer !== undefined) return Promise.resolve(this.autoAnswer);
      return new Promise((resolve, reject) => {
        this.calls.push({ via: 'post', path, body: structuredClone(body), resolve, reject });
      });
    },
    view: (name: string, q: ViewParams<{ requestId: string }> = {}) =>
      new Promise((resolve, reject) => {
        this.calls.push({ via: 'view', path: name, q: structuredClone(q), resolve, reject });
      }),
    page: () => Promise.reject(new Error('pendingWrite reads no page')),
  };

  get api(): ApiClient {
    return this.client as unknown as ApiClient;
  }

  posts(): Call[] {
    return this.calls.filter((c) => c.via === 'post');
  }

  views(): Call[] {
    return this.calls.filter((c) => c.via === 'view');
  }

  last(): Call {
    const c = this.calls[this.calls.length - 1];
    if (!c) throw new Error('no call yet');
    return c;
  }
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

let fake: FakeApi;
let dispose: (() => void) | undefined;
let root: HTMLElement;

beforeEach(() => {
  fake = new FakeApi();
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  vi.spyOn(crypto, 'randomUUID').mockReturnValueOnce(ID_1).mockReturnValueOnce(ID_2);
});

afterEach(async () => {
  // leave no request running or pending for the next test: answer what is still waiting, then
  // give up whatever is left
  for (const c of fake.calls) c.resolve({ requestResolution: 'CANCELLED' });
  await settle();
  fake.autoAnswer = { requestResolution: 'CANCELLED' };
  await pendingWrite.abandon();
  await settle();
  closeDialog();
  dispose?.();
  dispose = undefined;
  root.remove();
  vi.restoreAllMocks();
  setLang('th');
});

// ── reading the page ─────────────────────────────────────────────────────────

function dialog(): HTMLDialogElement {
  const found = root.querySelectorAll('dialog');
  expect(found.length).toBe(1);
  return found[0] as HTMLDialogElement;
}

function dialogText(): string {
  return dialog().open ? (dialog().textContent ?? '') : '';
}

function closeX(): HTMLButtonElement {
  return dialog().querySelector('button[aria-label]') as HTMLButtonElement;
}

function button(text: string): HTMLButtonElement {
  const found = [...dialog().querySelectorAll('button')].filter((b) => (b.textContent ?? '').trim() === text);
  expect(found.length, `one button ${text}`).toBe(1);
  return found[0] as HTMLButtonElement;
}

function toastText(): string | null {
  const status = root.querySelector('[role="status"]') as HTMLElement | null;
  if (!status || status.hidden || status.closest('[hidden]')) return null;
  const t = (status.textContent ?? '').trim();
  return t === '' ? null : t;
}

function leavingIsQuestioned(): boolean {
  const e = new Event('beforeunload', { cancelable: true });
  window.dispatchEvent(e);
  return e.defaultPrevented;
}

function cancelIsPrevented(): boolean {
  const e = new Event('cancel', { cancelable: true });
  dialog().dispatchEvent(e);
  return e.defaultPrevented;
}

/** A form in the dialog, as a menu opens one before it writes. */
function openForm(): void {
  openDialog('ตั้งเรท', () => (
    <form>
      <input name="buy" value="32.1" />
      <select name="code">
        <option>USD</option>
      </select>
      <button type="button">บันทึก</button>
    </form>
  ));
}

/** run → the post fails with the server's refusal → the failure dialog. */
async function failOnce(onSaved = vi.fn(), branch = 'B1'): Promise<void> {
  openForm();
  void pendingWrite.run(fake.api, branch, 'adminSaveRate', { code: 'USD', buy: '32.1' }, onSaved);
  await settle();
  fake.last().reject(new ApiError({ code: 'refused', message: RATE_REFUSED, status: 503 }));
  await settle();
}

describe('pendingWrite.run (#476 P4, §8)', () => {
  it('posts {action, payload + branchId + a new requestId} to /api/rpc through the ApiClient', async () => {
    void pendingWrite.run(fake.api, 'B1', 'adminSaveRate', { code: 'USD', buy: '32.1', sell: '33' }, vi.fn());
    await settle();
    const posts = fake.posts();
    expect(posts.length).toBe(1);
    expect(posts[0]!.path).toBe(RPC);
    expect(posts[0]!.body).toEqual({
      action: 'adminSaveRate',
      payload: { code: 'USD', buy: '32.1', sell: '33', branchId: 'B1', requestId: ID_1 },
    });
  });

  it('busy while the post runs; a run while busy is ignored (§9)', async () => {
    expect(pendingWrite.busy()).toBe(false);
    void pendingWrite.run(fake.api, 'B1', 'adminSaveRate', { code: 'USD' }, vi.fn());
    await settle();
    expect(pendingWrite.busy()).toBe(true);
    const ignored = vi.fn();
    void pendingWrite.run(fake.api, 'B1', 'adminOpenShift', { opening: {} }, ignored);
    await settle();
    expect(fake.posts().length).toBe(1);
    // the ignored write left no trace: the request that fails and is retried is the first one
    fake.last().reject(new ApiError({ code: 'refused', message: RATE_REFUSED, status: 503 }));
    await settle();
    expect(pendingWrite.busy()).toBe(false);
    button(RETRY).click();
    await settle();
    expect(fake.posts().length).toBe(2);
    expect(fake.posts()[1]!.body).toEqual(fake.posts()[0]!.body);
    expect(fake.posts()[1]!.body!.action).toBe('adminSaveRate');
    fake.last().resolve({});
    await settle();
    expect(ignored).not.toHaveBeenCalled();
  });

  it("while it runs, the open dialog's inputs, selects, buttons and its close are disabled", async () => {
    openForm();
    void pendingWrite.run(fake.api, 'B1', 'adminSaveRate', { code: 'USD' }, vi.fn());
    await settle();
    const controls = [...dialog().querySelectorAll('form input, form select, form button')] as Array<
      HTMLInputElement | HTMLSelectElement | HTMLButtonElement
    >;
    expect(controls.length).toBe(3);
    for (const c of controls) expect(c.disabled, c.tagName).toBe(true);
    expect(closeX().disabled).toBe(true);
    fake.last().resolve({});
    await settle();
  });

  it('success: toast, the dialog closes, refreshTick moves once, onSaved is called once', async () => {
    openForm();
    const onSaved = vi.fn();
    const tick = refreshTick();
    void pendingWrite.run(fake.api, 'B1', 'adminSaveRate', { code: 'USD' }, onSaved);
    await settle();
    expect(onSaved).not.toHaveBeenCalled();
    fake.last().resolve({ version: 1, rates: [{ code: 'USD' }] });
    await settle();
    expect(toastText()).toBe(SAVED);
    expect(dialog().open).toBe(false);
    expect(refreshTick()).toBe(tick + 1);
    expect(onSaved).toHaveBeenCalledTimes(1);
    expect(pendingWrite.busy()).toBe(false);
    expect(leavingIsQuestioned()).toBe(false);
  });

  it('two writes one after the other carry two different request ids', async () => {
    void pendingWrite.run(fake.api, 'B1', 'adminSaveRate', { code: 'USD' }, vi.fn());
    await settle();
    fake.last().resolve({});
    await settle();
    void pendingWrite.run(fake.api, 'B1', 'adminSaveRate', { code: 'EUR' }, vi.fn());
    await settle();
    const ids = fake.posts().map((p) => p.body!.payload.requestId);
    expect(ids).toEqual([ID_1, ID_2]);
    fake.last().resolve({});
    await settle();
  });

  it('branchId is the branch argument of run', async () => {
    void pendingWrite.run(fake.api, 'B7', 'adminCreateBranch', { name: 'Airport' }, vi.fn());
    await settle();
    expect(fake.last().body!.payload).toEqual({ name: 'Airport', branchId: 'B7', requestId: ID_1 });
    fake.last().resolve({});
    await settle();
  });
});

describe('pendingWrite — a failed write (#476 P4, §9)', () => {
  it('the dialog ยังไม่ได้รับการยืนยัน with the error, the note, retry and inspect; nothing saved', async () => {
    const onSaved = vi.fn();
    const tick = refreshTick();
    await failOnce(onSaved);
    const text = dialogText();
    expect(text).toContain(NOT_CONFIRMED);
    expect(text).toContain('เซิร์ฟเวอร์ขัดข้อง'); // #2156: a 5xx is the unknown-outcome case, worded error.serverError
    expect(text).toContain(NOTE);
    button(RETRY);
    button(INSPECT);
    expect(onSaved).not.toHaveBeenCalled();
    expect(refreshTick()).toBe(tick);
    expect(pendingWrite.busy()).toBe(false);
  });

  it('while pending: the close stays disabled, cancel is prevented, leaving the page is questioned (§9)', async () => {
    expect(leavingIsQuestioned()).toBe(false);
    await failOnce();
    expect(closeX().disabled).toBe(true);
    expect(cancelIsPrevented()).toBe(true);
    expect(leavingIsQuestioned()).toBe(true);
  });

  it('a new run while a failed request is pending is ignored', async () => {
    await failOnce();
    void pendingWrite.run(fake.api, 'B1', 'adminOpenShift', { opening: {} }, vi.fn());
    await settle();
    expect(fake.posts().length).toBe(1);
  });

  it('retry sends the same request again, with the same request id (§9)', async () => {
    const onSaved = vi.fn();
    await failOnce(onSaved);
    button(RETRY).click();
    await settle();
    const posts = fake.posts();
    expect(posts.length).toBe(2);
    expect(posts[1]!.path).toBe(RPC);
    expect(posts[1]!.body).toEqual(posts[0]!.body);
    expect(posts[1]!.body!.payload.requestId).toBe(ID_1);
    expect(pendingWrite.busy()).toBe(true);
    fake.last().resolve({});
    await settle();
    expect(onSaved).toHaveBeenCalledTimes(1);
    expect(toastText()).toBe(SAVED);
    expect(dialog().open).toBe(false);
    expect(leavingIsQuestioned()).toBe(false);
    expect(cancelIsPrevented()).toBe(false);
  });
});

describe('pendingWrite.inspect (#476 P4, admin-html:123)', () => {
  it('reads admin_request_status for the request on its branch; applied → toast, done', async () => {
    await failOnce(vi.fn(), 'B7');
    // a later run on another branch is ignored and changes nothing about the pending request
    void pendingWrite.run(fake.api, 'B2', 'adminSaveRate', { code: 'EUR' }, vi.fn());
    await settle();
    button(INSPECT).click();
    await settle();
    const views = fake.views();
    expect(views.length).toBe(1);
    expect(views[0]!.path).toBe('admin_request_status');
    expect(views[0]!.q!.branch).toBe('B7');
    expect(views[0]!.q!.filter).toEqual({ requestId: ID_1 });
    views[0]!.resolve(makeViewAnswer({ applied: true }, { version: 3, changed: true }));
    await settle();
    expect(toastText()).toBe(FOUND);
    expect(dialog().open).toBe(false);
    expect(leavingIsQuestioned()).toBe(false);
    void pendingWrite.run(fake.api, 'B1', 'adminSaveRate', { code: 'USD' }, vi.fn());
    await settle();
    expect(fake.posts().length, 'a new write is accepted again').toBe(2);
    fake.last().resolve({});
    await settle();
  });

  it('not applied: the dialog ยังไม่พบคำขอนี้ใน เซิร์ฟเวอร์ with retry and discard; still pending', async () => {
    await failOnce();
    void pendingWrite.inspect();
    await settle();
    fake.views()[0]!.resolve({ version: 3, changed: true, data: { applied: false } });
    await settle();
    const text = dialogText();
    expect(text).toContain(NOT_FOUND);
    expect(text).toContain(NOT_FOUND_TEXT);
    button(RETRY);
    button(DISCARD);
    expect(leavingIsQuestioned()).toBe(true);
    expect(closeX().disabled).toBe(true);
  });

  it('a failed read: toast of its message; still pending', async () => {
    await failOnce();
    button(INSPECT).click();
    await settle();
    fake.views()[0]!.reject(new ApiError({ code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง', status: 500 }));
    await settle();
    expect(toastText()).toBe('เซิร์ฟเวอร์ขัดข้อง');
    expect(leavingIsQuestioned()).toBe(true);
  });
});

describe('pendingWrite.abandon (#476 P4, admin-html:142)', () => {
  async function toNotFound(branch = 'B1'): Promise<void> {
    await failOnce(vi.fn(), branch);
    button(INSPECT).click();
    await settle();
    fake.views()[0]!.resolve({ version: 3, changed: true, data: { applied: false } });
    await settle();
  }

  it('posts adminAbandonRequest {requestId, branchId} of the pending request; CANCELLED → its toast', async () => {
    await toNotFound('B7');
    button(DISCARD).click();
    await settle();
    const last = fake.posts()[fake.posts().length - 1]!;
    expect(last.path).toBe(RPC);
    expect(last.body).toEqual({ action: 'adminAbandonRequest', payload: { requestId: ID_1, branchId: 'B7' } });
    last.resolve({ requestResolution: 'CANCELLED', version: 1 });
    await settle();
    expect(toastText()).toBe(CANCELLED_TOAST);
    expect(dialog().open).toBe(false);
    expect(leavingIsQuestioned()).toBe(false);
    expect(cancelIsPrevented()).toBe(false);
  });

  it('APPLIED → เซิร์ฟเวอร์ ยืนยันว่ารายการบันทึกแล้ว', async () => {
    await failOnce();
    void pendingWrite.abandon();
    await settle();
    const last = fake.posts()[fake.posts().length - 1]!;
    expect(last.body!.action).toBe('adminAbandonRequest');
    last.resolve({ requestResolution: 'APPLIED' });
    await settle();
    expect(toastText()).toBe(APPLIED_TOAST);
    expect(leavingIsQuestioned()).toBe(false);
  });

  it('a failed abandon: toast of its message; the request stays pending', async () => {
    await failOnce();
    void pendingWrite.abandon();
    await settle();
    fake.posts()[fake.posts().length - 1]!.reject(new ApiError({ code: 'refused', message: 'ไม่พบสาขาที่เลือก กรุณารีเฟรช', status: 400 }));
    await settle();
    expect(toastText()).toBe('ไม่พบสาขาที่เลือก กรุณารีเฟรช');
    expect(leavingIsQuestioned()).toBe(true);
  });

  it('nothing pending: abandon sends nothing', async () => {
    await pendingWrite.abandon();
    await settle();
    expect(fake.posts()).toEqual([]);
  });
});

describe('pendingWrite — English (#476, W10)', () => {
  it("the failure dialog's texts switch", async () => {
    setLang('en');
    openForm();
    void pendingWrite.run(fake.api, 'B1', 'adminSaveRate', { code: 'USD' }, vi.fn());
    await settle();
    fake.last().reject(new ApiError({ code: 'refused', message: 'Rate changed', status: 503 }));
    await settle();
    const text = dialogText();
    expect(text).toContain('Server error'); // #2156: a 5xx is worded error.serverError (en)
    expect(THAI.test(text), text).toBe(false);
    expect(THAI.test(closeX().getAttribute('aria-label') ?? '')).toBe(false);
  });
});
