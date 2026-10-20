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
import { fill, setLang } from '../i18n';
import { en } from '../i18n/en';
import { th } from '../i18n/th';
import { queueStore, type QueueStore } from '../queue/queue-store';
import type { SessionStore } from '../session/session';
import type { KioskQueueRow, Me } from '../testing/fixtures';
import { makeKioskQueueRow, makeQueueBoardData } from '../testing/fixtures';
import { meOf } from '../../test-support/me';
import { QueuePanel } from './QueuePanel';

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

const settled = () => Promise.resolve();

class FakeApi {
  readonly branches = new Map<string, Branch>();
  failNextRead: unknown = undefined;
  postAnswer: ((body: { payload: { id: string; branchId: string } }) => Promise<unknown>) | undefined;

  branch(id: string): Branch {
    let b = this.branches.get(id);
    if (!b) {
      b = new Branch(id);
      this.branches.set(id, b);
    }
    return b;
  }

  readonly api = {
    view: async <T,>(name: string, params: ViewParams = {}): Promise<ViewAnswer<T>> => {
      const p = { ...params } as ViewParams<{ state?: string }>;
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
      if (failure !== undefined) throw failure;
      const version = b.version;
      const list = b.rows[(p.filter?.state ?? 'waiting') as State].slice(0, p.limit ?? 51);
      if (p.since !== undefined && p.since === version) {
        return { version, changed: false };
      }
      return { version, changed: true, data: list as unknown as T };
    },
    page: async <R,>(name: string, params: PageParams = {}): Promise<R[]> => {
      const p = { ...params } as PageParams<{ state?: string }>;
      if (name !== 'kiosk_queue') throw new Error(`unexpected page ${name}`);
      await settled();
      const b = this.branch(String(p.branch));
      const failure = this.failNextRead;
      this.failNextRead = undefined;
      if (failure !== undefined) throw failure;
      const all = b.rows[(p.filter?.state ?? 'waiting') as State];
      const from = p.before === undefined ? 0 : all.findIndex((r) => r.id === p.before) + 1;
      return all.slice(from, from + (p.limit ?? 51)) as unknown as R[];
    },
    post: async <B, R>(_path: string, body: B): Promise<R> => {
      const accept = body as unknown as { payload: { id: string; branchId: string } };
      if (this.postAnswer) return (await this.postAnswer(accept)) as R;
      return { saved: true, claimed: true } as R;
    },
  };
}

function session(): SessionStore {
  const [me] = createSignal<Me | undefined>(ME);
  return { me, signOut: vi.fn(async () => {}), dispose: () => {} };
}

