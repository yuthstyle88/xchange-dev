// V1-kiosk-queue-panel (#486): the shared queue panel, design docs/design/xchange-v3-web.md §12.18.4
// rows P1–P14 (v2: src/devices-public/queue-panel.js:1-39 of xchange-v2-source@4d0bdbb), and the
// shared `queue.*` texts of §12.18.7 (P1–P12, C1–C9 and account-bar.js:45's store-error sentence).
// Written before the implementation by the lane controller. Do not edit, delete or weaken; add
// tests in a file of your own.
//
// The panel runs against one fake `ApiClient`, passed both as the panel's `api` prop and as the
// store's `api` (`queueStore(branch, { api, session })` is created first, so the panel finds it):
// it answers `kiosk_queue` and (the store's poll) `queue_board` like the server — a `since` equal to the
// branch version gets `changed: false`, `before` starts strictly after that row. Rows are shown
// in v2's shape: an `<article>` per row, the times in a `<dl>` of `<dt>`/`<dd>`, the message in
// `role="status"`. Amounts are decimal strings the panel shows through MoneyText (W7).
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `Me` fixture (`kind`/`role`/`username`) → `meOf(…)` and `employee {id, role}` → `employeeOf(…)` (`web/src/test-support/me.ts`, design §3.3: FRONT → TELLER at its branch). Nothing else changes.
// QU-6 (#1354, rulings issuecomment-6117574257 and issuecomment-6118321787) by the test-writer claude/14cc24bd: the
// shared store polls `queue_board` (the panel itself stays on `kiosk_queue`), so the fake answers the store's poll as
// `queue_board` (`QueueBoardData`: one WAITING ticket per waiting request) and counts those polls as `boardReads()`.
// `queue_board` carries no `serverNow`, so the store runs on this browser's clock. Ruling (b): P8 ticks on the client
// clock; for the same reason the store's `lastUpdated` in P2/P3 and the P14 stamp are the client clock (`T0`), not
// `T0 + SKEW`. The panel's own `kiosk_queue` rows are still stamped by the server (`SKEW`). Nothing else changes.
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
import { collectScreens, setLang } from '../i18n';
import { en } from '../i18n/en';
import { th } from '../i18n/th';
import type { SessionStore } from '../session/session';
import { queueStore, type QueueStore } from '../queue/queue-store';
import type { QueueBoardData } from '../api/generated/queue_board';
import type { KioskQueueRow } from '../queue/wire';
import { QueuePanel } from './QueuePanel';
import { meOf } from '../../test-support/me';

// ── fixtures ─────────────────────────────────────────────────────────────────

const T0 = Date.parse('2026-10-08T03:00:00.000Z');
/** The server's clock runs ten minutes ahead of this browser's. */
const SKEW = 10 * 60 * 1000;
const THAI = /[฀-๿]/;

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

/**
 * The server reads its state one microtask after the call, so rows a test adds right after
 * mounting (in the same synchronous step) are in the first answer.
 */
const settled = () => Promise.resolve();

/** One fake `ApiClient` for every branch a test uses. */
class FakeApi {
  readonly branches = new Map<string, Branch>();
  readonly calls: Call[] = [];
  readonly posts: Array<{ path: string; body: unknown }> = [];
  /** The next kiosk_queue read is refused with this. */
  failNextRead: unknown = undefined;
  /** What the next accept answers; by default the server claims the row. */
  postAnswer: ((body: { payload: { id: string; branchId: string } }) => Promise<unknown>) | undefined;
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

  /** The next kiosk_queue read (view or page) stays in flight until the returned function is called. */
  holdNextRead(): () => void {
    this.holdRead = true;
    return this.releaser();
  }

  /** The next post stays in flight until the returned function is called. */
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

  queueReads(): Call[] {
    return this.calls.filter((c) => c.name === 'kiosk_queue');
  }

