// #2487: Desk refunds — a post refusal (and its Retry button / retry state) must not stay on screen after the
// refunds panel is closed and reopened for the next customer; closing while a post is in flight clears nothing.
// Renders the real RefundsStep over the real store; the poster and the api client are stubbed.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient } from '../../../../shared/api/client';
import type { DeskRefundRow } from '../../../../shared/api/generated/desk_refunds';
import { setLang } from '../../../../shared/i18n';
import * as texts from './i18n';

type Reply =
  | { doc: string }
  | { refuse: string; status?: number }
  | 'network'
  | { hang: Promise<Reply> };

let replies: Reply[] = [];
const postedIds: string[] = [];

async function runReply(r: Reply | undefined): Promise<string> {
  if (!r || r === 'network') throw new TypeError('Failed to fetch');
  if ('hang' in r) return runReply(await r.hang);
  if ('refuse' in r) {
    throw new ApiError({ code: r.refuse, message: r.refuse, status: r.status ?? 409 });
  }
  return r.doc;
}

vi.mock('./post', () => ({
  createPoster: (_api: unknown, id: string) => ({
    async post(): Promise<string> {
      postedIds.push(id);
      return runReply(replies.shift());
    },
  }),
}));

const { createRefundsStore } = await import('./refunds-store');
const { RefundsStep } = await import('./RefundsStep');

function row(id: string): DeskRefundRow {
  return {
    reversal: {
      id,
      originalTransactionId: `tx-${id}`,
      status: 'APPROVED',
      reason: 'r',
      amount: '1.00',
      requestedBy: 'u',
      requestedAt: '2026-10-11T04:00:00Z',
      approvedAt: '2026-10-11T04:10:00Z',
      postedAt: null,
      documentNumber: null,
    },
    receiptNumber: `XC-${id}`,
    lines: [],
    payments: [],
  } as DeskRefundRow;
}

let dispose: (() => void) | undefined;
let host: HTMLDivElement;

function fakeApi(): ApiClient {
  return {
    async view(name: string) {
      if (name === 'desk_refunds') return { data: { rows: [row('rv-1'), row('rv-2')] } };
      return { data: null };
    },
  } as unknown as ApiClient;
}

async function setup() {
  const store = createRefundsStore(fakeApi());
  store.setOpen(true);
  await store.readRefunds();
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <RefundsStep store={store} />, host);
  return store;
}

async function post(store: Awaited<ReturnType<typeof setup>>, id: string): Promise<void> {
  store.startConfirm(id);
  await store.submitPost();
}

const shown = () => (host.textContent ?? '').replace(/\s+/g, ' ');
const retryButton = () => host.querySelector<HTMLButtonElement>('button[data-action="retry"]');
const closeButton = () => host.querySelector<HTMLButtonElement>('button[data-action="close"]')!;

beforeEach(() => {
  setLang('th');
  replies = [];
  postedIds.length = 0;
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

describe('Desk refunds: post refusal and Retry do not outlive panel close (#2487 §8 & §9)', () => {
  it('row 1: post refused, Close, reopen -> no refusal text and no Retry', async () => {
    const store = await setup();
    replies = [
      { refuse: 'stockInsufficient', status: 409 },
      { refuse: 'serverError', status: 503 },
    ];

    // 409 domain refusal -> shown before close, cleared after close + reopen
    await post(store, 'rv-1');
    expect(shown()).toContain(texts.th['error.stockInsufficient']);
    expect(store.postError()).toBe(texts.th['error.stockInsufficient']);
    expect(retryButton()).toBeNull();

    closeButton().click();
    expect(store.open()).toBe(false);
    store.setOpen(true);
    await store.readRefunds();

    expect(store.postError()).toBeNull();
    expect(store.canRetry()).toBe(false);
    expect(shown(), 'refusal text is gone after reopen').not.toContain(texts.th['error.stockInsufficient']);
    expect(retryButton(), 'no Retry button after reopen').toBeNull();

    // 503 server refusal -> shows error.unknown and Retry before close, both cleared after close + reopen
    await post(store, 'rv-1');
    expect(shown()).toContain(texts.th['error.unknown']);
    expect(store.canRetry()).toBe(true);
    expect(retryButton()).not.toBeNull();

    closeButton().click();
    store.setOpen(true);
    await store.readRefunds();

    expect(store.postError()).toBeNull();
    expect(store.canRetry()).toBe(false);
    expect(shown(), '5xx refusal text is gone after reopen').not.toContain(texts.th['error.unknown']);
    expect(retryButton(), 'no Retry button after reopen').toBeNull();

    await store.retry();
    expect(postedIds, 'retry() after close does not re-post the old refund').toEqual(['rv-1', 'rv-1']);
  });

  it('row 2: post fails with a network error, Close, reopen -> no Retry for the old refund', async () => {
    const store = await setup();
    replies = ['network', { doc: 'XC-0001-R1' }];

    await post(store, 'rv-1');
    expect(store.postError()).toBe(texts.th['error.unknown']);
    expect(store.canRetry()).toBe(true);
    expect(shown()).toContain(texts.th['error.unknown']);
    expect(retryButton()).not.toBeNull();

    closeButton().click();
    expect(store.open()).toBe(false);
    store.setOpen(true);
    await store.readRefunds();

    expect(store.postError()).toBeNull();
    expect(store.canRetry()).toBe(false);
    expect(shown(), 'network error text is gone after reopen').not.toContain(texts.th['error.unknown']);
    expect(retryButton(), 'no Retry button for the old refund').toBeNull();

    await store.retry();
    expect(postedIds, 'retry() is a no-op after close').toEqual(['rv-1']);
  });

  // Kept rows from §8:
  // - "Post refused, no close -> Refusal and (for 5xx) Retry stay" is pinned by refunds-store.outcome.test.tsx.
  // - "Last posted document cleared on close" is pinned by refunds-store.lastdoc.test.tsx.
  // - "In-flight posting guard" is pinned by refunds-store.inflight.test.tsx.

  it('edge: closing while a post is in flight clears nothing; its answer and retry still apply, and a later close when idle clears them', async () => {
    const store = await setup();
    let releaseInFlight!: (r: Reply) => void;
    const hangPromise = new Promise<Reply>((resolve) => {
      releaseInFlight = resolve;
    });
    replies = [{ hang: hangPromise }, 'network'];

    store.startConfirm('rv-1');
    const inFlightPost = store.submitPost();
    expect(store.posting()).toBe(true);

    // Close and reopen while the post is still in flight -> must not clear currentPoster / currentPostingId
    closeButton().click();
    expect(store.open()).toBe(false);
    store.setOpen(true);

    // The in-flight post now fails with a network error -> its answer applies and Retry still re-sends rv-1
    releaseInFlight('network');
    await inFlightPost;

    expect(store.postError()).toBe(texts.th['error.unknown']);
    expect(store.canRetry()).toBe(true);
    expect(retryButton()).not.toBeNull();

    await store.retry();
    expect(postedIds, 'retry() still works because close during in-flight post did not clear poster').toEqual([
      'rv-1',
      'rv-1',
    ]);
    expect(store.canRetry()).toBe(true);

    // Now that no post is in flight, closing and reopening clears the error and retry state
    closeButton().click();
    store.setOpen(true);
    expect(store.postError()).toBeNull();
    expect(store.canRetry()).toBe(false);
    expect(retryButton()).toBeNull();

    await store.retry();
    expect(postedIds).toEqual(['rv-1', 'rv-1']);
  });
});
