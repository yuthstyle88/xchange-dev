// KC-W (kiosk-cancel FE, docs/design/blueprint/kiosk-cancel.md §2, §3.5, §5 row KC-W): the kiosk queue panel's cancel.
// Written before the implementation by the sub-controller claude/36d9dc6a. Do not edit, delete or weaken these
// tests; add tests in a file of your own.
//
// What these tests pin (on top of KC-R's CancelReasonDialog and cancelKioskRequest):
// - Every row on the waiting and working tabs has a button[data-action="cancel-request"]; completed rows do not.
// - The button opens CancelReasonDialog (dialog.xc-dialog with select[name="reason"]). Confirm posts
//   POST /api/kiosk {action: "cancel", payload: {id, branchId, reason, note?}} for that row and branch.
// - On success the row leaves the panel. Dismiss posts nothing.
// - On a refusal (400 {"error": sentence} → ApiError code `refused`), the server's sentence shows inside that row's
//   <article> and the row stays.
// Wire shape and sentences: AG controller KC, #564 issuecomment-6307996084.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type PageParams, type ViewAnswer, type ViewParams } from '../api/client';
import { setLang } from '../i18n';
import type { SessionStore } from '../session/session';
import { queueStore } from '../queue/queue-store';
import type { KioskQueueRow, Me } from '../testing/fixtures';
import { makeKioskQueueRow, makeQueueBoardData } from '../testing/fixtures';
import { QueuePanel } from './QueuePanel';
import { meOf } from '../../test-support/me';

const T0 = Date.parse('2026-10-21T03:00:00.000Z');
const ME: Me = meOf({ id: 'E1', role: 'FRONT', branch: 'B-HOME', name: 'Somchai', branchName: 'B-HOME', branches: [] });

type State = 'waiting' | 'working' | 'completed';

function row(branch: string, n: number, over: Partial<KioskQueueRow> = {}) {
  return makeKioskQueueRow({
    id: `${branch}-kq-${n}`,
    queue: `${branch}-Q${n}`,
    orderNo: `ORD-${n}`,
    transactionNo: null,
    status: 'WAITING',
    type: 'BUY',
    currency: 'USD',
    foreign: '100.00',
    thb: '3325.00',
    staff: '',
    startedAt: null,
    completedAt: null,
    durationSeconds: null,
    canAccept: true,
    canContinue: false,
    ...over,
  });
}

interface Post {
  path: string;
  body: { action: string; payload: Record<string, unknown> };
}

/** A fake server: kiosk_queue / queue_board reads, and POST /api/kiosk cancel removes the row. */
class FakeApi {
  version = 3;
  readonly rows: Record<State, KioskQueueRow[]> = { waiting: [], working: [], completed: [] };
  readonly posts: Post[] = [];
  /** When set, the next cancel is refused with this error. */
  refuseNext: unknown = undefined;

  readonly api = {
    view: async <T,>(name: string, params: ViewParams = {}): Promise<ViewAnswer<T>> => {
      await Promise.resolve();
      if (name === 'queue_board') {
        if (params.since !== undefined && params.since === this.version) return { version: this.version, changed: false };
        return {
          version: this.version,
          changed: true,
          data: makeQueueBoardData({ waiting: [], called: [], myCounter: null }) as unknown as T,
        };
      }
      if (name !== 'kiosk_queue') throw new Error(`unexpected view ${name}`);
      const state = ((params as ViewParams<{ state?: string }>).filter?.state ?? 'waiting') as State;
      if (params.since !== undefined && params.since === this.version) return { version: this.version, changed: false };
      return { version: this.version, changed: true, data: this.rows[state].slice(0, params.limit ?? 51) as unknown as T };
    },
    page: async <R,>(name: string, params: PageParams = {}): Promise<R[]> => {
      await Promise.resolve();
      if (name !== 'kiosk_queue') throw new Error(`unexpected page ${name}`);
      const state = ((params as PageParams<{ state?: string }>).filter?.state ?? 'waiting') as State;
      return this.rows[state].slice(0, params.limit ?? 51) as unknown as R[];
    },
    post: async <B, R>(path: string, body: B): Promise<R> => {
      const b = body as unknown as Post['body'];
      this.posts.push({ path, body: b });
      await Promise.resolve();
      if (b.action !== 'cancel') throw new Error(`unexpected kiosk action ${b.action}`);
      if (this.refuseNext !== undefined) {
        const e = this.refuseNext;
        this.refuseNext = undefined;
        throw e;
      }
      const id = String(b.payload.id);
      for (const s of ['waiting', 'working'] as const) {
        const i = this.rows[s].findIndex((r) => r.id === id);
        if (i >= 0) {
          const [r] = this.rows[s].splice(i, 1);
          this.version += 1;
          return { id, queue: r!.queue, orderNo: r!.orderNo, transactionNo: r!.transactionNo, status: 'CANCELLED' } as R;
        }
      }
      return { id, queue: '', orderNo: null, transactionNo: null, status: 'CANCELLED' } as R;
    },
  };
}

function session(): SessionStore {
  const [me] = createSignal<Me | undefined>(ME);
  return { me, signOut: vi.fn(async () => {}), dispose: () => {} };
}

let fake: FakeApi;
let branchNo = 0;
const disposers: Array<() => void> = [];

function mount(): { host: HTMLElement; branch: string } {
  branchNo += 1;
  const branch = `KC${branchNo}`;
  queueStore(branch, { api: fake.api as unknown as ApiClient, session: session() });
  const host = document.createElement('div');
  document.body.append(host);
  disposers.push(render(() => <QueuePanel branch={branch} shown api={fake.api as unknown as ApiClient} />, host));
  return { host, branch };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await vi.advanceTimersByTimeAsync(0);
}