  boardReads(): Call[] {
    return this.calls.filter((c) => c.name === 'queue_board');
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
      // the answer is what the server had when it was asked
      const version = b.version;
      const list = b.rows[(p.filter?.state ?? 'waiting') as State].slice(0, p.limit ?? 51);
      await this.wait('read');
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
      await this.wait('read');
      if (failure !== undefined) throw failure;
      return list as unknown as R[];
    },
    post: async <B, R>(path: string, body: B): Promise<R> => {
      this.posts.push({ path, body });
      await this.wait('post');
      const accept = body as unknown as { payload: { id: string; branchId: string } };
      if (this.postAnswer) return (await this.postAnswer(accept)) as R;
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

let branchNo = 0;
function freshBranch(): string {
  branchNo += 1;
  return `QP${branchNo}`;
}

let fake: FakeApi;
const disposers: Array<() => void> = [];

/** The store of `branch`, created with the fake before the panel asks for it. */
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
  return text(host.querySelector('[role="status"]'));
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

/** The `<dd>` beside the `<dt>` that reads `label`. */
function term(article: Element, label: string): string {
  const dt = [...article.querySelectorAll('dt')].find((d) => text(d) === label);
  if (!dt) throw new Error(`no <dt> "${label}" in: ${text(article)}`);
  return text(dt.nextElementSibling);
}

function stamp(ms: number | string, loc = 'th-TH'): string {
  return new Date(ms).toLocaleString(loc, { timeZone: 'Asia/Bangkok' });
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
  setLang('th');
});

// ── P1, P13: tabs and what makes the panel read ──────────────────────────────

describe('P1: tabs and reload', () => {
  it('shows the three tabs and โหลดล่าสุด; รอรับเรื่อง is pressed first and read with state waiting, limit 51, branch', async () => {
    const p = mount();
    p.data().rows.waiting.push(...rows(p.branch(), 2));
    await flush();

    for (const label of ['รอรับเรื่อง', 'กำลังทำรายการ', 'เสร็จสิ้น', 'โหลดล่าสุด']) button(p.host, label);
    expect(pressed(p.host)).toEqual(['รอรับเรื่อง']);
    expect(button(p.host, 'กำลังทำรายการ').getAttribute('aria-pressed')).toBe('false');
    expect(button(p.host, 'เสร็จสิ้น').getAttribute('aria-pressed')).toBe('false');

    const first = fake.queueReads()[0]!;
    expect(first.method).toBe('view');
    expect(first.params).toMatchObject({ branch: p.branch(), limit: 51, filter: { state: 'waiting' } });
    expect(first.params.since).toBeUndefined();
    expect(shownQueues(p.host)).toEqual([`${p.branch()}-Q1`, `${p.branch()}-Q2`]);
  });

  it('a tab reads its state from page 1 without since, and moves aria-pressed', async () => {
    const p = mount();
    p.data().rows.waiting.push(...rows(p.branch(), 60));
    p.data().rows.working.push(row(p.branch(), 900, { status: 'ACCEPTED', canAccept: false, staff: 'malee' }));
    await flush();
    button(p.host, 'ถัดไป →').click();
    await flush();
    expect(text(p.host)).toContain('หน้า 2');

    button(p.host, 'กำลังทำรายการ').click();
    await flush();
    const last = fake.queueReads().at(-1)!;
    expect(last.method).toBe('view');
    expect(last.params).toMatchObject({ branch: p.branch(), limit: 51, filter: { state: 'working' } });
    expect(last.params.since).toBeUndefined();
    expect(last.params.before).toBeUndefined();
    expect(pressed(p.host)).toEqual(['กำลังทำรายการ']);
    expect(shownQueues(p.host)).toEqual([`${p.branch()}-Q900`]);
    expect(text(p.host)).toContain('หน้า 1');
  });

  it('โหลดล่าสุด reads again', async () => {
    const p = mount();
    p.data().rows.waiting.push(...rows(p.branch(), 1));
    await flush();
    const before = fake.queueReads().length;
    button(p.host, 'โหลดล่าสุด').click();
    await flush();
    expect(fake.queueReads().length).toBe(before + 1);
  });
});

describe('P13: the panel reads when shown, on a change, and when the store version moves — it has no timer of its own', () => {
  it('a store poll that answers unchanged makes no read and redraws nothing', async () => {
    const p = mount();
    p.data().rows.waiting.push(...rows(p.branch(), 3));
    await flush();
    const reads = fake.queueReads().length;
    const nodes = articles(p.host);
    const polls = fake.boardReads().length;

    await vi.advanceTimersByTimeAsync(15_000);
    expect(fake.boardReads().length).toBe(polls + 3);
    expect(fake.queueReads().length).toBe(reads);
    expect(articles(p.host)).toEqual(nodes);
    articles(p.host).forEach((a, i) => expect(a).toBe(nodes[i]));
  });

  it('when the store version moves while shown, page 1 is read again with since = the last answer, and shows the change', async () => {
    const p = mount();
    p.data().rows.waiting.push(...rows(p.branch(), 2));
    await flush();
    const reads = fake.queueReads().length;

    p.data().rows.waiting.unshift(row(p.branch(), 77));
    p.data().version = 8;
    await vi.advanceTimersByTimeAsync(5_000);
    await flush();

    const again = fake.queueReads().slice(reads);
    expect(again).toHaveLength(1);
    expect(again[0]!.method).toBe('view');
    expect(again[0]!.params).toMatchObject({ branch: p.branch(), limit: 51, filter: { state: 'waiting' }, since: 7 });
    expect(shownQueues(p.host)[0]).toBe(`${p.branch()}-Q77`);
  });

  it('on a later page a version move re-reads that page by its before cursor, without since', async () => {
    const p = mount();
    p.data().rows.waiting.push(...rows(p.branch(), 60));
    await flush();
    button(p.host, 'ถัดไป →').click();
    await flush();
    const reads = fake.queueReads().length;

    p.data().version = 8;
    await vi.advanceTimersByTimeAsync(5_000);
    await flush();
    const again = fake.queueReads().slice(reads);
    expect(again).toHaveLength(1);
    expect(again[0]!.method).toBe('page');
    expect(again[0]!.params).toMatchObject({ branch: p.branch(), before: `${p.branch()}-kq-50`, limit: 51 });
    expect(again[0]!.params.since).toBeUndefined();
  });

  it('not shown: no read, not even when the version moves; becoming shown reads, and the latest rows appear', async () => {
    const p = mount({ shown: false });
    p.data().rows.waiting.push(...rows(p.branch(), 1));
    await flush();
    expect(fake.queueReads()).toHaveLength(0);

    p.data().rows.waiting.unshift(row(p.branch(), 5));
    p.data().version = 8;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(fake.queueReads()).toHaveLength(0);

    p.setShown(true);
    await flush();
    expect(fake.queueReads()).toHaveLength(1);
    expect(shownQueues(p.host)).toEqual([`${p.branch()}-Q5`, `${p.branch()}-Q1`]);

    p.setShown(false);
    await flush();
    const reads = fake.queueReads().length;
    p.data().version = 9;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(fake.queueReads()).toHaveLength(reads);
  });

  it('a branch change clears the list at once and reads the new branch from page 1 without since', async () => {
    const p = mount();
    p.data().rows.waiting.push(...rows(p.branch(), 60));
    await flush();
    button(p.host, 'ถัดไป →').click();
    await flush();

    const other = freshBranch();
    fake.branch(other).rows.waiting.push(...rows(other, 2));
    const release = fake.holdNextRead();
    p.setBranch(other);
    await flush();
    expect(articles(p.host)).toHaveLength(0);
    const last = fake.queueReads().at(-1)!;
    expect(last.method).toBe('view');
    expect(last.params).toMatchObject({ branch: other, limit: 51, filter: { state: 'waiting' } });
    expect(last.params.since).toBeUndefined();

    release();
    await flush();
    expect(shownQueues(p.host)).toEqual([`${other}-Q1`, `${other}-Q2`]);
    expect(text(p.host)).toContain('หน้า 1');
  });

  it("an answer for the branch the panel left is dropped when it arrives late", async () => {
    const first = freshBranch();
    fake.branch(first).rows.waiting.push(...rows(first, 2));
    const release = fake.holdNextRead();
    const p = mount({ branch: first });
    await flush();

    const other = freshBranch();
    fake.branch(other).rows.waiting.push(...rows(other, 1));
    p.setBranch(other);
    await flush();
    expect(shownQueues(p.host)).toEqual([`${other}-Q1`]);

    release();
    await flush();
    expect(shownQueues(p.host)).toEqual([`${other}-Q1`]);
  });

  it('an older read that answers after a newer one is dropped', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.working.push(row(b, 1, { status: 'ACCEPTED', canAccept: false }));
    p.data().rows.completed.push(row(b, 2, { status: 'COMPLETED', canAccept: false }));
    await flush();

    const release = fake.holdNextRead();
    button(p.host, 'กำลังทำรายการ').click();
    await flush();
    button(p.host, 'เสร็จสิ้น').click();
    await flush();
    expect(shownQueues(p.host)).toEqual([`${b}-Q2`]);

    release();
    await flush();
    expect(shownQueues(p.host)).toEqual([`${b}-Q2`]);
    expect(pressed(p.host)).toEqual(['เสร็จสิ้น']);
  });
});

