// #2630: clicking Next/Prev twice quickly while a page read is in flight must
// not advance the page number twice or corrupt the cursor stack.
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
import type { Me } from '../api/generated/common';
import { setLang } from '../i18n';
import type { SessionStore } from '../session/session';
import { queueStore, type QueueStore } from '../queue/queue-store';
import type { QueueBoardData } from '../api/generated/queue_board';
import type { KioskQueueRow } from '../queue/wire';
import { QueuePanel } from './QueuePanel';
import { meOf } from '../../test-support/me';

const T0 = Date.parse('2026-10-08T03:00:00.000Z');

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
  failNextRead: unknown = undefined;
  private holdRead = false;
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

  private releaser(): () => void {
    const index = this.held.length;
    return () => {
      const release = this.held[index];
      if (release) release();
    };
  }

  private async wait(): Promise<void> {
    if (this.holdRead) {
      this.holdRead = false;
      await new Promise<void>((resolve) => this.held.push(resolve));
    }
  }

  queueReads(): Call[] {
    return this.calls.filter((c) => c.name === 'kiosk_queue');
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
      if (name !== 'kiosk_queue') throw new Error(`unexpected view ${name}`);
      const failure = this.failNextRead;
      this.failNextRead = undefined;
      const version = b.version;
      const list = b.rows[(p.filter?.state ?? 'waiting') as State].slice(0, p.limit ?? 51);
      await this.wait();
      if (failure !== undefined) throw failure;
      if (p.since !== undefined && p.since === version) return { version, changed: false };
      return { version, changed: true, data: list as unknown as T };
    },
    page: async <R,>(name: string, params: PageParams = {}): Promise<R[]> => {
      const p = { ...params } as Call['params'];
      this.calls.push({ method: 'page', name, params: p });
      if (name !== 'kiosk_queue') throw new Error(`unexpected page ${name}`);
      await settled();
      const b = this.branch(String(p.branch));
      const failure = this.failNextRead;
      this.failNextRead = undefined;
      const all = b.rows[(p.filter?.state ?? 'waiting') as State];
      const from = p.before === undefined ? 0 : all.findIndex((r) => r.id === p.before) + 1;
      const list = all.slice(from, from + (p.limit ?? 51));
      await this.wait();
      if (failure !== undefined) throw failure;
      return list as unknown as R[];
    },
    post: async <B, R>(path: string, body: B): Promise<R> => {
      this.posts.push({ path, body });
      return { saved: true, claimed: true } as R;
    },
  };
}

function session(): SessionStore {
  const [me] = createSignal<Me | undefined>(ME);
  return { me, signOut: vi.fn(async () => {}), dispose: () => {} };
}

let branchNo = 0;
function freshBranch(): string {
  branchNo += 1;
  return `QP${branchNo}`;
}

let fake: FakeApi;
const disposers: Array<() => void> = [];

function storeOf(branch: string): QueueStore {
  return queueStore(branch, { api: fake.api as unknown as ApiClient, session: session() });
}

function mount(opts: { branch?: string; shown?: boolean } = {}) {
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
  expect(found, `button "${label}"`).toBeDefined();
  return found!;
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
  setLang('th');
});

