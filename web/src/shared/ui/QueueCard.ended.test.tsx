import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `Me` fixture (`kind`/`role`/`username`) → `meOf(…)` and `employee {id, role}` → `employeeOf(…)` (`web/src/test-support/me.ts`, design §3.3: FRONT → TELLER at its branch). Nothing else changes.
// QU-6 (#1354, ruling issuecomment-6118321787) by the test-writer claude/14cc24bd: the store holds `queue_board`'s
// `QueueBoardData`; each answer's count/latest is re-expressed as that many WAITING tickets, the newest with that id.
// Nothing else changes.
import { createSignal, type Accessor } from 'solid-js';
import { render } from 'solid-js/web';
import type { Me } from '../api/generated/common';
import type { EndReason, SessionStore } from '../session/session';
import type { QueueError, QueueStore } from '../queue/queue-store';
import type { QueueBoardData, QueueBoardWaiting } from '../api/generated/queue_board';
import { meOf } from '../../test-support/me';

interface FakeQueues {
  branch: Accessor<string>;
  setBranch(b: string): void;
  store(b: string): FakeStore;
  reset(): void;
}

interface FakeStore extends Omit<QueueStore, 'state'> {
  state: Accessor<QueueBoardData | undefined>;
  setState(v: QueueBoardData | undefined): void;
  setError(v: QueueError | undefined): void;
  refreshes: number;
}

const fake = vi.hoisted(() => ({ queues: undefined as unknown }));

vi.mock('../queue/queue-store', async () => {
  const solid = await import('solid-js');
  const [branch, setBranch] = solid.createSignal('B1');
  const stores = new Map<string, FakeStore>();
  const store = (b: string): FakeStore => {
    let s = stores.get(b);
    if (!s) {
      const [state, setState] = solid.createSignal<QueueBoardData | undefined>(undefined);
      const [error, setError] = solid.createSignal<QueueError | undefined>(undefined);
      const [lastUpdated] = solid.createSignal<number | undefined>(undefined);
      const [version] = solid.createSignal<number | undefined>(undefined);
      const created: FakeStore = {
        state,
        error,
        lastUpdated,
        version,
        now: () => Date.now(),
        refreshes: 0,
        async refresh() {
          created.refreshes += 1;
        },
        setState: (v) => setState(() => v),
        setError: (v) => setError(() => v),
      };
      s = created;
      stores.set(b, s);
    }
    return s;
  };
  const queues: FakeQueues = {
    branch,
    setBranch: (b) => setBranch(b),
    store,
    reset() {
      stores.clear();
      setBranch('B1');
    },
  };
  fake.queues = queues;
  return {
    queueBranch: branch,
    setQueueBranch: (b: string) => setBranch(b),
    queueStore: (b: string) => store(b),
    useQueueStore: (b: Accessor<string>) => () => store(b()),
  };
});

function queues(): FakeQueues {
  return fake.queues as FakeQueues;
}

const NOK_V3 = { id: 'E7', role: 'FRONT', branch: 'B1', name: 'Nok', branchName: 'B1', branches: [] };
const NOK: Me = meOf(NOK_V3);

/**
 * `count` WAITING tickets in call order (`queue_board`'s `waiting`); the newest one (the latest `issuedAt`) has the id
 * `latest`, so it is the card's "new request" key. The older ones keep the same ids from one answer to the next.
 */
function waiting(count: number, latest: string | null): QueueBoardWaiting[] {
  const list: QueueBoardWaiting[] = [];
  for (let i = 1; i <= count; i += 1) {
    const newest = i === count;
    list.push({
      id: newest && latest !== null ? latest : `older-${i}`,
      displayNumber: `W${String(i).padStart(3, '0')}`,
      serviceType: 'WALK_IN',
      sequence: i,
      issuedAt: new Date(Date.parse('2026-10-08T02:00:00.000Z') + i * 60_000).toISOString(),
    });
  }
  return list;
}