// ── P2, P3: the message line and the empty list ──────────────────────────────

describe('P2, P3: the status line', () => {
  it('reads กำลังโหลดคิว… while a forced read runs, then อัปเดต <the store lastUpdated> · …', async () => {
    const release = fake.holdNextRead();
    const p = mount();
    await flush();
    expect(message(p.host)).toBe('กำลังโหลดคิว…');

    release();
    await flush();
    const updated = p.store().lastUpdated();
    expect(updated).toBe(T0);
    expect(message(p.host)).toBe(`อัปเดต ${stamp(updated!)} · เวลานับตั้งแต่รับเรื่องจนบันทึกสำเร็จ`);

    const again = fake.holdNextRead();
    button(p.host, 'โหลดล่าสุด').click();
    await flush();
    expect(message(p.host)).toBe('กำลังโหลดคิว…');
    again();
    await flush();
    expect(message(p.host)).toContain('อัปเดต ');
  });

  it("the time follows the store's lastUpdated: an unchanged poll 5 s later moves it", async () => {
    const p = mount();
    await flush();
    await vi.advanceTimersByTimeAsync(5_000);
    await flush();
    expect(p.store().lastUpdated()).toBe(T0 + 5_000);
    expect(message(p.host)).toBe(
      `อัปเดต ${stamp(T0 + 5_000)} · เวลานับตั้งแต่รับเรื่องจนบันทึกสำเร็จ`,
    );
  });

  it('an empty list reads ไม่มีรายการในหมวดนี้', async () => {
    const p = mount();
    await flush();
    expect(articles(p.host)).toHaveLength(0);
    expect(text(p.host)).toContain('ไม่มีรายการในหมวดนี้');
  });
});

