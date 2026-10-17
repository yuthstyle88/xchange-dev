import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import type { AdminMenuProps } from '../menus';
import { Members } from './Members';

beforeAll(
  () =>
    warmModules(
      () => import('./Members'),
      () => import('./i18n'),
      () => import('../../../shared/api/client'),
      () => import('../../../shared/i18n'),
      () => import('../../../shared/i18n/worded-error'),
    ),
  60_000,
);

interface CapturedWrite {
  method: string;
  pathname: string;
  idempotencyKey: string;
  rawBody: string;
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T | PromiseLike<T>) => void;
  reject: (reason?: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function makeProps(): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-14', to: '2026-10-14' });
  const [shown] = createSignal(true);
  return { branch, period, shown };
}

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    await new Promise((r) => realSetTimeout(r, 0));
  }
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('Admin Members in-flight double-submit guards (#2310)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let defineCalls: CapturedWrite[];
  let snapshotCalls: CapturedWrite[];
  let defineResponder: (callIndex: number, req: CapturedWrite) => Promise<Response>;
  let snapshotResponder: (callIndex: number, req: CapturedWrite) => Promise<Response>;

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);
    defineCalls = [];
    snapshotCalls = [];

    defineResponder = async (idx) =>
      jsonResponse(200, { id: `seg-${idx + 1}`, version: 1 });
    snapshotResponder = async (idx) =>
      jsonResponse(200, { id: `snap-${idx + 1}`, memberCount: 25 });

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl =
        typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(rawUrl, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers = new Headers(init?.headers);
      const idempotencyKey = headers.get('idempotency-key') ?? '';
      const rawBody = typeof init?.body === 'string' ? init.body : '';

      if (method === 'POST' && u.pathname === '/api/segments') {
        const entry: CapturedWrite = {
          method,
          pathname: u.pathname,
          idempotencyKey,
          rawBody,
        };
        const idx = defineCalls.length;
        defineCalls.push(entry);
        return defineResponder(idx, entry);
      }

      if (method === 'POST' && /^\/api\/segments\/[^/]+\/snapshots$/.test(u.pathname)) {
        const entry: CapturedWrite = {
          method,
          pathname: u.pathname,
          idempotencyKey,
          rawBody,
        };
        const idx = snapshotCalls.length;
        snapshotCalls.push(entry);
        return snapshotResponder(idx, entry);
      }

      if (method === 'GET' && u.pathname === '/api/v3/views/segment_preview') {
        return jsonResponse(200, {
          version: 1,
          changed: true,
          data: {
            snapshotId: u.searchParams.get('id') ?? 'snap-1',
            memberCount: 25,
            distribution: {
              avgRecencyDays: 10,
              avgFrequency: 2,
              totalValueThb: '10000.00',
            },
            sample: [],
          },
        });
      }

      return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
    });
  });

  afterEach(async () => {
    await settle();
    dispose?.();
    dispose = undefined;
    root.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  function mountMembers() {
    dispose = render(() => <Members {...makeProps()} />, root);
  }

  function setDefineInputs(nameVal: string, recencyVal = '30') {
    const nameInput = root.querySelector('#segment-name') as HTMLInputElement;
    expect(nameInput).not.toBeNull();
    nameInput.value = nameVal;
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));

    const recencyInput = root.querySelector('#segment-recency-days') as HTMLInputElement;
    expect(recencyInput).not.toBeNull();
    recencyInput.value = recencyVal;
    recencyInput.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function defineButton(): HTMLButtonElement {
    const btn = root.querySelector('button[data-action="define"]') as HTMLButtonElement;
    expect(btn).not.toBeNull();
    return btn;
  }

  function snapshotButton(): HTMLButtonElement {
    const btn = root.querySelector('button[data-action="snapshot"]') as HTMLButtonElement;
    expect(btn).not.toBeNull();
    return btn;
  }

  it('row 1: Two clicks on Define while the first is in flight -> One request, no error', async () => {
    const firstCall = deferred<Response>();
    defineResponder = (idx) => {
      if (idx === 0) return firstCall.promise;
      return Promise.resolve(
        jsonResponse(409, {
          error: { code: 'idempotencyConflict', message: 'Idempotency key already in flight' },
        }),
      );
    };

    mountMembers();
    setDefineInputs('VIP Gold', '30');

    const btn = defineButton();
    expect(btn.disabled).toBe(false);

    // Two clicks while the first request is in flight
    btn.click();
    btn.click();
    expect(defineCalls).toHaveLength(1);
    expect(btn.disabled).toBe(true);

    firstCall.resolve(jsonResponse(200, { id: 'seg-1', version: 1 }));
    await settle();

    expect(defineCalls).toHaveLength(1);
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(btn.disabled).toBe(false);
  });

  it('row 2: Two clicks on Snapshot while in flight -> One request, no error', async () => {
    const firstSnapshot = deferred<Response>();
    snapshotResponder = (idx) => {
      if (idx === 0) return firstSnapshot.promise;
      return Promise.resolve(
        jsonResponse(409, {
          error: { code: 'idempotencyConflict', message: 'Snapshot key already in flight' },
        }),
      );
    };

    mountMembers();
    setDefineInputs('VIP Gold', '30');
    defineButton().click();
    await settle();
    expect(defineCalls).toHaveLength(1);

    const snapBtn = snapshotButton();
    expect(snapBtn.disabled).toBe(false);

    // Two clicks while the first Snapshot request is in flight
    snapBtn.click();
    snapBtn.click();
    expect(snapshotCalls).toHaveLength(1);
    expect(snapBtn.disabled).toBe(true);

    // Even if disabled attribute were bypassed in DOM, handler guard blocks in-flight call
    snapBtn.disabled = false;
    snapBtn.click();
    expect(snapshotCalls).toHaveLength(1);

    firstSnapshot.resolve(jsonResponse(200, { id: 'snap-1', memberCount: 25 }));
    await settle();

    expect(snapshotCalls).toHaveLength(1);
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(snapBtn.disabled).toBe(false);
  });

  it('row 3: Enter pressed in the form while defining -> No second request', async () => {
    const firstCall = deferred<Response>();
    defineResponder = (idx) => {
      if (idx === 0) return firstCall.promise;
      return Promise.resolve(
        jsonResponse(409, {
          error: { code: 'idempotencyConflict', message: 'Idempotency key already in flight' },
        }),
      );
    };

    mountMembers();
    setDefineInputs('VIP Gold', '30');

    const form = root.querySelector('form') as HTMLFormElement;
    expect(form).not.toBeNull();

    // First submit starts defining
    defineButton().click();
    expect(defineCalls).toHaveLength(1);

    // Pressing Enter in an input dispatches submit on the form while defining is true
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    expect(defineCalls).toHaveLength(1);

    firstCall.resolve(jsonResponse(200, { id: 'seg-1', version: 1 }));
    await settle();

    expect(defineCalls).toHaveLength(1);
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it('row 4: Submit, 2xx, then submit the same input again -> New key, second request (kept, Members.keyed.test.tsx:247)', async () => {
    const firstCall = deferred<Response>();
    defineResponder = (idx) => {
      if (idx === 0) return firstCall.promise;
      return Promise.resolve(jsonResponse(200, { id: `seg-${idx + 1}`, version: idx + 1 }));
    };

    mountMembers();
    setDefineInputs('VIP Gold', '30');

    const btn = defineButton();
    btn.click();
    expect(defineCalls).toHaveLength(1);
    expect(btn.disabled).toBe(true);

    // An extra click while in flight is ignored
    btn.click();
    expect(defineCalls).toHaveLength(1);

    // First submit completes with 2xx
    firstCall.resolve(jsonResponse(200, { id: 'seg-1', version: 1 }));
    await settle();
    expect(btn.disabled).toBe(false);

    // Submitting the same input again after 2xx sends a second request with a new key
    btn.click();
    await settle();

    expect(defineCalls).toHaveLength(2);
    expect(defineCalls[0]!.idempotencyKey).toBeTruthy();
    expect(defineCalls[1]!.idempotencyKey).toBeTruthy();
    expect(defineCalls[1]!.idempotencyKey).not.toBe(defineCalls[0]!.idempotencyKey);
    expect(defineCalls[1]!.rawBody).toBe(defineCalls[0]!.rawBody);
  });

  it('row 5: Network failure, then submit again -> Same key (kept, Members.keyed.test.tsx:164)', async () => {
    const firstCall = deferred<Response>();
    defineResponder = (idx) => {
      if (idx === 0) return firstCall.promise;
      return Promise.resolve(jsonResponse(200, { id: 'seg-1', version: 1 }));
    };

    mountMembers();
    setDefineInputs('VIP Gold', '30');

    const btn = defineButton();
    btn.click();
    expect(defineCalls).toHaveLength(1);
    expect(btn.disabled).toBe(true);

    // First request fails with a network error
    firstCall.reject(new TypeError('Failed to fetch'));
    await settle();
    expect(btn.disabled).toBe(false);

    // Submitting again reuses the same key
    btn.click();
    await settle();

    expect(defineCalls).toHaveLength(2);
    expect(defineCalls[0]!.idempotencyKey).toBeTruthy();
    expect(defineCalls[1]!.idempotencyKey).toBe(defineCalls[0]!.idempotencyKey);
    expect(defineCalls[1]!.rawBody).toBe(defineCalls[0]!.rawBody);
  });

  it('edge case (§9): The busy flag clears in finally, including on a network error', async () => {
    const defineCall1 = deferred<Response>();
    const snapshotCall1 = deferred<Response>();

    defineResponder = (idx) => {
      if (idx === 0) return defineCall1.promise;
      return Promise.resolve(jsonResponse(200, { id: 'seg-1', version: 1 }));
    };
    snapshotResponder = (idx) => {
      if (idx === 0) return snapshotCall1.promise;
      return Promise.resolve(jsonResponse(200, { id: 'snap-1', memberCount: 25 }));
    };

    mountMembers();
    setDefineInputs('VIP Gold', '30');

    const defBtn = defineButton();
    defBtn.click();
    expect(defBtn.disabled).toBe(true);

    // Network error on Define clears defining() in finally
    defineCall1.reject(new TypeError('Failed to fetch'));
    await settle();
    expect(defBtn.disabled).toBe(false);

    // Retry Define succeeds
    defBtn.click();
    await settle();
    expect(defineCalls).toHaveLength(2);

    // Now test Snapshot network error clearing snapshotting() in finally
    const snapBtn = snapshotButton();
    snapBtn.click();
    expect(snapBtn.disabled).toBe(true);

    snapshotCall1.reject(new TypeError('Failed to fetch'));
    await settle();
    expect(snapBtn.disabled).toBe(false);

    // Retry Snapshot succeeds
    snapBtn.click();
    await settle();
    expect(snapshotCalls).toHaveLength(2);
  });
});
