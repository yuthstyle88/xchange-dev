import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor } from 'solid-js';
import { render } from 'solid-js/web';
import type { Me } from '../api/generated/common';
import type { EndReason, SessionStore } from '../session/session';
import type { QueueError, QueueStore } from '../queue/queue-store';
import type { KioskNotifications } from '../queue/wire';

interface FakeQueues {
  branch: Accessor<string>;
  setBranch(b: string): void;
  store(b: string): FakeStore;
  reset(): void;
}

interface FakeStore extends QueueStore {
  setState(v: KioskNotifications | undefined): void;
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
      const [state, setState] = solid.createSignal<KioskNotifications | undefined>(undefined);
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

const NOK: Me = { kind: 'staff', id: 'E7', role: 'FRONT', branch: 'B1', name: 'Nok', username: 'nok', branchName: 'B1', branches: [] };

let serverNow = Date.parse('2026-10-08T03:00:00.000Z');

function answer(branch: string, over: Partial<KioskNotifications> = {}) {
  serverNow += 5000;
  const s = queues().store(branch);
  s.setError(undefined);
  s.setState({
    serverNow: new Date(serverNow).toISOString(),
    branchId: branch,
    branchName: 'Silom',
    count: 0,
    latest: null,
    workingCount: 0,
    working: [],
    employee: { id: NOK.id, role: NOK.role },
    ...over,
  });
}

function pushState(branch: string, over: Partial<KioskNotifications> = {}) {
  serverNow += 5000;
  const s = queues().store(branch);
  s.setState({
    serverNow: new Date(serverNow).toISOString(),
    branchId: branch,
    branchName: 'Silom',
    count: 1,
    latest: 'kq-1',
    workingCount: 0,
    working: [],
    employee: { id: NOK.id, role: NOK.role },
    ...over,
  });
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
    answer('B1', { count: 0, latest: null });
    await settle();
    await page.sound.toggleSound();
    await settle();
    expect(audio.chimes()).toBe(1);

    // Session ends in store with expired; store retains/updates state with count > 0 and latest
    fail('B1', { kind: 'ended', reason: 'expired' });
    pushState('B1', { count: 1, latest: 'kq-1' });
    await settle();

    // No announcement chime played for kq-1
    expect(audio.chimes()).toBe(1);
    expect(storage.map.has('xc.queue.sound.E7.B1')).toBe(false);

    // Toggling the branch announces nothing
    fail('B2', { kind: 'ended', reason: 'expired' });
    pushState('B2', { count: 1, latest: 'kq-2' });
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
    answer('B1', { count: 0, latest: null });
    await settle();
    await page.sound.toggleSound();
    await settle();
    expect(audio.chimes()).toBe(1);

    // Session ends in store with switched
    fail('B1', { kind: 'ended', reason: 'switched' });
    pushState('B1', { count: 2, latest: 'kq-switched' });
    await settle();
    expect(audio.chimes()).toBe(1);
    expect(storage.map.has('xc.queue.sound.E7.B1')).toBe(false);

    // Toggling branch
    fail('B2', { kind: 'ended', reason: 'switched' });
    pushState('B2', { count: 3, latest: 'kq-b2' });
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
    answer('B1', { count: 0, latest: null });
    await settle();
    await page.sound.toggleSound();
    await settle();
    expect(audio.chimes()).toBe(1);

    // Session ended is set to switched on session, but store has not received ended error yet
    page.setEnded('switched');
    answer('B1', { count: 1, latest: 'kq-switched-session' });
    await settle();
    expect(audio.chimes()).toBe(1);
    expect(storage.map.has('xc.queue.sound.E7.B1')).toBe(false);

    // Toggling branch
    answer('B2', { count: 2, latest: 'kq-b2-session' });
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