// ── P4–P8: one card per row ──────────────────────────────────────────────────

describe('P4–P8: a card per row', () => {
  it('P4: the queue is the heading, then the state word: WAITING, COMPLETED, else กำลังทำรายการ', async () => {
    const p = mount();
    const b = p.branch();
    // canContinue rows carry no holder line, so กำลังทำรายการ in them can only be the state word;
    // every card has the label เสร็จสิ้น once (P7), so a COMPLETED card has it twice
    p.data().rows.waiting.push(
      row(b, 1, { status: 'WAITING' }),
      row(b, 2, { status: 'COMPLETED', canAccept: false }),
      row(b, 3, { status: 'ACCEPTED', canAccept: false, canContinue: true }),
      row(b, 4, { status: 'ID_CHECKED', canAccept: false, canContinue: true }),
    );
    await flush();
    expect(shownQueues(p.host)).toEqual([`${b}-Q1`, `${b}-Q2`, `${b}-Q3`, `${b}-Q4`]);
    const words = (queue: string) => {
      const all = text(card(p.host, queue));
      const count = (word: string) => all.split(word).length - 1;
      return [count('รอรับเรื่อง'), count('กำลังทำรายการ'), count('เสร็จสิ้น')];
    };
    expect(words(`${b}-Q1`)).toEqual([1, 0, 1]);
    expect(words(`${b}-Q2`)).toEqual([0, 0, 2]);
    expect(words(`${b}-Q3`)).toEqual([0, 1, 1]);
    expect(words(`${b}-Q4`)).toEqual([0, 1, 1]);
  });

  it('P5, P6: side, both amounts as the view sends them (W7), order and transaction', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(
      row(b, 1, { type: 'BUY', foreign: '1234.5600', currency: 'USD', thb: '41037.1234', transactionNo: 'TX-0001' }),
      row(b, 2, { type: 'SELL', foreign: '0.10', currency: 'JPY', thb: '1000000.00' }),
    );
    await flush();
    const buy = text(card(p.host, `${b}-Q1`));
    expect(buy).toContain('ร้านรับซื้อ · 1,234.5600 USD → 41,037.1234 THB');
    expect(buy).toContain('ออเดอร์ ORD-1 · Transaction TX-0001');
    const sell = text(card(p.host, `${b}-Q2`));
    expect(sell).toContain('ร้านขาย · 0.10 JPY → 1,000,000.00 THB');
    expect(sell).toContain('ออเดอร์ ORD-2 · Transaction ยังไม่บันทึก');
  });

  it('P7: who, start and finish, each with its fallback', async () => {
    const p = mount();
    const b = p.branch();
    const started = '2026-10-08T02:10:00.000Z';
    const done = '2026-10-08T02:25:30.000Z';
    p.data().rows.waiting.push(
      row(b, 1),
      row(b, 2, { status: 'COMPLETED', staff: 'malee', startedAt: started, completedAt: done, durationSeconds: 930, canAccept: false }),
      row(b, 3, { status: 'COMPLETED', staff: 'malee', canAccept: false }),
      row(b, 4, { status: 'ACCEPTED', staff: 'malee', startedAt: started, canAccept: false }),
    );
    await flush();

    const waiting = card(p.host, `${b}-Q1`);
    expect(term(waiting, 'ผู้รับผิดชอบ')).toBe('ยังไม่มีผู้รับเรื่อง');
    expect(term(waiting, 'เริ่มทำรายการ')).toBe('ยังไม่มีข้อมูล');
    expect(term(waiting, 'เสร็จสิ้น')).toBe('—');

    const finished = card(p.host, `${b}-Q2`);
    expect(term(finished, 'ผู้รับผิดชอบ')).toBe('malee');
    expect(term(finished, 'เริ่มทำรายการ')).toBe(stamp(started));
    expect(term(finished, 'เสร็จสิ้น')).toBe(stamp(done));

    const old = card(p.host, `${b}-Q3`);
    expect(term(old, 'เสร็จสิ้น')).toBe('ไม่มีเวลาย้อนหลัง');
    expect(term(old, 'เริ่มทำรายการ')).toBe('ยังไม่มีข้อมูล');

    expect(term(card(p.host, `${b}-Q4`), 'เสร็จสิ้น')).toBe('—');
  });

  it("P8: durationSeconds as h/m/s; a started row without it ticks every 1 s on the store's clock; otherwise ยังไม่มีข้อมูล", async () => {
    const p = mount();
    const b = p.branch();
    // ruling (b): `queue_board` carries no `serverNow`, so the store's clock is this browser's
    const clockNow = T0;
    p.data().rows.waiting.push(
      row(b, 1, { status: 'COMPLETED', startedAt: '2026-10-08T01:00:00.000Z', completedAt: '2026-10-08T02:02:05.000Z', durationSeconds: 3725, canAccept: false }),
      row(b, 2, { status: 'ACCEPTED', staff: 'malee', startedAt: new Date(clockNow - 65_000).toISOString(), canAccept: false }),
      row(b, 3, { status: 'COMPLETED', startedAt: '2026-10-08T01:00:00.000Z', canAccept: false }),
      row(b, 4),
    );
    await flush();
    expect(term(card(p.host, `${b}-Q1`), 'ระยะเวลาดำเนินการ')).toBe('1 ชม. 2 นาที 5 วินาที');
    expect(term(card(p.host, `${b}-Q2`), 'ระยะเวลาดำเนินการ')).toBe('0 ชม. 1 นาที 5 วินาที (กำลังดำเนินการ)');
    expect(term(card(p.host, `${b}-Q3`), 'ระยะเวลาดำเนินการ')).toBe('ยังไม่มีข้อมูล');
    expect(term(card(p.host, `${b}-Q4`), 'ระยะเวลาดำเนินการ')).toBe('ยังไม่มีข้อมูล');

    await vi.advanceTimersByTimeAsync(1_000);
    expect(term(card(p.host, `${b}-Q2`), 'ระยะเวลาดำเนินการ')).toBe('0 ชม. 1 นาที 6 วินาที (กำลังดำเนินการ)');
    await vi.advanceTimersByTimeAsync(2_000);
    expect(term(card(p.host, `${b}-Q2`), 'ระยะเวลาดำเนินการ')).toBe('0 ชม. 1 นาที 8 วินาที (กำลังดำเนินการ)');
    expect(term(card(p.host, `${b}-Q1`), 'ระยะเวลาดำเนินการ')).toBe('1 ชม. 2 นาที 5 วินาที');
  });
});