function text(node: Element | null | undefined): string {
  return (node?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function card(host: Element, queue: string): HTMLElement | undefined {
  return [...host.querySelectorAll('article')].find((a) =>
    [...a.querySelectorAll('h1,h2,h3,h4,h5,h6')].some((h) => text(h) === queue),
  ) as HTMLElement | undefined;
}

function tab(host: Element, label: string): HTMLButtonElement {
  const b = [...host.querySelectorAll('nav.xc-queue-tabs button')].find((x) => text(x) === label);
  if (!b) throw new Error(`no tab "${label}"`);
  return b as HTMLButtonElement;
}

function cancelBtn(article: Element): HTMLButtonElement | null {
  return article.querySelector<HTMLButtonElement>('button[data-action="cancel-request"]');
}

function dialog(): HTMLDialogElement | null {
  return document.querySelector<HTMLDialogElement>('dialog.xc-dialog');
}

async function confirmWith(reason: string, note?: string): Promise<void> {
  const d = dialog();
  if (!d) throw new Error('CancelReasonDialog did not open');
  const sel = d.querySelector<HTMLSelectElement>('select[name="reason"]')!;
  sel.value = reason;
  sel.dispatchEvent(new Event('change', { bubbles: true }));
  if (note !== undefined) {
    const n = d.querySelector<HTMLTextAreaElement>('textarea[name="note"]')!;
    n.value = note;
    n.dispatchEvent(new Event('input', { bubbles: true }));
  }
  d.querySelector<HTMLButtonElement>('button[data-action="confirm-cancel-request"]')!.click();
  await flush();
}

beforeEach(() => {
  vi.useFakeTimers({ now: T0 });
  fake = new FakeApi();
  setLang('th');
});

afterEach(() => {
  for (const dispose of disposers.splice(0)) dispose();
  vi.clearAllTimers();
  vi.useRealTimers();
  document.body.innerHTML = '';
});

describe('KC-W: QueuePanel cancel (kiosk-cancel.md §3.5)', () => {
  it('row 1: every waiting row has a cancel button; confirming posts the cancel for that row and branch, and the row leaves', async () => {
    const p = mount();
    fake.rows.waiting.push(row(p.branch, 1), row(p.branch, 2));
    await flush();
    const a = card(p.host, `${p.branch}-Q1`)!;
    expect(cancelBtn(a), 'cancel-request on a waiting row').not.toBeNull();
    expect(cancelBtn(card(p.host, `${p.branch}-Q2`)!)).not.toBeNull();

    cancelBtn(a)!.click();
    await flush();
    expect(dialog(), 'CancelReasonDialog opens').not.toBeNull();
    await confirmWith('CUSTOMER_LEFT');

    expect(fake.posts).toHaveLength(1);
    expect(fake.posts[0]).toEqual({
      path: '/api/kiosk',
      body: { action: 'cancel', payload: { id: `${p.branch}-kq-1`, branchId: p.branch, reason: 'CUSTOMER_LEFT' } },
    });
    expect(card(p.host, `${p.branch}-Q1`), 'the cancelled row leaves the panel').toBeUndefined();
    expect(card(p.host, `${p.branch}-Q2`), 'the other row stays').toBeDefined();
  });

  it('row 2: a working row has a cancel button too; OTHER sends its trimmed note', async () => {
    const p = mount();
    fake.rows.working.push(row(p.branch, 7, { status: 'ACCEPTED', staff: 'somchai', canAccept: false, canContinue: true }));
    await flush();
    tab(p.host, 'กำลังทำรายการ').click();
    await flush();
    const a = card(p.host, `${p.branch}-Q7`)!;
    expect(cancelBtn(a), 'cancel-request on a working row').not.toBeNull();
    cancelBtn(a)!.click();
    await flush();
    await confirmWith('OTHER', '  wrong currency  ');
    expect(fake.posts[0]!.body.payload).toEqual({
      id: `${p.branch}-kq-7`,
      branchId: p.branch,
      reason: 'OTHER',
      note: 'wrong currency',
    });
  });

  it('row 3: completed rows have no cancel button', async () => {
    const p = mount();
    fake.rows.completed.push(row(p.branch, 9, { status: 'COMPLETED', canAccept: false, completedAt: new Date(T0).toISOString() }));
    await flush();
    tab(p.host, 'เสร็จสิ้น').click();
    await flush();
    const a = card(p.host, `${p.branch}-Q9`)!;
    expect(a).toBeDefined();
    expect(cancelBtn(a)).toBeNull();
  });

  it('row 4: a refusal shows the server sentence inside that row, and the row stays', async () => {
    const p = mount();
    fake.rows.waiting.push(row(p.branch, 3));
    await flush();
    fake.refuseNext = new ApiError({ code: 'refused', message: 'คำขอนี้มีพนักงานคนอื่นรับเรื่องอยู่', status: 400 });
    cancelBtn(card(p.host, `${p.branch}-Q3`)!)!.click();
    await flush();
    await confirmWith('DUPLICATE');
    const a = card(p.host, `${p.branch}-Q3`);
    expect(a, 'the refused row stays').toBeDefined();
    expect(text(a)).toContain('คำขอนี้มีพนักงานคนอื่นรับเรื่องอยู่');
  });

  it('row 5: dismissing the dialog posts nothing', async () => {
    const p = mount();
    fake.rows.waiting.push(row(p.branch, 4));
    await flush();
    cancelBtn(card(p.host, `${p.branch}-Q4`)!)!.click();
    await flush();
    dialog()!.querySelector<HTMLButtonElement>('button[data-action="dismiss"]')!.click();
    await flush();
    expect(fake.posts).toHaveLength(0);
    expect(card(p.host, `${p.branch}-Q4`)).toBeDefined();
  });
});
