// Issue #2359: Admin NewTransactionButton — keep button enabled after a failed shift-state read,
// clear the error on the next click, and ignore clicks while a read is in flight.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import type { ApiClient, ViewAnswer, ViewParams } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { Dialog, closeDialog } from './Dialog';
import { NewTransactionButton } from './NewTransactionButton';
import { Toast } from './toast';

const LABEL = '＋ ทำรายการแลกเงิน';
const TITLE = 'ทำรายการแลกเงินจริง';

interface ReadCall {
  name: string;
  q: ViewParams;
  resolve: (v: ViewAnswer<{ shiftOpen: boolean; deskUrl: string }>) => void;
  reject: (err: unknown) => void;
}

function deferredApi() {
  const reads: ReadCall[] = [];
  const client = {
    view: (name: string, q: ViewParams = {}) =>
      new Promise<ViewAnswer<{ shiftOpen: boolean; deskUrl: string }>>((resolve, reject) => {
        reads.push({ name, q, resolve, reject });
      }),
    page: async () => {
      throw new Error('the button reads no page');
    },
    post: async () => ({}),
  };
  return { api: client as unknown as ApiClient, reads };
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

let dispose: (() => void) | undefined;
let root: HTMLElement;

function mount(api: ApiClient, initialBranch = 'B1'): { el: HTMLElement; setBranch: Setter<string> } {
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
  return { el: root, setBranch };
}

function theButton(el: HTMLElement): HTMLButtonElement {
  const found = [...el.querySelectorAll('button')].filter((b) => (b.textContent ?? '').trim() === LABEL);
  expect(found.length, 'one new-transaction button').toBe(1);
  return found[0] as HTMLButtonElement;
}

function dialog(el: HTMLElement): HTMLDialogElement {
  return el.querySelector('dialog') as HTMLDialogElement;
}

beforeEach(() => {
  vi.stubGlobal(
    'fetch',
    vi.fn(() => Promise.reject(new Error('the button never calls fetch'))),
  );
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

describe('NewTransactionButton recovery and in-flight guard (#2359)', () => {
  it('row 1: Read fails -> error shown, button enabled', async () => {
    const { api, reads } = deferredApi();
    const { el } = mount(api, 'B1');
    const button = theButton(el);

    button.click();
    expect(reads.length).toBe(1);
    reads[0]!.reject(new TypeError('Failed to fetch'));
    await settle();

    const alert = el.querySelector('.read-error[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert?.textContent).toBe(sharedTh['error.network']);
    expect(button.disabled).toBe(false);
  });

  it('row 2: Click again, read OK with an open shift -> error cleared, dialog opens (and branch change still clears error)', async () => {
    const { api, reads } = deferredApi();
    const { el, setBranch } = mount(api, 'B1');
    const button = theButton(el);

    // 1st click fails -> error shown, button stays enabled
    button.click();
    expect(reads.length).toBe(1);
    reads[0]!.reject(new TypeError('Failed to fetch'));
    await settle();

    expect(el.querySelector('.read-error[role="alert"]')?.textContent).toBe(sharedTh['error.network']);
    expect(button.disabled).toBe(false);

    // 2nd click clears the error immediately and starts a new read
    button.click();
    expect(reads.length).toBe(2);
    expect(el.querySelector('.read-error[role="alert"]')).toBeNull();

    reads[1]!.resolve({
      version: 1,
      changed: true,
      data: { shiftOpen: true, deskUrl: '/desk?branch=B1' },
    });
    await settle();

    expect(el.querySelector('.read-error[role="alert"]')).toBeNull();
    expect(dialog(el).open).toBe(true);
    expect(dialog(el).textContent).toContain(TITLE);
    closeDialog();

    // Edge case (§9): branch change still clears a read error
    button.click();
    expect(reads.length).toBe(3);
    reads[2]!.reject(new TypeError('Failed to fetch'));
    await settle();
    expect(el.querySelector('.read-error[role="alert"]')).not.toBeNull();

    setBranch('B2');
    await settle();
    expect(el.querySelector('.read-error[role="alert"]')).toBeNull();
  });

  it('row 3: Double click -> one read while in flight', async () => {
    const { api, reads } = deferredApi();
    const { el } = mount(api, 'B1');
    const button = theButton(el);

    button.click();
    button.click();
    expect(reads.length).toBe(1);

    reads[0]!.resolve({
      version: 1,
      changed: true,
      data: { shiftOpen: true, deskUrl: '/desk?branch=B1' },
    });
    await settle();

    expect(reads.length).toBe(1);
    expect(dialog(el).open).toBe(true);
  });
});