// ── P9, P10: claiming and continuing ─────────────────────────────────────────

describe('P9: รับเรื่องนี้', () => {
  it('posts accept with the id and the branch, switches to กำลังทำรายการ page 1, refreshes the store and calls onClaimed', async () => {
    const onClaimed = vi.fn();
    const p = mount({ onClaimed });
    const b = p.branch();
    p.data().rows.waiting.push(row(b, 1), row(b, 2, { canAccept: false }));
    await flush();
    expect([...card(p.host, `${b}-Q2`).querySelectorAll('button')].map(text)).not.toContain('รับเรื่องนี้');
    const polls = fake.boardReads().length;

    button(card(p.host, `${b}-Q1`), 'รับเรื่องนี้').click();
    await flush();

    expect(fake.posts).toEqual([{ path: '/api/kiosk', body: { action: 'accept', payload: { id: `${b}-kq-1`, branchId: b } } }]);
    expect(pressed(p.host)).toEqual(['กำลังทำรายการ']);
    expect(fake.queueReads().some((c) => c.params.filter?.state === 'working' && c.params.before === undefined)).toBe(true);
    expect(shownQueues(p.host)).toEqual([`${b}-Q1`]);
    expect(text(p.host)).toContain('หน้า 1');
    expect(fake.boardReads().length).toBeGreaterThan(polls);
    expect(onClaimed).toHaveBeenCalledWith(`${b}-kq-1`);
  });

  it('one claim at a time: a second รับเรื่องนี้ while the first runs sends nothing', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(row(b, 1), row(b, 2));
    await flush();

    const release = fake.holdNextPost();
    button(card(p.host, `${b}-Q1`), 'รับเรื่องนี้').click();
    await flush();
    button(card(p.host, `${b}-Q2`), 'รับเรื่องนี้').click();
    await flush();
    expect(fake.posts).toHaveLength(1);

    release();
    await flush();
    expect(fake.posts).toHaveLength(1);
  });

  it("a refusal shows the server's sentence, reads again, and the sentence stays after the read", async () => {
    const sentence = 'ออเดอร์นี้มี malee กำลังทำรายการแล้ว';
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(row(b, 1));
    await flush();
    fake.postAnswer = async () => {
      throw new ApiError({ code: 'refused', message: sentence, status: 400 });
    };
    const reads = fake.queueReads().length;

    button(card(p.host, `${b}-Q1`), 'รับเรื่องนี้').click();
    await flush();
    expect(fake.queueReads().length).toBeGreaterThan(reads);
    expect(message(p.host)).toBe(sentence);
    expect(pressed(p.host)).toEqual(['รอรับเรื่อง']);
  });

  it('a claim that succeeds after the branch changed switches no tab, and still refreshes the store', async () => {
    const p = mount();
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
    const polls = fake.boardReads().length;

    release();
    await flush();
    expect(fake.posts[0]!.body).toEqual({ action: 'accept', payload: { id: `${b}-kq-1`, branchId: b } });
    expect(pressed(p.host)).toEqual(['รอรับเรื่อง']);
    expect(shownQueues(p.host)).toEqual([`${other}-Q1`]);
    expect(fake.boardReads().length).toBeGreaterThan(polls);
  });
});

