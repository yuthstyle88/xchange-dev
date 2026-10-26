// #5126: page reserves bottom space while queue card is shown so inputs at bottom can scroll clear.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor } from 'solid-js';
import { render } from 'solid-js/web';
import type { SessionStore } from '../session/session';
import type { QueueError, QueueStore } from '../queue/queue-store';
import { makeMe, type Me, type QueueBoardData } from '../testing/fixtures';
import { meOf } from '../../test-support/me';
import { QueueCard } from './QueueCard';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

async function readSiblingCss(name: string): Promise<string> {
  const fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
  const path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
  const here = path.dirname(new URL(import.meta.url).pathname);
  return fs.readFileSync(path.join(here, name), 'utf8');
}

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

function triggerResize(height: number, target?: Element) {
  if (!activeCallback) return;
  const el = target ?? observedElements[0] ?? document.querySelector('.queue-card') ?? document.body;
  activeCallback(
    [
      {
        target: el,
        borderBoxSize: [{ blockSize: height, inlineSize: 320 }],
        contentRect: { height, width: 320, top: 0, bottom: height, left: 0, right: 320, x: 0, y: 0 },
      } as unknown as ResizeObserverEntry,
    ],
    {} as ResizeObserver,
  );
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

afterEach(() => {
  for (const d of disposers.splice(0)) d();
  document.body.innerHTML = '';
  document.documentElement.style.removeProperty('--xc-queue-card-space');
  vi.unstubAllGlobals();
});

describe('QueueCard bottom space reservation (#5126)', () => {
  it('card mounted, measured height H: sets --xc-queue-card-space to H + 36px', () => {
    mountCard();
    expect(observedElements.length).toBe(1);
    expect(observedElements[0].classList.contains('queue-card')).toBe(true);

    triggerResize(180);
    expect(document.documentElement.style.getPropertyValue('--xc-queue-card-space')).toBe('216px');
  });

  it('card folds or unfolds (height changes): property updates to new H + 36px', () => {
    mountCard();
    triggerResize(200);
    expect(document.documentElement.style.getPropertyValue('--xc-queue-card-space')).toBe('236px');

    // Fold: height becomes 64px
    triggerResize(64);
    expect(document.documentElement.style.getPropertyValue('--xc-queue-card-space')).toBe('100px');

    // Unfold: height becomes 240px
    triggerResize(240);
    expect(document.documentElement.style.getPropertyValue('--xc-queue-card-space')).toBe('276px');

    // Hidden: height becomes 0px
    triggerResize(0);
    expect(document.documentElement.style.getPropertyValue('--xc-queue-card-space')).toBe('0px');
  });

  it('card unmounts: the property is removed from document.documentElement', () => {
    const { dispose } = mountCard();
    triggerResize(180);
    expect(document.documentElement.style.getPropertyValue('--xc-queue-card-space')).toBe('216px');

    dispose();
    expect(document.documentElement.style.getPropertyValue('--xc-queue-card-space')).toBe('');
  });

  it('no card (signed out): no property is set and page remains unchanged', () => {
    mountCard(null);
    expect(document.querySelector('.queue-card')).toBeNull();
    expect(document.documentElement.style.getPropertyValue('--xc-queue-card-space')).toBe('');
  });

  it('no card (signed in without QueueOperate permission): no property is set', () => {
    const userWithoutQueue = makeMe({ permissions: [] });
    mountCard(userWithoutQueue);
    expect(document.querySelector('.queue-card')).toBeNull();
    expect(document.documentElement.style.getPropertyValue('--xc-queue-card-space')).toBe('');
  });

  it('falls back to contentRect if borderBoxSize is missing in ResizeObserverEntry', () => {
    mountCard();
    expect(activeCallback).toBeDefined();
    const el = observedElements[0];
    activeCallback!(
      [
        {
          target: el,
          contentRect: { height: 150, width: 320, top: 0, bottom: 150, left: 0, right: 320, x: 0, y: 0 },
        } as unknown as ResizeObserverEntry,
      ],
      {} as ResizeObserver,
    );
    expect(document.documentElement.style.getPropertyValue('--xc-queue-card-space')).toBe('186px');
  });

  it('queue-card.css adds html padding-bottom with var(--xc-queue-card-space, 0px)', async () => {
    const cssContent = await readSiblingCss('queue-card.css');
    expect(cssContent).toMatch(/html\s*\{[^}]*padding-bottom:\s*var\(--xc-queue-card-space,\s*0px\);/);
  });
});
