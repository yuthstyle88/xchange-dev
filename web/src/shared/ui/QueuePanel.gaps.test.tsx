import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
// QU-6 (#1354, ruling issuecomment-6118459878) by the test-writer claude/14cc24bd: the shared store polls `queue_board`
// (the panel itself stays on `kiosk_queue`), so the fake answers the store's poll as typed `QueueBoardData`. The panel's
// `kiosk_queue` answers and every assertion are unchanged.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `Me` fixture (`kind`/`role`/`username`) → `meOf(…)` and `employee {id, role}` → `employeeOf(…)` (`web/src/test-support/me.ts`, design §3.3: FRONT → TELLER at its branch). Nothing else changes.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import type { ApiClient, PageParams, ViewAnswer, ViewParams } from '../api/client';
import type { Me } from '../api/generated/common';
import { setLang } from '../i18n';
import type { SessionStore } from '../session/session';
import { queueStore, type QueueStore } from '../queue/queue-store';
import type { QueueBoardData } from '../api/generated/queue_board';
import type { KioskQueueRow } from '../queue/wire';
import { QueuePanel } from './QueuePanel';
import { meOf } from '../../test-support/me';

const T0 = Date.parse('2026-10-08T03:00:00.000Z');
const SKEW = 10 * 60 * 1000;

const ME: Me = meOf({
  id: 'E1',
  role: 'FRONT',
  branch: 'B-HOME',
  name: 'Somchai',
  branchName: 'B-HOME',
  branches: [],
});

type State = 'waiting' | 'working' | 'completed';

function row(branch: string, n: number, over: Partial<KioskQueueRow> = {}): KioskQueueRow {
  return {
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
  };
}

function rows(branch: string, count: number, over: Partial<KioskQueueRow> = {}): KioskQueueRow[] {
  return Array.from({ length: count }, (_, i) => row(branch, i + 1, over));
}

class Branch {
  version = 7;
  readonly rows: Record<State, KioskQueueRow[]> = { waiting: [], working: [], completed: [] };
  constructor(readonly id: string) {}
  /** The store's poll (`queue_board`): one WAITING ticket per waiting request, in call order. */
  board(): QueueBoardData {
    return {
      waiting: this.rows.waiting.map((r, i) => ({
        id: r.id,
        displayNumber: r.queue,
        serviceType: 'WALK_IN',
        sequence: i + 1,
        issuedAt: new Date(T0).toISOString(),
      })),
      called: [],
      myCounter: null,
    };
  }
}

interface Call {
  method: 'view' | 'page';
  name: string;
  params: ViewParams<{ state?: string }> & PageParams<{ state?: string }>;
}

const settled = () => Promise.resolve();

class FakeApi {
  readonly branches = new Map<string, Branch>();
  readonly calls: Call[] = [];
  readonly posts: Array<{ path: string; body: unknown }> = [];
  private holdRead = false;
  private holdPost = false;
  private readonly held: Array<() => void> = [];

  branch(id: string): Branch {
    let b = this.branches.get(id);
    if (!b) {
      b = new Branch(id);
      this.branches.set(id, b);
    }
    return b;
  }

  holdNextRead(): () => void {
    this.holdRead = true;
    return this.releaser();
  }

  holdNextPost(): () => void {
    this.holdPost = true;
    return this.releaser();
  }

  private releaser(): () => void {
    const index = this.held.length;
    return () => {
      const release = this.held[index];
      if (release) release();
    };
  }

  private async wait(kind: 'read' | 'post'): Promise<void> {
    if (kind === 'read' ? this.holdRead : this.holdPost) {
      if (kind === 'read') this.holdRead = false;
      else this.holdPost = false;
      await new Promise<void>((resolve) => this.held.push(resolve));
    }
  }

  readonly api = {
    view: async <T,>(name: string, params: ViewParams = {}): Promise<ViewAnswer<T>> => {
      const p = { ...params } as Call['params'];
      this.calls.push({ method: 'view', name, params: p });
      await settled();
      const b = this.branch(String(p.branch));
      if (name === 'queue_board') {
        if (p.since !== undefined && p.since === b.version) return { version: b.version, changed: false };
        return { version: b.version, changed: true, data: b.board() as unknown as T };
      }
      const version = b.version;
      const list = b.rows[(p.filter?.state ?? 'waiting') as State].slice(0, p.limit ?? 51);
      await this.wait('read');
      if (p.since !== undefined && p.since === version) return { version, changed: false };
      return { version, changed: true, data: list as unknown as T };
    },
    page: async <R,>(name: string, params: PageParams = {}): Promise<R[]> => {
      const p = { ...params } as Call['params'];
      this.calls.push({ method: 'page', name, params: p });
      await settled();
      const b = this.branch(String(p.branch));
      const all = b.rows[(p.filter?.state ?? 'waiting') as State];
      const from = p.before === undefined ? 0 : all.findIndex((r) => r.id === p.before) + 1;
      const list = all.slice(from, from + (p.limit ?? 51));
      await this.wait('read');
      return list as unknown as R[];
    },
    post: async <B, R>(path: string, body: B): Promise<R> => {
      this.posts.push({ path, body });
      await this.wait('post');
      const accept = body as unknown as { payload: { id: string; branchId: string } };
      const b = this.branch(accept.payload.branchId);
      const i = b.rows.waiting.findIndex((r) => r.id === accept.payload.id);
      if (i >= 0) {
        const [claimed] = b.rows.waiting.splice(i, 1);
        b.rows.working.unshift({
          ...claimed!,
          status: 'ACCEPTED',
          staff: 'somchai',
          startedAt: new Date(Date.now() + SKEW).toISOString(),
          canAccept: false,
          canContinue: true,
        });
        b.version += 1;
      }
      return { saved: true, claimed: true } as R;
    },
  };
}