describe('P10: ทำรายการต่อ → or who holds it', () => {
  it('canContinue links to /desk with branch and kiosk; ACCEPTED / ID_CHECKED otherwise name the holder; others show neither', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(
      row(b, 1, { id: 'kq/1 a', status: 'ACCEPTED', staff: 'somchai', canAccept: false, canContinue: true }),
      row(b, 2, { status: 'ACCEPTED', staff: 'malee', canAccept: false }),
      row(b, 3, { status: 'ID_CHECKED', staff: 'malee', canAccept: false }),
      row(b, 4, { status: 'COMPLETED', staff: 'malee', canAccept: false }),
    );
    await flush();

    const link = [...card(p.host, `${b}-Q1`).querySelectorAll('a')].find((a) => text(a) === 'ทำรายการต่อ →');
    expect(link).toBeDefined();
    const url = new URL(link!.getAttribute('href')!, 'http://xc.test');
    expect(url.pathname).toBe('/desk');
    expect(url.searchParams.get('branch')).toBe(b);
    expect(url.searchParams.get('kiosk')).toBe('kq/1 a');
    expect(text(card(p.host, `${b}-Q1`))).not.toContain('กำลังทำรายการนี้');

    expect(text(card(p.host, `${b}-Q2`))).toContain('malee กำลังทำรายการนี้');
    expect(text(card(p.host, `${b}-Q3`))).toContain('malee กำลังทำรายการนี้');
    expect(card(p.host, `${b}-Q2`).querySelector('a')).toBeNull();
    expect(text(card(p.host, `${b}-Q4`))).not.toContain('กำลังทำรายการนี้');
    expect(card(p.host, `${b}-Q4`).querySelector('a')).toBeNull();
  });
});

// ── P11: the pager ───────────────────────────────────────────────────────────

describe('P11: pager over a before cursor', () => {
  it('50 rows: page 1, back and next both disabled', async () => {
    const p = mount();
    p.data().rows.waiting.push(...rows(p.branch(), 50));
    await flush();
    expect(articles(p.host)).toHaveLength(50);
    expect(text(p.host)).toContain('หน้า 1');
    expect(button(p.host, '← ก่อนหน้า').disabled).toBe(true);
    expect(button(p.host, 'ถัดไป →').disabled).toBe(true);
  });

  it('51 rows: 50 shown and next enabled; pages go forward by before and back by the cursor stack', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(...rows(b, 120));
    await flush();
    expect(articles(p.host)).toHaveLength(50);
    expect(shownQueues(p.host).at(-1)).toBe(`${b}-Q50`);
    expect(button(p.host, 'ถัดไป →').disabled).toBe(false);

    button(p.host, 'ถัดไป →').click();
    await flush();
    let last = fake.queueReads().at(-1)!;
    expect(last.method).toBe('page');
    expect(last.params).toEqual({ branch: b, before: `${b}-kq-50`, limit: 51, filter: { state: 'waiting' } });
    expect(shownQueues(p.host)[0]).toBe(`${b}-Q51`);
    expect(articles(p.host)).toHaveLength(50);
    expect(text(p.host)).toContain('หน้า 2');
    expect(button(p.host, '← ก่อนหน้า').disabled).toBe(false);

    button(p.host, 'ถัดไป →').click();
    await flush();
    last = fake.queueReads().at(-1)!;
    expect(last.params.before).toBe(`${b}-kq-100`);
    expect(shownQueues(p.host)).toHaveLength(20);
    expect(text(p.host)).toContain('หน้า 3');
    expect(button(p.host, 'ถัดไป →').disabled).toBe(true);

    button(p.host, '← ก่อนหน้า').click();
    await flush();
    last = fake.queueReads().at(-1)!;
    expect(last.params.before).toBe(`${b}-kq-50`);
    expect(shownQueues(p.host)[0]).toBe(`${b}-Q51`);
    expect(text(p.host)).toContain('หน้า 2');

    button(p.host, '← ก่อนหน้า').click();
    await flush();
    expect(fake.queueReads().at(-1)!.params.before).toBeUndefined();
    expect(shownQueues(p.host)[0]).toBe(`${b}-Q1`);
    expect(articles(p.host)).toHaveLength(50);
    expect(text(p.host)).toContain('หน้า 1');
    expect(button(p.host, '← ก่อนหน้า').disabled).toBe(true);
  });
});