describe('#2630: queue panel next/prev in-flight guard', () => {
  it('row 1: 51 rows, two clicks on Next before the answer -> one page read; label "Page 2"; Prev then goes back to page 1', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(...rows(b, 120));
    await flush();

    expect(text(p.host)).toContain('หน้า 1');
    const nextBtn = button(p.host, 'ถัดไป →');
    const prevBtn = button(p.host, '← ก่อนหน้า');
    expect(nextBtn.disabled).toBe(false);
    expect(prevBtn.disabled).toBe(true);

    // Hold next read
    const releaseNext = fake.holdNextRead();

    // First click on Next
    nextBtn.click();
    await flush();

    // Next and Prev buttons must be disabled while read is in flight
    expect(nextBtn.disabled, 'next button disabled while reading').toBe(true);
    expect(prevBtn.disabled, 'prev button disabled while reading').toBe(true);

    // Second click on Next before answer lands
    nextBtn.click();
    await flush();

    // Still only 1 page read initiated
    const pageReads = fake.queueReads().filter((r) => r.method === 'page');
    expect(pageReads.length, 'only one page read sent').toBe(1);

    // Answer the read
    releaseNext();
    await flush();

    // Label must be Page 2 (หน้า 2), not Page 3
    expect(text(p.host)).toContain('หน้า 2');
    expect(text(p.host)).not.toContain('หน้า 3');

    // Prev button is enabled
    expect(prevBtn.disabled).toBe(false);

    // Prev then goes back to page 1
    prevBtn.click();
    await flush();
    expect(text(p.host)).toContain('หน้า 1');
  });

  it('row 2: two clicks on Prev from page 3 while a read is in flight -> one read', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(...rows(b, 120));
    await flush();

    // Navigate to page 2
    button(p.host, 'ถัดไป →').click();
    await flush();
    expect(text(p.host)).toContain('หน้า 2');

    // Navigate to page 3
    button(p.host, 'ถัดไป →').click();
    await flush();
    expect(text(p.host)).toContain('หน้า 3');

    const prevBtn = button(p.host, '← ก่อนหน้า');
    expect(prevBtn.disabled).toBe(false);

    const prevReadsBefore = fake.queueReads().filter((r) => r.method === 'page').length;

    // Hold read
    const releasePrev = fake.holdNextRead();

    // First click Prev
    prevBtn.click();
    await flush();

    expect(prevBtn.disabled, 'prev button disabled while reading').toBe(true);

    // Second click Prev while in flight
    prevBtn.click();
    await flush();

    const prevReadsDuring = fake.queueReads().filter((r) => r.method === 'page').length;
    expect(prevReadsDuring - prevReadsBefore, 'only one read sent on double prev click').toBe(1);

    releasePrev();
    await flush();

    expect(text(p.host)).toContain('หน้า 2');
  });

  it('row 3: single Next/Prev with the answer between clicks -> unchanged (kept)', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(...rows(b, 120));
    await flush();

    expect(text(p.host)).toContain('หน้า 1');

    button(p.host, 'ถัดไป →').click();
    await flush();
    expect(text(p.host)).toContain('หน้า 2');

    button(p.host, 'ถัดไป →').click();
    await flush();
    expect(text(p.host)).toContain('หน้า 3');

    button(p.host, '← ก่อนหน้า').click();
    await flush();
    expect(text(p.host)).toContain('หน้า 2');

    button(p.host, '← ก่อนหน้า').click();
    await flush();
    expect(text(p.host)).toContain('หน้า 1');
  });

  it('edge case (§9): a failed page read leaves the buttons usable again', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(...rows(b, 120));
    await flush();

    const nextBtn = button(p.host, 'ถัดไป →');
    const prevBtn = button(p.host, '← ก่อนหน้า');
    expect(nextBtn.disabled).toBe(false);

    // Make next read fail
    fake.failNextRead = new ApiError({ code: 'serverError', message: 'Connection failed', status: 500 });

    const release = fake.holdNextRead();
    nextBtn.click();
    await flush();

    // While read is in flight, buttons are disabled
    expect(nextBtn.disabled, 'next button disabled while reading').toBe(true);
    expect(prevBtn.disabled, 'prev button disabled while reading').toBe(true);

    release();
    await flush();

    // After failure, loading is cleared and buttons are usable again
    expect(nextBtn.disabled, 'next button is not stuck disabled after failure').toBe(false);
    expect(prevBtn.disabled, 'prev button is usable after failure').toBe(false);

    // Clicking prev goes back to page 1
    prevBtn.click();
    await flush();
    expect(text(p.host)).toContain('หน้า 1');
  });
});