function answer(branch: string, over: Partial<QueueBoardData> = {}) {
  const s = queues().store(branch);
  s.setError(undefined);
  const data: QueueBoardData = { waiting: [], called: [], myCounter: null, ...over };
  s.setState(data);
}

function pushState(branch: string, over: Partial<QueueBoardData> = {}) {
  const s = queues().store(branch);
  const data: QueueBoardData = { waiting: waiting(1, 'kq-1'), called: [], myCounter: null, ...over };
  s.setState(data);
}

function fail(branch: string, error: QueueError) {
  queues().store(branch).setError(error);
}

const audio = {
  resumeTo: 'running',
  contexts: [] as FakeAudioContext[],
  frequencies: [] as number[],
  chimes(): number {
    return audio.frequencies.filter((f) => f === 660).length;
  },
};

class FakeAudioContext {
  state = 'suspended';
  currentTime = 1;
  readonly destination = {};
  constructor() {
    audio.contexts.push(this);
  }
  async resume() {
    this.state = audio.resumeTo;
  }
  createOscillator() {
    const osc = {
      frequency: {
        set value(v: number) {
          audio.frequencies.push(v);
        },
        get value() {
          return audio.frequencies.at(-1) ?? 0;
        },
      },
      connect() {},
      start() {},
      stop() {},
    };
    return osc;
  }
  createGain() {
    return {
      gain: { setValueAtTime() {}, linearRampToValueAtTime() {}, exponentialRampToValueAtTime() {} },
      connect() {},
    };
  }
}

const locks = {
  async request<T>(_name: string, cb: () => T | Promise<T>): Promise<T> {
    await Promise.resolve();
    return cb();
  },
};

class FakeStorage {
  readonly map = new Map<string, string>();
  get length() {
    return this.map.size;
  }
  key(i: number) {
    return [...this.map.keys()][i] ?? null;
  }
  getItem(k: string) {
    return this.map.has(k) ? (this.map.get(k) as string) : null;
  }
  setItem(k: string, v: string) {
    this.map.set(k, String(v));
  }
  removeItem(k: string) {
    this.map.delete(k);
  }
  clear() {
    this.map.clear();
  }
}

let storage: FakeStorage;

const disposers: Array<() => void> = [];

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

interface Page {
  setEnded(r: EndReason | undefined): void;
  sound: typeof import('../queue/queue-sound');
}

async function openPage(initialEnded?: EndReason): Promise<Page> {
  vi.resetModules();
  const card = await import('./QueueCard');
  const sound = await import('../queue/queue-sound');
  queues().reset();
  const [meSig] = createSignal<Me | undefined>(NOK);
  const [endedSig, setEnded] = createSignal<EndReason | undefined>(initialEnded);
  const session: SessionStore = {
    me: meSig,
    ended: endedSig,
    signOut: async () => {},
    dispose: () => {},
  };
  const host = document.createElement('div');
  document.body.appendChild(host);
  disposers.push(render(() => <card.QueueCard session={session} />, host));
  return { setEnded, sound };
}

beforeEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  audio.resumeTo = 'running';
  audio.contexts = [];
  audio.frequencies = [];
  storage = new FakeStorage();
  vi.stubGlobal('localStorage', storage);
  Object.defineProperty(window, 'localStorage', { value: storage, configurable: true });
  vi.stubGlobal('AudioContext', FakeAudioContext);
  Object.defineProperty(window, 'AudioContext', { value: FakeAudioContext, configurable: true, writable: true });
  Object.defineProperty(navigator, 'locks', { value: locks, configurable: true });
  vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 503 })));
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  for (const name of ['AudioContext', 'localStorage']) {
    if (Object.getOwnPropertyDescriptor(window, name)?.configurable) {
      delete (window as unknown as Record<string, unknown>)[name];
    }
  }
  delete (navigator as unknown as Record<string, unknown>).locks;
});

