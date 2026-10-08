// V1-queue-card (#487): the account bar renders the queue card, design
// docs/design/xchange-v3-web.md §12.18.1 KQ3 and §12.18.6 (v2: the card on every page with `BAR`,
// src/server.mjs:97, :106-108, :128-130 of xchange-v2-source@4d0bdbb). Written before the
// implementation by the lane controller. Do not edit, delete or weaken; add tests in a file of your
// own. AccountBar.test.tsx (#402) is not changed and must stay green.
//
// `AccountBar` renders `<QueueCard session={session()} />` inside its `<Show when={user()}>`
// (AccountBar.tsx:43-53), so the card exists only while someone is signed in, and it reads the
// bar's own session. The queue store (#486) is replaced with a fake, as in QueueCard.test.tsx.
//
// The card is not inside the bar's own element: AccountBar.test.tsx's "role alone" row asserts that
// the bar's host has no `·`, and the card's title `XC · …` has one — v2 appended the card to
// `document.body`, apart from the bar (account-bar.js:31).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor } from 'solid-js';
import { render } from 'solid-js/web';
import type { Me } from '../api/generated/common';
import type { SessionStore } from '../session/session';
import type { QueueError, QueueStore } from '../queue/queue-store';
import type { KioskNotifications } from '../queue/wire';

interface FakeStore extends QueueStore {
  setState(v: KioskNotifications | undefined): void;
}

const fake = vi.hoisted(() => ({ store: undefined as unknown, reset: undefined as unknown }));

vi.mock('../queue/queue-store', async () => {
  const solid = await import('solid-js');
  const [branch, setBranch] = solid.createSignal('B1');
  const stores = new Map<string, FakeStore>();
  const store = (b: string): FakeStore => {
    let s = stores.get(b);
    if (!s) {
      const [state, setState] = solid.createSignal<KioskNotifications | undefined>(undefined);
      const [error] = solid.createSignal<QueueError | undefined>(undefined);
      const [lastUpdated] = solid.createSignal<number | undefined>(undefined);
      const [version] = solid.createSignal<number | undefined>(undefined);
      s = {
        state,
        error,
        lastUpdated,
        version,
        now: () => Date.now(),
        refresh: async () => {},
        setState: (v) => setState(() => v),
      };
      stores.set(b, s);
    }
    return s;
  };
  fake.store = store;
  fake.reset = () => {
    stores.clear();
    setBranch('B1');
  };
  return {
    queueBranch: branch,
    setQueueBranch: (b: string) => setBranch(b),
    queueStore: (b: string) => store(b),
    useQueueStore: (b: Accessor<string>) => () => store(b()),
  };
});

class FakeAudioContext {
  static chimes = 0;
  state = 'suspended';
  currentTime = 1;
  readonly destination = {};
  async resume() {
    this.state = 'running';
  }
  createOscillator() {
    return {
      frequency: {
        set value(v: number) {
          if (v === 660) FakeAudioContext.chimes += 1;
        },
        get value() {
          return 0;
        },
      },
      connect() {},
      start() {},
      stop() {},
    };
  }
  createGain() {
    return {
      gain: { setValueAtTime() {}, linearRampToValueAtTime() {}, exponentialRampToValueAtTime() {} },
      connect() {},
    };
  }
}

const storage = new Map<string, string>();
const fakeStorage = {
  get length() {
    return storage.size;
  },
  key: (i: number) => [...storage.keys()][i] ?? null,
  getItem: (k: string) => (storage.has(k) ? (storage.get(k) as string) : null),
  setItem: (k: string, v: string) => void storage.set(k, String(v)),
  removeItem: (k: string) => void storage.delete(k),
  clear: () => storage.clear(),
};

const ARIA = 'แจ้งเตือนคิวลูกค้า';
const OPEN = 'เปิดคิว / รับเรื่อง →';

const NOK: Me = { kind: 'staff', id: 'E7', role: 'FRONT', branch: 'B1', name: 'Nok', username: 'nok' };
const RATES: Me = { kind: 'staff', id: 'E9', role: 'RATES', branch: 'B1', name: 'Pim', username: 'pim' };

const disposers: Array<() => void> = [];

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

