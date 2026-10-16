import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { ApiClient, ViewAnswer, ViewParams } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { Dialog, closeDialog } from './Dialog';
import { NewTransactionButton } from './NewTransactionButton';
import { pendingWrite } from './pending-write';
import { Toast } from './toast';

const LABEL = '＋ ทำรายการแลกเงิน';
const OPEN_SHIFT_FIRST = 'เปิดกะก่อนทำรายการแลกเงิน';
const DESK_LINK = 'เปิดหน้าทำรายการ ↗';

interface DeferredRead {
  name: string;
  q: ViewParams;
  resolve: (value: ViewAnswer<{ shiftOpen: boolean; deskUrl: string }>) => void;
  reject: (err: unknown) => void;
}

function createDeferredApi() {
  const reads: DeferredRead[] = [];
  const posts: Array<{ path: string; resolve: (v: unknown) => void }> = [];
  const client = {
    view: (name: string, q: ViewParams = {}) =>
      new Promise<ViewAnswer<{ shiftOpen: boolean; deskUrl: string }>>((resolve, reject) => {
        reads.push({ name, q, resolve, reject });
      }),
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

function mountWithBranchSignal(api: ApiClient, initialBranch = 'A') {
  const [branch, setBranch] = createSignal(initialBranch);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <NewTransactionButton api={api} branch={branch()} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  return { el: root, branch, setBranch };
}

beforeEach(() => {
  fetchSpy = vi.fn(() => Promise.reject(new Error('the button never calls fetch')));
  vi.stubGlobal('fetch', fetchSpy);
  location.hash = '';
  setLang('th');
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

describe('NewTransactionButton branch switch (#2442)', () => {
  it("row 1: Click in A, switch to B, A answers shiftOpen with A's deskUrl -> No dialog opens", async () => {
    const { api, reads } = createDeferredApi();
    const { el, setBranch } = mountWithBranchSignal(api, 'A');

    theButton(el).click();
    expect(reads).toHaveLength(1);
    expect(reads[0]!.q.branch).toBe('A');

    setBranch('B');
    await settle();

    reads[0]!.resolve({
      version: 1,
      changed: true,
      data: { shiftOpen: true, deskUrl: '/desk?branch=A' },
    });
    await settle();

    expect(dialog(el).open).toBe(false);
    expect(toastText(el)).toBeNull();
    expect(location.hash).toBe('');
  });

  it('row 2: Click in A, switch to B, A answers shiftOpen:false -> No toast and location.hash unchanged', async () => {
    const { api, reads } = createDeferredApi();
    const { el, setBranch } = mountWithBranchSignal(api, 'A');

    theButton(el).click();
    expect(reads).toHaveLength(1);
    expect(reads[0]!.q.branch).toBe('A');

    setBranch('B');
    await settle();

    reads[0]!.resolve({
      version: 1,
      changed: true,
      data: { shiftOpen: false, deskUrl: '/desk?branch=A' },
    });
    await settle();

    expect(toastText(el)).toBeNull();
    expect(location.hash).toBe('');
    expect(dialog(el).open).toBe(false);
  });

  it("row 3: Click in A, switch to B, A's read fails -> No error shown", async () => {
    const { api, reads } = createDeferredApi();
    const { el, setBranch } = mountWithBranchSignal(api, 'A');

    theButton(el).click();
    expect(reads).toHaveLength(1);
    expect(reads[0]!.q.branch).toBe('A');

    setBranch('B');
    await settle();

    reads[0]!.reject(new Error('network failure for A'));
    await settle();

    expect(el.querySelector('[role="alert"]')).toBeNull();
    expect(dialog(el).open).toBe(false);
    expect(location.hash).toBe('');
  });

  it("row 4: After the switch, click in B -> B's shift state is read and acted on", async () => {
    const { api, reads } = createDeferredApi();
    const { el, setBranch } = mountWithBranchSignal(api, 'A');

    // Click in A; read for A stays pending
    theButton(el).click();
    expect(reads).toHaveLength(1);
    expect(reads[0]!.q.branch).toBe('A');

    // Switch to B while A's read is still pending, then click in B
    setBranch('B');
    await settle();

    theButton(el).click();
    expect(reads).toHaveLength(2);
    expect(reads[1]!.q.branch).toBe('B');

    // B answers shiftOpen:true with B's deskUrl
    reads[1]!.resolve({
      version: 1,
      changed: true,
      data: { shiftOpen: true, deskUrl: '/desk?branch=B' },
    });
    await settle();

    const d = dialog(el);
    expect(d.open).toBe(true);
    const links = [...d.querySelectorAll('a')].filter((a) => (a.textContent ?? '').trim() === DESK_LINK);
    expect(links).toHaveLength(1);
    expect(links[0]!.getAttribute('href')).toBe('/desk?branch=B');

    // Stale read from A resolves later with shiftOpen:false — must not toast or jump to #cash
    reads[0]!.resolve({
      version: 1,
      changed: true,
      data: { shiftOpen: false, deskUrl: '/desk?branch=A' },
    });
    await settle();

    expect(location.hash).toBe('');
    expect(toastText(el)).toBeNull();
  });

  it('edge case 1 (§9) & row 5: Same branch unchanged behaviour (error, toast, dialog) after switching away from a pending read', async () => {
    const { api, reads } = createDeferredApi();
    const { el, setBranch } = mountWithBranchSignal(api, 'A');

    // Start a read in A, then switch to B and stay in B for multiple same-branch clicks
    theButton(el).click();
    expect(reads).toHaveLength(1);
    setBranch('B');
    await settle();

    // 1. Click in B -> error in B is shown when branch stays B
    theButton(el).click();
    expect(reads).toHaveLength(2);
    expect(reads[1]!.q.branch).toBe('B');
    reads[1]!.reject(new Error('temporary error'));
    await settle();
    expect(el.querySelector('[role="alert"]')).not.toBeNull();

    // 2. Click again in same branch B -> shiftOpen:false clears error, shows toast and sets #cash
    theButton(el).click();
    expect(reads).toHaveLength(3);
    expect(reads[2]!.q.branch).toBe('B');
    reads[2]!.resolve({
      version: 1,
      changed: true,
      data: { shiftOpen: false, deskUrl: '/desk?branch=B' },
    });
    await settle();
    expect(el.querySelector('[role="alert"]')).toBeNull();
    expect(toastText(el)).toBe(OPEN_SHIFT_FIRST);
    expect(location.hash).toBe('#cash');

    // 3. Click again in same branch B -> shiftOpen:true opens dialog with B's deskUrl
    location.hash = '';
    theButton(el).click();
    expect(reads).toHaveLength(4);
    expect(reads[3]!.q.branch).toBe('B');
    reads[3]!.resolve({
      version: 1,
      changed: true,
      data: { shiftOpen: true, deskUrl: '/desk?branch=B' },
    });
    await settle();
    expect(dialog(el).open).toBe(true);
  });

  it('edge case 2 (§9): pendingWrite busy still blocks the click after a branch switch, and unblocks once write finishes', async () => {
    const { api, reads, posts } = createDeferredApi();
    const { el, setBranch } = mountWithBranchSignal(api, 'A');

    // Click in A (read 0 pending), then switch to B (resets reading)
    theButton(el).click();
    expect(reads).toHaveLength(1);
    setBranch('B');
    await settle();

    // Start a pending write -> button must be disabled and block clicks even though reading was reset
    void pendingWrite.run(api, 'B', 'adminSaveRate', { code: 'USD' }, () => {});
    await settle();
    expect(pendingWrite.busy()).toBe(true);
    expect(theButton(el).disabled).toBe(true);

    theButton(el).click();
    expect(reads).toHaveLength(1);

    // Finish pendingWrite -> button is enabled in B and clicking reads B (even while A's read 0 is still pending)
    posts[0]!.resolve({});
    await settle();
    expect(pendingWrite.busy()).toBe(false);
    expect(theButton(el).disabled).toBe(false);

    theButton(el).click();
    expect(reads).toHaveLength(2);
    expect(reads[1]!.q.branch).toBe('B');
  });
});