describe('QueueCard: no chimes after session has ended (#498)', () => {
  it('store error expired: toggling sound or branch announces nothing', async () => {
    const page = await openPage();
    await settle();
    // Turn sound on first with no items waiting
    answer('B1', { waiting: waiting(0, null) });
    await settle();
    await page.sound.toggleSound();
    await settle();
    expect(audio.chimes()).toBe(1);

    // Session ends in store with expired; store retains/updates state with count > 0 and latest
    fail('B1', { kind: 'ended', reason: 'expired' });
    pushState('B1', { waiting: waiting(1, 'kq-1') });
    await settle();

    // No announcement chime played for kq-1
    expect(audio.chimes()).toBe(1);
    expect(storage.map.has('xc.queue.sound.E7.B1')).toBe(false);

    // Toggling the branch announces nothing
    fail('B2', { kind: 'ended', reason: 'expired' });
    pushState('B2', { waiting: waiting(1, 'kq-2') });
    queues().setBranch('B2');
    await settle();
    expect(audio.chimes()).toBe(1);
    expect(storage.map.has('xc.queue.sound.E7.B2')).toBe(false);

    // Toggling sound off and on announces nothing
    await page.sound.toggleSound(); // off
    await settle();
    await page.sound.toggleSound(); // on: plays 1 toggle chime
    await settle();
    expect(audio.chimes()).toBe(2);
    expect(storage.map.has('xc.queue.sound.E7.B1')).toBe(false);
    expect(storage.map.has('xc.queue.sound.E7.B2')).toBe(false);
  });

  it('store error switched: toggling sound or branch announces nothing', async () => {
    const page = await openPage();
    await settle();
    answer('B1', { waiting: waiting(0, null) });
    await settle();
    await page.sound.toggleSound();
    await settle();
    expect(audio.chimes()).toBe(1);

    // Session ends in store with switched
    fail('B1', { kind: 'ended', reason: 'switched' });
    pushState('B1', { waiting: waiting(2, 'kq-switched') });
    await settle();
    expect(audio.chimes()).toBe(1);
    expect(storage.map.has('xc.queue.sound.E7.B1')).toBe(false);

    // Toggling branch
    fail('B2', { kind: 'ended', reason: 'switched' });
    pushState('B2', { waiting: waiting(3, 'kq-b2') });
    queues().setBranch('B2');
    await settle();
    expect(audio.chimes()).toBe(1);
    expect(storage.map.has('xc.queue.sound.E7.B2')).toBe(false);

    // Toggling sound
    await page.sound.toggleSound();
    await settle();
    await page.sound.toggleSound();
    await settle();
    expect(audio.chimes()).toBe(2);
    expect(storage.map.has('xc.queue.sound.E7.B1')).toBe(false);
    expect(storage.map.has('xc.queue.sound.E7.B2')).toBe(false);
  });

  it('session.ended set to switched while store not yet ended: announces nothing', async () => {
    const page = await openPage();
    await settle();
    answer('B1', { waiting: waiting(0, null) });
    await settle();
    await page.sound.toggleSound();
    await settle();
    expect(audio.chimes()).toBe(1);

    // Session ended is set to switched on session, but store has not received ended error yet
    page.setEnded('switched');
    answer('B1', { waiting: waiting(1, 'kq-switched-session') });
    await settle();
    expect(audio.chimes()).toBe(1);
    expect(storage.map.has('xc.queue.sound.E7.B1')).toBe(false);

    // Toggling branch
    answer('B2', { waiting: waiting(2, 'kq-b2-session') });
    queues().setBranch('B2');
    await settle();
    expect(audio.chimes()).toBe(1);
    expect(storage.map.has('xc.queue.sound.E7.B2')).toBe(false);

    // Toggling sound
    await page.sound.toggleSound();
    await settle();
    await page.sound.toggleSound();
    await settle();
    expect(audio.chimes()).toBe(2);
    expect(storage.map.has('xc.queue.sound.E7.B1')).toBe(false);
    expect(storage.map.has('xc.queue.sound.E7.B2')).toBe(false);
  });
});
