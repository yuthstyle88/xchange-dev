// V2-parts (#476): extra unit tests for unspecified behaviors
// (a) NewTransactionButton is disabled and shows the read error when its read fails
// (b) pendingWrite.inspect() calls requestRefresh() when it finds applied: true

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import { th } from '../../../shared/i18n/th';
import { Dialog, closeDialog } from './Dialog';
import { NewTransactionButton } from './NewTransactionButton';
import { pendingWrite } from './pending-write';
import { refreshTick } from './refresh';
import { Toast } from './toast';

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

let dispose: (() => void) | undefined;
let root: HTMLElement;

function mount(fn: () => JSX.Element): HTMLElement {
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          {fn()}
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  return root;
}

beforeEach(() => {
  location.hash = '';
});

afterEach(async () => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  location.hash = '';
  await settle();
});

describe('extra unit tests for unspecified behaviors (#476)', () => {
  it('(a) NewTransactionButton is enabled and displays read error when admin_shift_state read fails', async () => {
    const api = {
      view: vi
        .fn()
        .mockRejectedValue(new Error('Network error reading shift state')),
      page: vi.fn(),
      post: vi.fn(),
    } as unknown as ApiClient;

    const el = mount(() => <NewTransactionButton api={api} branch="B1" />);
    const button = el.querySelector('button') as HTMLButtonElement;
    expect(button.disabled).toBe(false);

    button.click();
    await settle();

    expect(button.disabled).toBe(false);
    expect(el.textContent).toContain(th['error.network']);
  });

  it('(b) pendingWrite.inspect() calls requestRefresh() when it finds applied: true', async () => {
    let inspectResolve: (v: unknown) => void;
    let postReject: (e: unknown) => void;

    const api = {
      post: vi.fn(
        () =>
          new Promise((_, reject) => {
            postReject = reject;
          }),
      ),
      view: vi.fn(
        () =>
          new Promise((resolve) => {
            inspectResolve = resolve;
          }),
      ),
      page: vi.fn(),
    } as unknown as ApiClient;

    const tickBefore = refreshTick();
    void pendingWrite.run(
      api,
      'B1',
      'adminSaveRate',
      { code: 'USD' },
      () => {},
    );
    await settle();

    // Trigger failure so it enters the not-confirmed pending state
    postReject!(new Error('Rate timed out'));
    await settle();

    expect(refreshTick()).toBe(tickBefore);

    // Call inspect
    void pendingWrite.inspect();
    await settle();

    // Resolve inspect with applied: true
    inspectResolve!({
      version: 5,
      changed: true,
      data: { applied: true },
    });
    await settle();

    expect(refreshTick()).toBe(tickBefore + 1);
  });

  it('(c) beforeunload guard is active while first POST is in-flight, and disabled after success', async () => {
    function leavingIsQuestioned(): boolean {
      const e = new Event('beforeunload', { cancelable: true });
      window.dispatchEvent(e);
      return e.defaultPrevented;
    }

    let postResolve: (v: unknown) => void;
    const api = {
      post: vi.fn(
        () =>
          new Promise((resolve) => {
            postResolve = resolve;
          }),
      ),
      view: vi.fn(),
      page: vi.fn(),
    } as unknown as ApiClient;

    expect(leavingIsQuestioned()).toBe(false);

    void pendingWrite.run(
      api,
      'B1',
      'adminSaveRate',
      { code: 'USD' },
      () => {},
    );
    await settle();

    // While initial POST is unresolved, beforeunload must be active
    expect(leavingIsQuestioned()).toBe(true);

    // Settle POST with success
    postResolve!({});
    await settle();

    // After success, beforeunload is disabled
    expect(leavingIsQuestioned()).toBe(false);
  });

  it('(d) pendingWrite.inspect() ignores calls while already in flight (busy guard)', async () => {
    let inspectResolve: (v: unknown) => void;
    let postReject: (e: unknown) => void;

    const api = {
      post: vi.fn(
        () =>
          new Promise((_, reject) => {
            postReject = reject;
          }),
      ),
      view: vi.fn(
        () =>
          new Promise((resolve) => {
            inspectResolve = resolve;
          }),
      ),
      page: vi.fn(),
    } as unknown as ApiClient;

    void pendingWrite.run(
      api,
      'B1',
      'adminSaveRate',
      { code: 'USD' },
      () => {},
    );
    await settle();

    // Fail initial write to establish pending request
    postReject!(new Error('Network error'));
    await settle();

    // First inspect
    void pendingWrite.inspect();
    await settle();
    expect(api.view).toHaveBeenCalledTimes(1);

    // Second inspect while first inspect is in flight -> must be ignored
    void pendingWrite.inspect();
    await settle();
    expect(api.view).toHaveBeenCalledTimes(1);

    // Resolve first inspect
    inspectResolve!({
      version: 1,
      changed: true,
      data: { applied: false },
    });
    await settle();

    expect(api.view).toHaveBeenCalledTimes(1);
  });
});
