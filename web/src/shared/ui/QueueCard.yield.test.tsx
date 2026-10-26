// #5133 Q1–Q4: while a form holds a yield (queue-card-yield.ts), the floating queue card folds; when the yield
// ends it returns to the fold state it had, unless the user pressed −/+ during the yield, whose choice then stands.
// Harness copied from QueueCard.space.test.tsx (#5126).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor } from 'solid-js';
import { render } from 'solid-js/web';
import type { SessionStore } from '../session/session';
import type { QueueError, QueueStore } from '../queue/queue-store';
import { type Me, type QueueBoardData } from '../testing/fixtures';
import { meOf } from '../../test-support/me';
import { QueueCard } from './QueueCard';
import { yieldQueueCard } from './queue-card-yield';

interface FakeStore extends Omit<QueueStore, 'state'> {
  state: Accessor<QueueBoardData | undefined>;
  setState(v: QueueBoardData | undefined): void;
  setError(v: QueueError | undefined): void;
}

const fake = vi.hoisted(() => ({ stores: new Map<string, unknown>() }));

vi.mock('../queue/queue-store', async () => {
  const solid = await import('solid-js');
  const [branch, setBranch] = solid.createSignal('B1');
  const store = (b: string): FakeStore => {
    let s = fake.stores.get(b) as FakeStore | undefined;
    if (!s) {
      const [state, setState] = solid.createSignal<QueueBoardData | undefined>(undefined);
      const [error, setError] = solid.createSignal<QueueError | undefined>(undefined);
      const [lastUpdated] = solid.createSignal<number | undefined>(undefined);
      const [version] = solid.createSignal<number | undefined>(undefined);
      s = {
        state,
        error,
        lastUpdated,
        version,
        now: () => Date.now(),
        async refresh() {},
        setState: (v) => setState(() => v),
        setError: (v) => setError(() => v),
      };
      fake.stores.set(b, s);
    }
    return s;
  };
  return {
    queueBranch: branch,
    setQueueBranch: (b: string) => setBranch(b),
    queueStore: (b: string) => store(b),
    useQueueStore: (b: Accessor<string>) => () => store(b()),
  };
});

const NOK_V3 = { id: 'E7', role: 'FRONT', branch: 'B1', name: 'Nok', branchName: 'B1', branches: [] };
const NOK: Me = meOf(NOK_V3);

let activeCallback: ResizeObserverCallback | undefined;
let observedElements: Element[] = [];

class MockResizeObserver {
  callback: ResizeObserverCallback;
  constructor(cb: ResizeObserverCallback) {
    this.callback = cb;
    activeCallback = cb;
  }
  observe(el: Element) {
    observedElements.push(el);
  }
  unobserve(el: Element) {
    observedElements = observedElements.filter((e) => e !== el);
  }
  disconnect() {
    observedElements = [];
    if (activeCallback === this.callback) {
      activeCallback = undefined;
    }
  }
}

const disposers: Array<() => void> = [];

function mountCard(user?: Me | null) {
  const me = user === null ? undefined : (user ?? NOK);
  const [meSig, setMe] = createSignal<Me | undefined>(me);
  const session: SessionStore = {
    me: meSig,
    signOut: async () => {},
    dispose: () => {},
  };
  const host = document.createElement('div');
  document.body.appendChild(host);
  const dispose = render(() => <QueueCard session={session} />, host);
  disposers.push(dispose);
  return { host, setMe, dispose };
}

beforeEach(() => {
  fake.stores.clear();
  observedElements = [];
  activeCallback = undefined;
  vi.stubGlobal('ResizeObserver', MockResizeObserver);
});

const holds: Array<() => void> = [];
function hold(): () => void {
  const release = yieldQueueCard();
  holds.push(release);
  return release;
}

afterEach(() => {
  for (const release of holds.splice(0)) release();
  for (const d of disposers.splice(0)) d();
  document.body.innerHTML = '';
  document.documentElement.style.removeProperty('--xc-queue-card-space');
  vi.unstubAllGlobals();
});

function card(): HTMLElement {
  const el = document.querySelector<HTMLElement>('.queue-card');
  expect(el, 'the queue card').not.toBeNull();
  return el!;
}
const folded = () => card().classList.contains('minimized');
const toggle = () => card().querySelector<HTMLButtonElement>('.queue-card-toggle')!;
async function settle() {
  for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('#5133 the queue card folds while a form holds a yield', () => {
  it('Q1: card unfolded, a yield starts → folded, toggle aria-expanded="false"', async () => {
    mountCard();
    await settle();
    expect(folded(), 'starts unfolded (jsdom has no 700px match)').toBe(false);
    hold();
    await settle();
    expect(folded()).toBe(true);
    expect(toggle().getAttribute('aria-expanded')).toBe('false');
  });

  it('Q2: the yield ends → unfolded again', async () => {
    mountCard();
    await settle();
    const release = hold();
    await settle();
    expect(folded()).toBe(true);
    release();
    await settle();
    expect(folded()).toBe(false);
    expect(toggle().getAttribute('aria-expanded')).toBe('true');
  });

  it('Q3: folded by the user before the yield → stays folded through it and after', async () => {
    mountCard();
    await settle();
    toggle().click();
    await settle();
    expect(folded()).toBe(true);
    const release = hold();
    await settle();
    expect(folded()).toBe(true);
    release();
    await settle();
    expect(folded(), 'the user had folded it: the yield ending must not unfold it').toBe(true);
  });

  it('Q4: the user presses + during the yield → unfolds, and stays unfolded when the yield ends', async () => {
    mountCard();
    await settle();
    // folded by the user first, so "restore what it was" (folded) and "keep the user's last choice" (unfolded) differ
    toggle().click();
    await settle();
    expect(folded()).toBe(true);
    const release = hold();
    await settle();
    expect(folded()).toBe(true);
    toggle().click();
    await settle();
    expect(folded(), 'the user can still unfold during a yield').toBe(false);
    release();
    await settle();
    expect(folded(), "the user's last choice stands after the yield").toBe(false);
  });
});