// ── P12: a failed read ───────────────────────────────────────────────────────

describe('P12: a failed read', () => {
  it('shows โหลดคิวไม่ได้: <message>, disables every button in the list and removes every link href', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(
      row(b, 1),
      row(b, 2, { status: 'ACCEPTED', staff: 'somchai', canAccept: false, canContinue: true }),
    );
    await flush();
    expect(card(p.host, `${b}-Q2`).querySelector('a')!.hasAttribute('href')).toBe(true);

    fake.failNextRead = new ApiError({ code: 'queueDown', message: 'boom', status: 502 });
    button(p.host, 'โหลดล่าสุด').click();
    await flush();
    expect(message(p.host)).toBe('โหลดคิวไม่ได้: boom');
    const inList = articles(p.host);
    expect(inList).toHaveLength(2);
    for (const a of inList) {
      for (const btn of a.querySelectorAll('button')) expect(btn.disabled).toBe(true);
      for (const link of a.querySelectorAll('a')) expect(link.hasAttribute('href')).toBe(false);
    }
  });
});

// ── P14: the other language ──────────────────────────────────────────────────

describe('P14: lang en', () => {
  it('every text of the panel comes from en, and times use en-GB in Asia/Bangkok', async () => {
    setLang('en');
    const p = mount();
    const b = p.branch();
    const started = '2026-10-08T02:10:00.000Z';
    p.data().rows.waiting.push(
      row(b, 1),
      row(b, 2, { type: 'SELL', status: 'COMPLETED', staff: 'malee', startedAt: started, completedAt: null, durationSeconds: 61, canAccept: false }),
      row(b, 3, { status: 'ACCEPTED', staff: 'malee', startedAt: started, canAccept: false, canContinue: true }),
      row(b, 4, { status: 'ID_CHECKED', staff: 'malee', canAccept: false }),
    );
    await flush();

    expect(text(p.host)).not.toMatch(THAI);
    expect(text(card(p.host, `${b}-Q2`))).toContain(stamp(started, 'en-GB'));
    expect(text(card(p.host, `${b}-Q2`))).not.toContain(stamp(started));
    const labels = [...p.host.querySelectorAll('button')].map((b2) => text(b2));
    for (const thai of ['รอรับเรื่อง', 'กำลังทำรายการ', 'เสร็จสิ้น', 'โหลดล่าสุด']) {
      expect(labels.some((l) => enOf([thai]).includes(l)), thai).toBe(true);
    }
    expect(message(p.host)).toContain(stamp(T0, 'en-GB'));

    fake.failNextRead = new ApiError({ code: 'queueDown', message: 'boom', status: 502 });
    const reload = labels.find((l) => enOf(['โหลดล่าสุด']).includes(l))!;
    button(p.host, reload).click();
    await flush();
    expect(message(p.host)).toContain('boom');
    expect(text(p.host)).not.toMatch(THAI);
  });
});

/** The en texts of the queue.* keys whose th text is one of `thai`. */
function enOf(thai: string[]): string[] {
  const table = th as Record<string, string>;
  const english = en as Record<string, string>;
  return Object.keys(table)
    .filter((k) => k.startsWith('queue.') && thai.includes(table[k]!))
    .map((k) => english[k]!);
}

// ── §12.18.7: the shared queue.* texts ───────────────────────────────────────

/**
 * Can the queue.* th texts say `sentence`? Either one text is the sentence, or a text with
 * `{placeholders}` matches it and every Thai part a placeholder took can itself be said. The
 * sentences below carry ASCII sample data where v2 put a number, a name or a time.
 */
function sayable(sentence: string, depth = 0): boolean {
  if (depth > 3) return false;
  const table = th as Record<string, string>;
  for (const [key, value] of Object.entries(table)) {
    if (!key.startsWith('queue.')) continue;
    if (value === sentence) return true;
    if (!/\{\w+\}/.test(value)) continue;
    const pattern = new RegExp(
      `^${value
        .split(/\{\w+\}/)
        .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
        .join('(.+)')}$`,
    );
    const m = sentence.match(pattern);
    if (!m) continue;
    if (m.slice(1).every((part) => !THAI.test(part) || sayable(part, depth + 1))) return true;
  }
  return false;
}

