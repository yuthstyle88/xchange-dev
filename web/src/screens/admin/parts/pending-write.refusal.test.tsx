import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { closeDialog, Dialog, isDialogLocked, openDialog } from './Dialog';
import { th as partsTh } from './i18n';
import { pendingWrite } from './pending-write';
import { refreshTick } from './refresh';
import { Toast } from './toast';

interface RpcCall {
  path: string;
  body: { action: string; payload: Record<string, unknown> };
}

type Reply =
  | 'network'
  | { status: number; body: unknown };

async function settle(): Promise<void> {
  for (let round = 0; round < 3; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

function leavingIsQuestioned(): boolean {
  const e = new Event('beforeunload', { cancelable: true });
  window.dispatchEvent(e);
  return e.defaultPrevented;
}

describe('pendingWrite definite 4xx refusals vs unknown-outcome failures (#2156)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let calls: RpcCall[];
  let queue: Reply[];
  let api: ApiClient;

  function openForm(title = 'เพิ่มสาขา'): void {
    openDialog(title, () => (
      <form>
        <input name="name" value="XC-NEW" />
        <button type="submit">บันทึก</button>
      </form>
    ));
  }

  function dialogEl(): HTMLDialogElement {
    const el = root.querySelector<HTMLDialogElement>('dialog');
    expect(el).not.toBeNull();
    return el!;
  }

  function closeBtn(): HTMLButtonElement {
    const btn = dialogEl().querySelector<HTMLButtonElement>('header button[aria-label]');
    expect(btn).not.toBeNull();
    return btn!;
  }

  function buttonWithText(label: string): HTMLButtonElement | undefined {
    return [...dialogEl().querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => (b.textContent ?? '').trim() === label,
    );
  }

  function toastText(): string {
    const el = root.querySelector<HTMLElement>('[role="status"]');
    if (!el || el.hidden || el.closest('[hidden]')) return '';
    return (el.textContent ?? '').trim();
  }

  beforeEach(() => {
    setLang('th');
    calls = [];
    queue = [];
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const url = new URL(String(input), 'http://localhost');
      const rawBody = init?.body ? JSON.parse(String(init.body)) : {};
      if (url.pathname === '/api/rpc' && rawBody.action === 'adminAbandonRequest') {
        return new Response(JSON.stringify({ requestResolution: 'CANCELLED' }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }
      calls.push({ path: url.pathname, body: rawBody });
      const next = queue.shift() ?? { status: 200, body: {} };
      if (next === 'network') {
        throw new TypeError('Failed to fetch');
      }
      return new Response(JSON.stringify(next.body), {
        status: next.status,
        headers: { 'content-type': 'application/json' },
      });
    });
    vi.stubGlobal('fetch', fetchMock);
    api = new ApiClient(fetchMock as unknown as typeof fetch);

    root = document.createElement('div');
    document.body.appendChild(root);
    dispose = render(
      () => (
        <>
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );
  });

  afterEach(async () => {
    await pendingWrite.abandon();
    closeDialog();
    dispose?.();
    dispose = undefined;
    root.remove();
    vi.unstubAllGlobals();
    setLang('th');
    await settle();
  });

  it('row 1: 400 validationFailed / 403 roleNotAllowedHere / any 4xx refusal shows worded refusal, unlocks dialog, leaves no pending request, disables beforeunload, and offers no Retry/Inspect', async () => {
    openForm();
    queue.push(
      {
        status: 400,
        body: { error: { code: 'validationFailed', message: 'ชื่อสาขาซ้ำในระบบ' } },
      },
      {
        status: 403,
        body: { error: { code: 'roleNotAllowedHere', message: 'raw role error' } },
      },
      {
        status: 409,
        body: { error: { code: 'conflictRefusal', message: 'สถานะเปลี่ยนไปแล้ว' } },
      },
    );

    // 1st attempt: 400 validationFailed
    await pendingWrite.run(api, 'B1', 'adminCreateBranch', { name: 'XC-1' }, vi.fn());
    await settle();

    expect(dialogEl().querySelector('.error')?.textContent).toBe('ชื่อสาขาซ้ำในระบบ');
    expect(isDialogLocked()).toBe(false);
    expect(closeBtn().disabled).toBe(false);
    expect(leavingIsQuestioned()).toBe(false);
    expect(buttonWithText(partsTh.retry)).toBeUndefined();
    expect(buttonWithText(partsTh.inspect)).toBeUndefined();
    expect(dialogEl().textContent).not.toContain(partsTh.notConfirmed);

    // Because no request is pending, a fresh run is sent with a new requestId
    await pendingWrite.run(api, 'B1', 'adminCreateBranch', { name: 'XC-2' }, vi.fn());
    await settle();

    expect(calls.length).toBe(2);
    expect(calls[1]!.body.payload.requestId).not.toBe(calls[0]!.body.payload.requestId);
    expect(dialogEl().querySelector('.error')?.textContent).toBe(sharedTh['error.roleNotAllowedHere']);
    expect(dialogEl().querySelector('.error')?.textContent).not.toContain('raw role error');
    expect(isDialogLocked()).toBe(false);
    expect(closeBtn().disabled).toBe(false);
    expect(leavingIsQuestioned()).toBe(false);
    expect(buttonWithText(partsTh.retry)).toBeUndefined();
    expect(buttonWithText(partsTh.inspect)).toBeUndefined();

    // 3rd attempt: 409 refusal
    await pendingWrite.run(api, 'B1', 'adminCreateBranch', { name: 'XC-3' }, vi.fn());
    await settle();

    expect(calls.length).toBe(3);
    expect(dialogEl().querySelector('.error')?.textContent).toBe('สถานะเปลี่ยนไปแล้ว');
    expect(isDialogLocked()).toBe(false);
    expect(leavingIsQuestioned()).toBe(false);
    expect(buttonWithText(partsTh.retry)).toBeUndefined();
    expect(buttonWithText(partsTh.inspect)).toBeUndefined();
  });

  it('row 2: network error (fetch rejects) opens the locked "not confirmed" dialog with Retry/Inspect and beforeunload active', async () => {
    openForm();
    queue.push('network');

    await pendingWrite.run(api, 'B1', 'adminCreateBranch', { name: 'XC-NET' }, vi.fn());
    await settle();

    expect(dialogEl().textContent).toContain(partsTh.notConfirmed);
    expect(dialogEl().querySelector('.error')?.textContent).toBe(sharedTh['error.network']);
    expect(isDialogLocked()).toBe(true);
    expect(closeBtn().disabled).toBe(true);
    expect(leavingIsQuestioned()).toBe(true);
    expect(buttonWithText(partsTh.retry)).toBeDefined();
    expect(buttonWithText(partsTh.inspect)).toBeDefined();

    // Request remains pending: a fresh run is ignored
    await pendingWrite.run(api, 'B1', 'adminCreateBranch', { name: 'IGNORED' }, vi.fn());
    expect(calls.length).toBe(1);
  });

  it('row 3: 5xx / serverError opens the locked "not confirmed" dialog with Retry/Inspect and beforeunload active', async () => {
    openForm();
    queue.push({
      status: 503,
      body: { error: { code: 'serverError', message: 'upstream unavailable' } },
    });

    await pendingWrite.run(api, 'B1', 'adminCreateBranch', { name: 'XC-503' }, vi.fn());
    await settle();

    expect(dialogEl().textContent).toContain(partsTh.notConfirmed);
    expect(dialogEl().querySelector('.error')?.textContent).toBe(sharedTh['error.serverError']);
    expect(isDialogLocked()).toBe(true);
    expect(closeBtn().disabled).toBe(true);
    expect(leavingIsQuestioned()).toBe(true);
    expect(buttonWithText(partsTh.retry)).toBeDefined();
    expect(buttonWithText(partsTh.inspect)).toBeDefined();
  });

  it('row 4: 408 Request Timeout opens the locked "not confirmed" dialog (outcome is unknown)', async () => {
    openForm();
    queue.push({
      status: 408,
      body: { error: { code: 'requestTimeout', message: 'คำขอใช้เวลานานเกินกำหนด' } },
    });

    await pendingWrite.run(api, 'B1', 'adminCreateBranch', { name: 'XC-408' }, vi.fn());
    await settle();

    expect(dialogEl().textContent).toContain(partsTh.notConfirmed);
    expect(dialogEl().querySelector('.error')?.textContent).toBe('คำขอใช้เวลานานเกินกำหนด');
    expect(isDialogLocked()).toBe(true);
    expect(closeBtn().disabled).toBe(true);
    expect(leavingIsQuestioned()).toBe(true);
    expect(buttonWithText(partsTh.retry)).toBeDefined();
    expect(buttonWithText(partsTh.inspect)).toBeDefined();
  });

  it('row 5: retry() answered with a 4xx releases the pending request, unlocks the dialog, disables beforeunload, removes Retry/Inspect, and shows the refusal', async () => {
    openForm();
    queue.push(
      'network',
      {
        status: 403,
        body: { error: { code: 'roleNotAllowedHere', message: 'Forbidden on retry' } },
      },
      { status: 200, body: {} },
    );

    await pendingWrite.run(api, 'B1', 'adminCreateBranch', { name: 'XC-RETRY' }, vi.fn());
    await settle();

    expect(isDialogLocked()).toBe(true);
    expect(leavingIsQuestioned()).toBe(true);
    const retryBtn = buttonWithText(partsTh.retry);
    expect(retryBtn).toBeDefined();

    // Retry fails with 403 roleNotAllowedHere
    retryBtn!.click();
    await settle();

    expect(calls.length).toBe(2);
    expect(calls[1]!.body.payload.requestId).toBe(calls[0]!.body.payload.requestId);
    expect(dialogEl().querySelector('.error')?.textContent).toBe(sharedTh['error.roleNotAllowedHere']);
    expect(dialogEl().querySelector('.error')?.textContent).not.toContain('Forbidden on retry');
    expect(isDialogLocked()).toBe(false);
    expect(closeBtn().disabled).toBe(false);
    expect(leavingIsQuestioned()).toBe(false);
    expect(buttonWithText(partsTh.retry)).toBeUndefined();
    expect(buttonWithText(partsTh.inspect)).toBeUndefined();
    expect(dialogEl().textContent).not.toContain(partsTh.notConfirmed);

    // Released: a new run is now accepted with a fresh requestId
    await pendingWrite.run(api, 'B1', 'adminCreateBranch', { name: 'XC-FRESH' }, vi.fn());
    await settle();
    expect(calls.length).toBe(3);
    expect(calls[2]!.body.payload.requestId).not.toBe(calls[0]!.body.payload.requestId);
  });

  it('row 6: success shows toast, closes dialog, increments refreshTick, and calls onSaved', async () => {
    openForm();
    queue.push({ status: 200, body: { ok: true } });
    const onSaved = vi.fn();
    const beforeTick = refreshTick();

    await pendingWrite.run(api, 'B1', 'adminCreateBranch', { name: 'XC-OK' }, onSaved);
    await settle();

    expect(toastText()).toBe(partsTh.serverSaved);
    expect(dialogEl().open).toBe(false);
    expect(isDialogLocked()).toBe(false);
    expect(leavingIsQuestioned()).toBe(false);
    expect(refreshTick()).toBe(beforeTick + 1);
    expect(onSaved).toHaveBeenCalledTimes(1);
  });
});