async function mountBar(initial: Me | undefined) {
  vi.resetModules();
  const { AccountBar } = await import('./AccountBar');
  // the fake exists once the bar imports the queue store (through QueueCard)
  if (typeof fake.reset === 'function') (fake.reset as () => void)();
  const [me, setMe] = createSignal<Me | undefined>(initial);
  const calls = { signOut: 0 };
  const session: SessionStore = {
    me,
    signOut: async () => {
      calls.signOut += 1;
    },
    dispose: () => {},
  };
  const host = document.createElement('div');
  document.body.appendChild(host);
  disposers.push(render(() => <AccountBar session={session} navigate={() => {}} />, host));
  await settle();
  return { host, setMe: (m: Me | undefined) => setMe(() => m), calls };
}

function cards(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('aside')).filter(
    (a) => a.getAttribute('aria-label') === ARIA,
  );
}

function linkIn(root: Element, text: string): HTMLAnchorElement | undefined {
  return Array.from(root.querySelectorAll('a')).find((a) => (a.textContent ?? '').trim() === text);
}

function shown(el: Element | null | undefined): boolean {
  if (!el || !el.isConnected) return false;
  for (let e: Element | null = el; e; e = e.parentElement) {
    if (e.hasAttribute('hidden')) return false;
    if (getComputedStyle(e).display === 'none') return false;
  }
  return true;
}

beforeEach(() => {
  storage.clear();
  FakeAudioContext.chimes = 0;
  vi.stubGlobal('localStorage', fakeStorage);
  Object.defineProperty(window, 'localStorage', { value: fakeStorage, configurable: true });
  vi.stubGlobal('AudioContext', FakeAudioContext);
  Object.defineProperty(window, 'AudioContext', { value: FakeAudioContext, configurable: true, writable: true });
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
});

describe('AccountBar renders the queue card (KQ3)', () => {
  it('a signed-in user: one card on the page, outside the bar’s own element', async () => {
    const { host } = await mountBar(NOK);
    expect(cards()).toHaveLength(1);
    expect(host.contains(cards()[0])).toBe(false);
    expect(host.textContent ?? '').not.toContain('XC ·');
    expect(host.textContent ?? '').toContain('Nok · nok · FRONT');
  });

  it('nobody signed in: no card; it appears on sign-in and goes when me() becomes undefined', async () => {
    const { setMe } = await mountBar(undefined);
    expect(cards()).toEqual([]);
    setMe(NOK);
    await settle();
    expect(cards()).toHaveLength(1);
    setMe(undefined);
    await settle();
    expect(cards()).toEqual([]);
  });

  it('the card reads the bar’s session: FRONT gets the queue link, RATES does not', async () => {
    const front = await mountBar(NOK);
    expect(cards()).toHaveLength(1);
    const a = linkIn(cards()[0], OPEN);
    expect(shown(a)).toBe(true);
    expect(a?.getAttribute('href')).toBe('/kiosk-queue.html?branch=B1');
    expect(front.calls.signOut).toBe(0);

    while (disposers.length) disposers.pop()?.();
    document.body.innerHTML = '';

    await mountBar(RATES);
    expect(cards()).toHaveLength(1);
    expect(shown(linkIn(cards()[0], OPEN))).toBe(false);
  });

  it('the sound key carries the bar’s user: xc.queue.sound.<me().id>.<branch>', async () => {
    await mountBar(NOK);
    expect(cards()).toHaveLength(1);
    const sound = Array.from(cards()[0].querySelectorAll('button')).find(
      (b) => (b.textContent ?? '').trim() === 'เปิดเสียงแจ้งเตือน',
    );
    sound?.click();
    await settle();
    (fake.store as (b: string) => FakeStore)('B1').setState({
      serverNow: '2026-10-08T03:00:05.000Z',
      branchId: 'B1',
      branchName: 'Silom',
      count: 1,
      latest: 'kq-1',
      workingCount: 0,
      working: [],
      employee: { id: NOK.id, role: NOK.role },
    });
    await settle();
    expect(storage.get('xc.queue.sound.E7.B1')).toBe('kq-1');
    expect(FakeAudioContext.chimes).toBe(2);
  });
});
