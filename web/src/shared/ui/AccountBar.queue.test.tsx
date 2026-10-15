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
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `Me` fixture (`kind`/`role`/`username`) → `meOf(…)` and `employee {id, role}` → `employeeOf(…)` (`web/src/test-support/me.ts`, design §3.3: FRONT → TELLER at its branch). Nothing else changes.
// The bar's line `Nok · nok · FRONT` → `Nok · <shell.account.role.teller>` (ruling (a): name, then the role
// labels held at me.branch; no username), read from the shared table, not hard-coded.
// QU-6 (#1354, rulings issuecomment-6117574257 and issuecomment-6118321787) by the test-writer claude/14cc24bd: the
// store holds `queue_board`'s `QueueBoardData` (one WAITING ticket `kq-1` is the new request). Ruling (a): a session
// without QueueOperate (RATES) gets no card at all. Nothing else changes.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor } from 'solid-js';
import { render } from 'solid-js/web';
import type { Me } from '../api/generated/common';
import type { SessionStore } from '../session/session';
import type { QueueError, QueueStore } from '../queue/queue-store';
import type { QueueBoardData, QueueBoardWaiting } from '../api/generated/queue_board';
import { meOf, roleLabel } from '../../test-support/me';
import { warmModules } from '../testing/warm-modules';

interface FakeStore extends Omit<QueueStore, 'state'> {
  state: Accessor<QueueBoardData | undefined>;
  setState(v: QueueBoardData | undefined): void;
}

const fake = vi.hoisted(() => ({ store: undefined as unknown, reset: undefined as unknown }));

vi.mock('../queue/queue-store', async () => {
  const solid = await import('solid-js');
  const [branch, setBranch] = solid.createSignal('B1');
  const stores = new Map<string, FakeStore>();
  const store = (b: string): FakeStore => {
    let s = stores.get(b);
    if (!s) {
      const [state, setState] = solid.createSignal<QueueBoardData | undefined>(undefined);
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

const NOK_V3 = { id: 'E7', role: 'FRONT', branch: 'B1', name: 'Nok', branchName: 'B1', branches: [] };
const NOK: Me = meOf(NOK_V3);
const RATES: Me = meOf({ id: 'E9', role: 'RATES', branch: 'B1', name: 'Pim', branchName: 'B1', branches: [] });

/** One WAITING ticket, issued now: `queue_board`'s `waiting` row. */
const KQ1: QueueBoardWaiting = {
  id: 'kq-1',
  displayNumber: 'W001',
  serviceType: 'WALK_IN',
  sequence: 1,
  issuedAt: '2026-10-08T03:00:05.000Z',
};

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

beforeAll(() => warmModules(() => import('./AccountBar')), 60_000);

beforeEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
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
    expect(host.textContent ?? '').toContain(`Nok · ${roleLabel('teller')}`);
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

    // QU-6 ruling (a): RATES holds no QueueOperate, so it gets no card at all (and so no link).
    await mountBar(RATES);
    expect(cards()).toEqual([]);
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
      waiting: [KQ1],
      called: [],
      myCounter: null,
    });
    await settle();
    expect(storage.get('xc.queue.sound.E7.B1')).toBe('kq-1');
    expect(FakeAudioContext.chimes).toBe(2);
  });
});