function session(): SessionStore {
  const [me] = createSignal<Me | undefined>(ME);
  return { me, signOut: vi.fn(async () => {}), dispose: () => {} };
}

let branchNo = 2000;
function freshBranch(): string {
  branchNo += 1;
  return `QP-GAP-${branchNo}`;
}

let fake: FakeApi;
const disposers: Array<() => void> = [];

function storeOf(branch: string): QueueStore {
  return queueStore(branch, { api: fake.api as unknown as ApiClient, session: session() });
}

function mount(opts: { branch?: string; shown?: boolean; onClaimed?: (id: string) => void } = {}) {
  const [branch, setBranch] = createSignal(opts.branch ?? freshBranch());
  const [shown, setShown] = createSignal(opts.shown ?? true);
  storeOf(branch());
  const host = document.createElement('div');
  document.body.append(host);
  disposers.push(
    render(
      () => (
        <QueuePanel
          branch={branch()}
          shown={shown()}
          onClaimed={opts.onClaimed}
          api={fake.api as unknown as ApiClient}
        />
      ),
      host,
    ),
  );
  return {
    host,
    branch,
    data: () => fake.branch(branch()),
    store: () => storeOf(branch()),
    setShown,
    setBranch: (b: string) => {
      storeOf(b);
      setBranch(b);
    },
  };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) await vi.advanceTimersByTimeAsync(0);
}

function text(node: Element | null | undefined): string {
  return (node?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function button(host: Element, label: string): HTMLButtonElement {
  const found = [...host.querySelectorAll('button')].find((b) => text(b) === label);
  if (!found) throw new Error(`no button "${label}" in: ${text(host)}`);
  return found;
}

function articles(host: Element): HTMLElement[] {
  return [...host.querySelectorAll('article')];
}

function card(host: Element, queue: string): HTMLElement {
  const found = articles(host).find((a) =>
    [...a.querySelectorAll('h1,h2,h3,h4,h5,h6')].some((h) => text(h) === queue),
  );
  if (!found) throw new Error(`no card headed "${queue}"`);
  return found;
}

function shownQueues(host: Element): string[] {
  return articles(host).map((a) => text(a.querySelector('h1,h2,h3,h4,h5,h6')));
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

describe('QueuePanel gaps (issue #486 rework)', () => {
  // Mutant: QueuePanel.tsx:237-239 adding `props.onClaimed?.(rowId)` in the branch-changed path
  it('assert onClaimed is not called after a branch change', async () => {
    const onClaimed = vi.fn();
    const p = mount({ onClaimed });
    const b = p.branch();
    p.data().rows.waiting.push(row(b, 1));
    await flush();

    const release = fake.holdNextPost();
    button(card(p.host, `${b}-Q1`), 'รับเรื่องนี้').click();
    await flush();

    const other = freshBranch();
    fake.branch(other).rows.waiting.push(row(other, 1));
    p.setBranch(other);
    await flush();

    release();
    await flush();

    expect(onClaimed).not.toHaveBeenCalled();
  });

  // Mutant: QueuePanel.tsx:97 keeping only `seq !== currentSeq`
  it('a branch change while the panel is not shown must still drop the late answer', async () => {
    const first = freshBranch();
    fake.branch(first).rows.waiting.push(...rows(first, 2));
    const release = fake.holdNextRead();
    const p = mount({ branch: first });
    await flush();

    // Hide the panel while the initial read of `first` is in flight
    p.setShown(false);
    await flush();

    // Change branch while hidden (clears list, does not start a new read because shown is false)
    const other = freshBranch();
    p.setBranch(other);
    await flush();

    // Release the late answer for `first`
    release();
    await flush();

    // The late answer for `first` must be dropped, not populate the list
    expect(articles(p.host)).toHaveLength(0);
    expect(shownQueues(p.host)).toEqual([]);
  });
});
