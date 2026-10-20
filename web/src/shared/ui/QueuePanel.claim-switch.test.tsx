import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import {
  ApiError,
  type ApiClient,
  type PageParams,
  type ViewAnswer,
  type ViewParams,
} from '../api/client';
import { setLang } from '../i18n';
import type { SessionStore } from '../session/session';
import { queueStore, type QueueStore } from '../queue/queue-store';
import type { KioskQueueRow, Me } from '../testing/fixtures';
import { makeKioskQueueRow, makeQueueBoardData } from '../testing/fixtures';
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

class Branch {
  version = 7;
  readonly rows: Record<State, KioskQueueRow[]> = { waiting: [], working: [], completed: [] };
  constructor(readonly id: string) {}
  board() {
    return makeQueueBoardData({
      waiting: this.rows.waiting.map((r, i) => ({
        id: r.id,
        displayNumber: r.queue,
        serviceType: 'WALK_IN',
        sequence: i + 1,
        issuedAt: new Date(T0).toISOString(),
      })),
      called: [],
      myCounter: null,
    });
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
  failNextPost: unknown = undefined;
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

  queueReads(branch?: string, state?: string): Call[] {
    return this.calls.filter(
      (c) =>
        c.name === 'kiosk_queue' &&
        (branch === undefined || String(c.params.branch) === branch) &&
        (state === undefined || c.params.filter?.state === state),
    );
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
      if (this.failNextPost !== undefined) {
        const err = this.failNextPost;
        this.failNextPost = undefined;
        throw err;
      }
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

let branchNo = 3000;
function freshBranch(): string {
  branchNo += 1;
  return `QP-SW-${branchNo}`;
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

function message(host: Element): string {
  return text(host.querySelector('.queue-message') ?? host.querySelector('[role="status"]'));
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

function pressed(host: Element): string[] {
  return [...host.querySelectorAll('button[aria-pressed="true"]')].map((b) => text(b));
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

describe('QueuePanel claim switch guards (#2713)', () => {
  it('1. Claim on branch A held, switch to B, A claim is refused -> B shows no refusal, no extra queue read', async () => {
    const branchA = freshBranch();
    const branchB = freshBranch();
    fake.branch(branchA).rows.waiting.push(row(branchA, 1));
    fake.branch(branchB).rows.waiting.push(row(branchB, 1));

    const p = mount({ branch: branchA });
    await flush();

    const sentence = 'ออเดอร์นี้มีผู้รับเรื่องแล้ว';
    fake.failNextPost = new ApiError({ code: 'refused', message: sentence, status: 400 });
    const release = fake.holdNextPost();

    button(card(p.host, `${branchA}-Q1`), 'รับเรื่องนี้').click();
    await flush();

    p.setBranch(branchB);
    await flush();

    const readsBeforeRelease = fake.queueReads(branchB).length;

    release();
    await flush();

    expect(message(p.host)).not.toContain(sentence);
    expect(fake.queueReads(branchB).length).toBe(readsBeforeRelease);
  });

  it('2. Claim on the Waiting tab held, switch to Completed, the claim is refused -> Completed shows no refusal', async () => {
    const b = freshBranch();
    fake.branch(b).rows.waiting.push(row(b, 1));
    fake.branch(b).rows.completed.push(row(b, 2, { status: 'COMPLETED' }));

    const p = mount({ branch: b });
    await flush();

    const sentence = 'ออเดอร์นี้มีผู้รับเรื่องแล้ว';
    fake.failNextPost = new ApiError({ code: 'refused', message: sentence, status: 400 });
    const release = fake.holdNextPost();

    button(card(p.host, `${b}-Q1`), 'รับเรื่องนี้').click();
    await flush();

    button(p.host, 'เสร็จสิ้น').click();
    await flush();
    expect(pressed(p.host)).toEqual(['เสร็จสิ้น']);

    release();
    await flush();

    expect(message(p.host)).not.toContain(sentence);
  });

  it('3. Claim on A, no switch, refused -> The refusal is shown and the queue re-reads', async () => {
    const b = freshBranch();
    fake.branch(b).rows.waiting.push(row(b, 1));

    const p = mount({ branch: b });
    await flush();

    const sentence = 'ออเดอร์นี้มี malee กำลังทำรายการแล้ว';
    fake.failNextPost = new ApiError({ code: 'refused', message: sentence, status: 400 });

    const readsBefore = fake.queueReads(b, 'waiting').length;

    button(card(p.host, `${b}-Q1`), 'รับเรื่องนี้').click();
    await flush();

    expect(message(p.host)).toBe(sentence);
    expect(fake.queueReads(b, 'waiting').length).toBeGreaterThan(readsBefore);
    expect(pressed(p.host)).toEqual(['รอรับเรื่อง']);
  });

  it('4. Claim on A, switch to B, the claim succeeds -> onClaimed is not called on B', async () => {
    const onClaimed = vi.fn();
    const branchA = freshBranch();
    const branchB = freshBranch();
    fake.branch(branchA).rows.waiting.push(row(branchA, 1));
    fake.branch(branchB).rows.waiting.push(row(branchB, 1));

    const p = mount({ branch: branchA, onClaimed });
    await flush();

    const release = fake.holdNextPost();
    button(card(p.host, `${branchA}-Q1`), 'รับเรื่องนี้').click();
    await flush();

    p.setBranch(branchB);
    await flush();

    release();
    await flush();

    expect(onClaimed).not.toHaveBeenCalled();
  });
});