const QUEUE_SENTENCES: Record<string, string[]> = {
  'P1, P4 tabs and state words': ['รอรับเรื่อง', 'กำลังทำรายการ', 'เสร็จสิ้น', 'โหลดล่าสุด'],
  'P2 status line': ['กำลังโหลดคิว…', 'อัปเดต 10:00:00 · เวลานับตั้งแต่รับเรื่องจนบันทึกสำเร็จ'],
  'P3 empty': ['ไม่มีรายการในหมวดนี้'],
  'P5 side': ['ร้านรับซื้อ', 'ร้านขาย'],
  'P6 references': ['ออเดอร์ ORD-1 · Transaction TX-1', 'ออเดอร์ ORD-1 · Transaction ยังไม่บันทึก'],
  'P7 times': ['ผู้รับผิดชอบ', 'ยังไม่มีผู้รับเรื่อง', 'เริ่มทำรายการ', 'ยังไม่มีข้อมูล', 'ไม่มีเวลาย้อนหลัง'],
  'P8 duration': ['ระยะเวลาดำเนินการ', '1 ชม. 2 นาที 5 วินาที'],
  'P9 claim': ['รับเรื่องนี้'],
  'P10 continue': ['ทำรายการต่อ →', 'malee กำลังทำรายการนี้'],
  'P11 pager': ['← ก่อนหน้า', 'หน้า 2', 'ถัดไป →'],
  'P12 failed read': ['โหลดคิวไม่ได้: boom'],
  'C1 card': ['แจ้งเตือนคิวลูกค้า', 'XC · คิวหน้าร้าน', 'กำลังตรวจคิว…', 'ย่อการแจ้งเตือน'],
  'C3 link': ['เปิดคิว / รับเรื่อง →'],
  'C4 answer': ['มีคิวลูกค้ารอ 3 รายการ · Silom', 'ไม่มีคิวลูกค้ารอ · Silom', 'กำลังตรวจคิวของสาขา…'],
  'C5 offline': ['เชื่อมต่อไม่ได้ · กำลังลองใหม่'],
  'C6 ended': [
    'เซสชันสิ้นสุด กรุณาเข้าสู่ระบบใหม่',
    'บัญชีเปลี่ยนในหน้าต่างอื่น กรุณาเข้าสู่ระบบใหม่ก่อนทำรายการ',
    'เข้าสู่ระบบอีกครั้ง →',
  ],
  'C7 sound': [
    'เปิดเสียงแจ้งเตือน',
    'ทดสอบเสียง',
    'กดเปิดเสียงครั้งแรกเพื่อให้เบราว์เซอร์อนุญาต',
    'เสียงเปิด · กดปิด',
    'แจ้งเตือนคิวใหม่ · ลดเสียงซ้ำระหว่างแท็บ',
    'ปิดเสียงแล้ว ยังแสดงการแจ้งเตือน',
    'เปิดเสียงไม่ได้ กรุณาตรวจการตั้งค่าเสียงเบราว์เซอร์',
    'เสียงทดสอบ · การแจ้งเตือนเปิดอยู่',
    'เสียงทดสอบ · กดเปิดเสียงเพื่อรับแจ้งเตือนคิว',
    'เล่นเสียงไม่ได้ กรุณาตรวจลำโพงและเบราว์เซอร์',
  ],
  'store error (account-bar.js:45)': ['เชื่อมต่อคิวไม่ได้ · กำลังลองใหม่'],
};

describe('§12.18.7: the shared queue.* texts', () => {
  it.each(Object.entries(QUEUE_SENTENCES))('th says v2 verbatim: %s', (_rows, sentences) => {
    for (const sentence of sentences) expect(sayable(sentence), sentence).toBe(true);
  });

  it('the running suffix (กำลังดำเนินการ) is sayable, alone or after a duration', () => {
    expect(sayable('(กำลังดำเนินการ)') || sayable('0 ชม. 1 นาที 5 วินาที (กำลังดำเนินการ)')).toBe(true);
  });

  it('every queue.* key has an en text with no Thai in it', () => {
    const keys = Object.keys(th).filter((k) => k.startsWith('queue.'));
    expect(keys.length).toBeGreaterThan(0);
    for (const key of keys) {
      const value = (en as Record<string, string>)[key];
      expect(value, key).toBeTruthy();
      expect(value, key).not.toMatch(THAI);
    }
  });

  it('queue is a shared prefix: a screen folder named queue is refused (SHARED_PREFIXES, index.ts:10)', () => {
    expect(() =>
      collectScreens({ '/src/screens/queue/i18n.ts': { th: { a: 'ก' }, en: { a: 'a' } } }),
    ).toThrow(/shared prefix/);
  });
});
