// #2533: Desk refunds — a failed desk_refunds read (initial or Refresh) must set postError to wordedError(err)
// with canRetry=false and keep current rows, while a later successful read replaces rows and leaves next-post handling intact.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient } from '../../../../shared/api/client';
import type { DeskRefundRow } from '../../../../shared/api/generated/desk_refunds';
import { setLang, t as sharedT } from '../../../../shared/i18n';
import * as texts from './i18n';

type ReadReply = { rows: DeskRefundRow[] } | { status: number; code?: string; message?: string } | 'network';
type PostReply = { doc: string } | { refuse: string; status?: number } | 'network';

let readReplies: ReadReply[] = [];
let postReplies: PostReply[] = [];
const postedIds: string[] = [];

vi.mock('./post', () => ({
  createPoster: (_api: unknown, id: string) => ({
    async post(): Promise<string> {
      postedIds.push(id);
      const r = postReplies.shift();
      if (!r || r === 'network') throw new TypeError('Failed to fetch');
      if ('refuse' in r) {
        throw new ApiError({ code: r.refuse, message: r.refuse, status: r.status ?? 409 });
      }
      return r.doc;
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
      reason: `เหตุผล ${id}`,
      amount: '100.00',
      requestedBy: 'u-1',
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
      if (name === 'desk_refunds') {
        const r = readReplies.shift();
        if (!r || r === 'network') throw new TypeError('Failed to fetch');
        if ('rows' in r) return { data: { rows: r.rows } };
        throw new ApiError({
          code: r.code ?? 'serverError',
          message: r.message ?? 'Internal server error',
          status: r.status,
        });
      }
      return { data: null };
    },
  } as unknown as ApiClient;
}

function mountStep() {
  const store = createRefundsStore(fakeApi());
  store.setOpen(true);
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <RefundsStep store={store} />, host);
  return store;
}

const shown = () => (host.textContent ?? '').replace(/\s+/g, ' ').trim();
const retryButton = () => host.querySelector<HTMLButtonElement>('button[data-action="retry"]');
const refreshButton = () => host.querySelector<HTMLButtonElement>('button[data-action="refresh"]')!;

beforeEach(() => {
  setLang('th');
  readReplies = [];
  postReplies = [];
  postedIds.length = 0;
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

describe('Desk refunds read error handling (#2533 §8 & §9)', () => {
  it('row 1: Initial read answers 500 -> postError is the worded server error; rows unchanged', async () => {
    readReplies = [
      { status: 500, code: 'serverError', message: 'db down' },
      { rows: [row('rv-1'), row('rv-2')] },
      { status: 500, code: 'serverError', message: 'db down again' },
    ];
    const store = mountStep();

    // Initial read fails with 500 -> postError is worded server error, canRetry is false, rows stay []
    await store.readRefunds();
    expect(store.postError()).toBe(sharedT('error.serverError'));
    expect(store.canRetry()).toBe(false);
    expect(store.rows()).toEqual([]);
    expect(shown()).toContain(sharedT('error.serverError'));
    expect(retryButton()).toBeNull();

    // When rows are already loaded and a subsequent 500 read occurs, existing rows remain unchanged
    await store.readRefunds();
    expect(store.rows().map((r) => r.reversal.id)).toEqual(['rv-1', 'rv-2']);

    await store.readRefunds();
    expect(store.postError()).toBe(sharedT('error.serverError'));
    expect(store.canRetry()).toBe(false);
    expect(store.rows().map((r) => r.reversal.id)).toEqual(['rv-1', 'rv-2']);
    expect(host.querySelectorAll('[data-reversal]')).toHaveLength(2);
  });

  it('row 2: Refresh read answers a network error -> postError is the worded network error', async () => {
    readReplies = [{ rows: [row('rv-1')] }, 'network'];
    const store = mountStep();

    await store.readRefunds();
    expect(store.rows().map((r) => r.reversal.id)).toEqual(['rv-1']);
    expect(store.postError()).toBeNull();

    // Clicking Refresh triggers void store.readRefunds() which encounters a network error
    refreshButton().click();
    await Promise.resolve();
    await Promise.resolve();

    expect(store.postError()).toBe(sharedT('error.network'));
    expect(store.canRetry()).toBe(false);
    expect(store.rows().map((r) => r.reversal.id)).toEqual(['rv-1']);
    expect(shown()).toContain(sharedT('error.network'));
    expect(retryButton()).toBeNull();
  });

  it('row 3: Read succeeds after an earlier failure -> rows replaced by the answer (error handling for the next post unchanged)', async () => {
    readReplies = [
      { status: 500, code: 'serverError', message: 'initial read failed' },
      { rows: [row('rv-1'), row('rv-2')] },
      { rows: [row('rv-2')] },
    ];
    postReplies = [
      { refuse: 'reversalWindowClosed', status: 409 },
      { doc: 'XC-0001-R1' },
    ];
    const store = mountStep();

    // 1. Earlier read failure sets postError to worded server error
    await store.readRefunds();
    expect(store.postError()).toBe(sharedT('error.serverError'));
    expect(store.rows()).toEqual([]);

    // 2. Subsequent read succeeds -> rows are replaced by the answer; readRefunds does not add new postError-clearing behaviour (§9)
    await store.readRefunds();
    expect(store.rows().map((r) => r.reversal.id)).toEqual(['rv-1', 'rv-2']);
    expect(host.querySelectorAll('[data-reversal]')).toHaveLength(2);
    expect(store.postError()).toBe(sharedT('error.serverError'));

    // 3. Error handling for the next post is unchanged (refusal replaces postError, then a successful post clears postError and shows doc)
    store.startConfirm('rv-1');
    await store.submitPost();
    expect(store.postError()).toBe(texts.th['error.reversalWindowClosed']);
    expect(store.canRetry()).toBe(false);

    store.startConfirm('rv-1');
    await store.submitPost();
    expect(store.postError()).toBeNull();
    expect(store.lastPostedDoc()).toBe('XC-0001-R1');
    expect(store.rows().map((r) => r.reversal.id)).toEqual(['rv-2']);
  });

  // Kept row from §8:
  // - "Post then read flows" is kept and pinned by refunds-store.outcome.test.tsx, refunds-store.lastdoc.test.tsx, and refunds-store.reopen.test.tsx.
});