let branchNo = 5000;
function freshBranch(): string {
  branchNo += 1;
  return `QP-WE-${branchNo}`;
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

describe('shared/ui/QueuePanel wordedError (WE-46 #1795)', () => {
  describe('Catch 1: readQueue failure', () => {
    it("(1) a network TypeError shows th['error.network'], and after setLang('en') shows en['error.network']", async () => {
      fake.failNextRead = new TypeError('Failed to fetch');
      const p = mount();
      await flush();

      expect(message(p.host)).toBe(
        fill(th['queue.status.loadFailed'], { message: th['error.network'] }),
      );
      expect(message(p.host)).not.toContain('Failed to fetch');

      setLang('en');
      await flush();
      expect(message(p.host)).toBe(
        fill(en['queue.status.loadFailed'], { message: en['error.network'] }),
      );
      expect(message(p.host)).not.toContain('Failed to fetch');

      fake.failNextRead = new TypeError('Failed to fetch');
      button(p.host, en['queue.tab.reload']).click();
      await flush();
      expect(message(p.host)).toBe(
        fill(en['queue.status.loadFailed'], { message: en['error.network'] }),
      );
      expect(message(p.host)).not.toContain('Failed to fetch');
    });

    it("(2) a 500 refusal with a keyed non-server code shows th['error.serverError']", async () => {
      fake.failNextRead = new ApiError({
        code: 'roleNotAllowedHere',
        message: 'raw 500 read error',
        status: 500,
      });
      const p = mount();
      await flush();

      expect(message(p.host)).toBe(
        fill(th['queue.status.loadFailed'], { message: th['error.serverError'] }),
      );
      expect(message(p.host)).not.toContain(th['error.roleNotAllowedHere']);
      expect(message(p.host)).not.toContain('raw 500 read error');
    });

    it('(3) a refusal with an unkeyed code and server message shows that message, and a keyed code shows the shared key text', async () => {
      const serverSentence = 'คิวของสาขานี้ปิดปรับปรุงชั่วคราว';
      fake.failNextRead = new ApiError({
        code: 'unkeyedQueueReadCode',
        message: serverSentence,
        status: 400,
      });
      const p = mount();
      await flush();

      expect(message(p.host)).toBe(
        fill(th['queue.status.loadFailed'], { message: serverSentence }),
      );
      expect(message(p.host)).not.toContain(th['error.serverError']);

      // Keyed non-5xx refusal must map through errorText (goes red on revert to raw err.message)
      fake.failNextRead = new ApiError({
        code: 'roleNotAllowedHere',
        message: 'raw role wire error',
        status: 403,
      });
      button(p.host, th['queue.tab.reload']).click();
      await flush();

      expect(message(p.host)).toBe(
        fill(th['queue.status.loadFailed'], { message: th['error.roleNotAllowedHere'] }),
      );
      expect(message(p.host)).not.toContain('raw role wire error');
    });
  });

  describe('Catch 2: claim failure', () => {
    it("(1) a network TypeError shows th['error.network'], and after setLang('en') shows en['error.network']", async () => {
      const p = mount();
      const b = p.branch();
      p.data().rows.waiting.push(row(b, 1));
      await flush();

      fake.postAnswer = async () => {
        throw new TypeError('Failed to fetch');
      };
      button(card(p.host, `${b}-Q1`), th['queue.action.claim']).click();
      await flush();

      expect(message(p.host)).toBe(th['error.network']);
      expect(message(p.host)).not.toContain('Failed to fetch');

      setLang('en');
      await flush();
      expect(message(p.host)).toBe(en['error.network']);
      expect(message(p.host)).not.toContain('Failed to fetch');

      button(card(p.host, `${b}-Q1`), en['queue.action.claim']).click();
      await flush();
      expect(message(p.host)).toBe(en['error.network']);
      expect(message(p.host)).not.toContain('Failed to fetch');
    });

    it("(2) a 500 refusal with a keyed non-server code shows th['error.serverError']", async () => {
      const p = mount();
      const b = p.branch();
      p.data().rows.waiting.push(row(b, 1));
      await flush();

      fake.postAnswer = async () => {
        throw new ApiError({
          code: 'roleNotAllowedHere',
          message: 'raw 500 claim error',
          status: 500,
        });
      };
      button(card(p.host, `${b}-Q1`), th['queue.action.claim']).click();
      await flush();

      expect(message(p.host)).toBe(th['error.serverError']);
      expect(message(p.host)).not.toBe(th['error.roleNotAllowedHere']);
      expect(message(p.host)).not.toContain('raw 500 claim error');
    });

    it('(3) a refusal with an unkeyed code and server message shows that message, and a keyed code shows the shared key text', async () => {
      const serverSentence = 'ออเดอร์นี้มีพนักงานท่านอื่นรับเรื่องแล้ว';
      const p = mount();
      const b = p.branch();
      p.data().rows.waiting.push(row(b, 1));
      await flush();

      fake.postAnswer = async () => {
        throw new ApiError({
          code: 'unkeyedQueueClaimCode',
          message: serverSentence,
          status: 400,
        });
      };
      button(card(p.host, `${b}-Q1`), th['queue.action.claim']).click();
      await flush();

      expect(message(p.host)).toBe(serverSentence);
      expect(message(p.host)).not.toBe(th['error.serverError']);

      // Keyed non-5xx refusal must map through errorText (goes red on revert to raw err.message)
      fake.postAnswer = async () => {
        throw new ApiError({
          code: 'roleNotAllowedHere',
          message: 'raw claim role wire error',
          status: 403,
        });
      };
      button(card(p.host, `${b}-Q1`), th['queue.action.claim']).click();
      await flush();

      expect(message(p.host)).toBe(th['error.roleNotAllowedHere']);
      expect(message(p.host)).not.toContain('raw claim role wire error');
    });
  });

  describe('§4.3: stale data isolation across branch switch', () => {
    it('clears previous branch rows when switching to a branch whose read fails', async () => {
      const first = freshBranch();
      fake.branch(first).rows.waiting.push(row(first, 1));
      const p = mount({ branch: first });
      await flush();

      expect(articles(p.host)).toHaveLength(1);
      expect(text(p.host)).toContain(`${first}-Q1`);

      const second = freshBranch();
      fake.failNextRead = new ApiError({
        code: 'roleNotAllowedHere',
        message: 'raw branch switch 500',
        status: 500,
      });
      p.setBranch(second);
      await flush();

      expect(articles(p.host)).toHaveLength(0);
      expect(text(p.host)).not.toContain(`${first}-Q1`);
      expect(message(p.host)).toBe(
        fill(th['queue.status.loadFailed'], { message: th['error.serverError'] }),
      );
    });
  });
});
