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
import { th } from '../i18n/th';
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

  readonly api = {
    view: async <T,>(name: string, params: ViewParams = {}): Promise<ViewAnswer<T>> => {
      const p = { ...params } as Call['params'];
      this.calls.push({ method: 'view', name, params: p });
      await settled();
      const b = this.branch(String(p.branch));
      if (name === 'queue_board') {
        if (p.since !== undefined && p.since === b.version) {
          return { version: b.version, changed: false };
        }
        return { version: b.version, changed: true, data: b.board() as unknown as T };
      }
      if (name !== 'kiosk_queue') throw new Error(`unexpected view ${name}`);
      const failure = this.failNextRead;
      this.failNextRead = undefined;
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
  return `QP-TAB-${branchNo}`;
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

describe('QueuePanel tab switch & empty-state guards (#2354)', () => {
  it('row 1: Waiting tab has rows; click the working tab while its read is held -> 0 cards; the loading text; no empty text', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(...rows(b, 2));
    await flush();
    expect(shownQueues(p.host)).toEqual([`${b}-Q1`, `${b}-Q2`]);

    // Click working tab while its read is held
    const releaseTabRead = fake.holdNextRead();
    button(p.host, 'กำลังทำรายการ').click();
    await flush();

    expect(pressed(p.host)).toEqual(['กำลังทำรายการ']);
    expect(articles(p.host)).toHaveLength(0);
    expect(message(p.host)).toBe('กำลังโหลดคิว…');
    expect(text(p.host)).not.toContain('ไม่มีรายการในหมวดนี้');
    expect(p.host.querySelector('.queue-empty')).toBeNull();

    releaseTabRead();
    await flush();

    // Also verify the claim success path clears waiting rows before switching to working while working read is held
    button(p.host, 'รอรับเรื่อง').click();
    await flush();
    expect(shownQueues(p.host)).toEqual([`${b}-Q1`, `${b}-Q2`]);

    const releaseClaimRead = fake.holdNextRead();
    button(card(p.host, `${b}-Q1`), 'รับเรื่องนี้').click();
    await flush();

    expect(pressed(p.host)).toEqual(['กำลังทำรายการ']);
    expect(articles(p.host)).toHaveLength(0);
    expect(message(p.host)).toBe('กำลังโหลดคิว…');
    expect(text(p.host)).not.toContain('ไม่มีรายการในหมวดนี้');

    releaseClaimRead();
    await flush();
    expect(shownQueues(p.host)).toEqual([`${b}-Q1`]);
  });

  it('row 2: Waiting tab has rows; switch tab and that read fails -> 0 cards from the waiting tab; the load error; no empty text', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(...rows(b, 2));
    await flush();
    expect(shownQueues(p.host)).toEqual([`${b}-Q1`, `${b}-Q2`]);

    fake.failNextRead = new ApiError({ code: 'queueDown', message: 'boom', status: 502 });
    button(p.host, 'กำลังทำรายการ').click();
    await flush();

    expect(pressed(p.host)).toEqual(['กำลังทำรายการ']);
    expect(articles(p.host)).toHaveLength(0);
    expect(message(p.host)).toBe(`โหลดคิวไม่ได้: ${th['error.serverError']}`);
    expect(text(p.host)).not.toContain('ไม่มีรายการในหมวดนี้');
    expect(p.host.querySelector('.queue-empty')).toBeNull();
  });

  it('row 3: First mount with the read held, or the first read failing -> No empty text', async () => {
    // 1. First mount with the read held
    const releaseFirst = fake.holdNextRead();
    const p1 = mount();
    await flush();

    expect(articles(p1.host)).toHaveLength(0);
    expect(message(p1.host)).toBe('กำลังโหลดคิว…');
    expect(text(p1.host)).not.toContain('ไม่มีรายการในหมวดนี้');
    expect(p1.host.querySelector('.queue-empty')).toBeNull();

    releaseFirst();
    await flush();
    // Once the empty list answers OK, the empty text is shown
    expect(text(p1.host)).toContain('ไม่มีรายการในหมวดนี้');

    // 2. First mount with the first read failing
    fake.failNextRead = new ApiError({ code: 'queueDown', message: 'boom', status: 502 });
    const p2 = mount();
    await flush();

    expect(articles(p2.host)).toHaveLength(0);
    expect(message(p2.host)).toBe(`โหลดคิวไม่ได้: ${th['error.serverError']}`);
    expect(text(p2.host)).not.toContain('ไม่มีรายการในหมวดนี้');
    expect(p2.host.querySelector('.queue-empty')).toBeNull();
  });

  it('row 4: Tab reads page 1 without since and shows only working rows', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(...rows(b, 60));
    p.data().rows.working.push(
      row(b, 900, { status: 'ACCEPTED', canAccept: false, staff: 'malee' }),
    );
    await flush();

    button(p.host, 'ถัดไป →').click();
    await flush();
    expect(text(p.host)).toContain('หน้า 2');

    const releaseWorking = fake.holdNextRead();
    button(p.host, 'กำลังทำรายการ').click();
    await flush();

    // Waiting rows are cleared immediately upon switching tabs while page 1 of working is read
    expect(articles(p.host)).toHaveLength(0);
    expect(text(p.host)).not.toContain('ไม่มีรายการในหมวดนี้');

    releaseWorking();
    await flush();

    const last = fake.queueReads().at(-1)!;
    expect(last.method).toBe('view');
    expect(last.params).toMatchObject({ branch: b, limit: 51, filter: { state: 'working' } });
    expect(last.params.since).toBeUndefined();
    expect(last.params.before).toBeUndefined();
    expect(pressed(p.host)).toEqual(['กำลังทำรายการ']);
    expect(shownQueues(p.host)).toEqual([`${b}-Q900`]);
    expect(text(p.host)).toContain('หน้า 1');
  });

  it('row 5: A failed reload on the SAME tab keeps its rows, buttons disabled, hrefs removed', async () => {
    const p = mount();
    const b = p.branch();
    p.data().rows.waiting.push(
      row(b, 1),
      row(b, 2, { status: 'ACCEPTED', staff: 'somchai', canAccept: false, canContinue: true }),
    );
    await flush();
    expect(card(p.host, `${b}-Q2`).querySelector('a')!.hasAttribute('href')).toBe(true);

    // Failed reload on the SAME tab keeps rows with disabled buttons and removed hrefs, and no empty text
    fake.failNextRead = new ApiError({ code: 'queueDown', message: 'boom', status: 502 });
    button(p.host, 'โหลดล่าสุด').click();
    await flush();

    expect(message(p.host)).toBe(`โหลดคิวไม่ได้: ${th['error.serverError']}`);
    const inList = articles(p.host);
    expect(inList).toHaveLength(2);
    for (const a of inList) {
      for (const btn of a.querySelectorAll('button')) expect(btn.disabled).toBe(true);
      for (const link of a.querySelectorAll('a')) expect(link.hasAttribute('href')).toBe(false);
    }

    // Switching to another tab whose read fails clears the previous tab's rows and shows no empty text
    fake.failNextRead = new ApiError({ code: 'queueDown', message: 'boom', status: 502 });
    button(p.host, 'เสร็จสิ้น').click();
    await flush();

    expect(articles(p.host)).toHaveLength(0);
    expect(message(p.host)).toBe(`โหลดคิวไม่ได้: ${th['error.serverError']}`);
    expect(text(p.host)).not.toContain('ไม่มีรายการในหมวดนี้');
  });
});
