// OL-6 wave B queue-card-place (audit #564 issuecomment-6702970101, shots 058, 013, 002). Written before the fix by the
// ticket author claude/studio-claude-17 for the sub-controller claude/1cc51615. Do not edit, delete or weaken.
//
// The card touches the screen edges at 700 px or less (058), sits on the last two columns of /staff/report (013), and
// its sound buttons have the paper fill, not the theme's secondary (002). The harness is QueueCard.float.test.tsx's.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Accessor } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../i18n';
import { queueStore, type QueueError, type QueueStore } from '../queue/queue-store';
import {
  makeQueueBoardData,
  makeQueueBoardWaiting,
  type Me,
  type QueueBoardData,
} from '../testing/fixtures';
import { QueueCard } from './QueueCard';
import { meOf } from '../../test-support/me';

// ── fake queue store (as QueueCard.test.tsx's) ───────────────────────────────

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

/**
 * `count` WAITING tickets in call order (`queue_board`'s `waiting`); the newest one (the latest `issuedAt`) has the id
 * `latest`, so it is the card's "new request" key. The older ones keep the same ids from one answer to the next.
 */
function waiting(count: number, latest: string | null) {
  const list = [];
  for (let i = 1; i <= count; i += 1) {
    const newest = i === count;
    list.push(
      makeQueueBoardWaiting({
        id: newest && latest !== null ? latest : `older-${i}`,
        displayNumber: `W${String(i).padStart(3, '0')}`,
        serviceType: 'WALK_IN',
        sequence: i,
        issuedAt: new Date(Date.parse('2026-10-08T02:00:00.000Z') + i * 60_000).toISOString(),
      }),
    );
  }
  return list;
}

function answer(count: number, latest: string | null = count ? `K${count}` : null) {
  const s = queueStore('B1') as FakeStore;
  s.setError(undefined);
  const data = makeQueueBoardData({ waiting: waiting(count, latest), called: [], myCounter: null });
  s.setState(data);
}

function viewport(width: number) {
  const mm = (query: string) => {
    const q = query.replace(/\s+/g, '');
    const max = /max-width:(\d+(?:\.\d+)?)px/.exec(q);
    const min = /min-width:(\d+(?:\.\d+)?)px/.exec(q);
    let matches = max !== null || min !== null;
    if (max) matches = matches && width <= Number(max[1]);
    if (min) matches = matches && width >= Number(min[1]);
    return {
      matches,
      media: query,
      onchange: null,
      addEventListener() {},
      removeEventListener() {},
      addListener() {},
      removeListener() {},
      dispatchEvent: () => false,
    };
  };
  Object.defineProperty(window, 'matchMedia', { value: mm, configurable: true, writable: true });
}

const flush = () => new Promise<void>((r) => setTimeout(r, 0));
let disposers: (() => void)[] = [];

async function mountCard(opts: { startFolded?: boolean } = {}) {
  const host = document.createElement('div');
  document.body.append(host);
  const session = { me: () => NOK, signOut: async () => {}, dispose: () => {} };
  const props = { session, ...opts } as unknown as Parameters<typeof QueueCard>[0];
  disposers.push(render(() => <QueueCard {...props} />, host));
  await flush();
}

function card(): HTMLElement {
  const all = [...document.querySelectorAll<HTMLElement>('aside')].filter(
    (a) => a.getAttribute('aria-label') === 'แจ้งเตือนคิวลูกค้า',
  );
  expect(all, 'one card labelled แจ้งเตือนคิวลูกค้า').toHaveLength(1);
  return all[0]!;
}

function fold(): HTMLButtonElement {
  const b = card().querySelector<HTMLButtonElement>('button[aria-expanded]');
  expect(b, 'the fold button').not.toBeNull();
  return b!;
}

function expectMinimized(want: boolean, why: string) {
  expect(fold().getAttribute('aria-expanded'), `${why}: aria-expanded`).toBe(want ? 'false' : 'true');
  expect(card().classList.contains('minimized'), `${why}: class minimized on the card`).toBe(want);
}

beforeEach(() => {
  setLang('th');
  fake.stores.clear();
  Reflect.deleteProperty(window, 'matchMedia');
  vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 500 })));
});

afterEach(() => {
  for (const d of disposers.splice(0)) d();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  Reflect.deleteProperty(window, 'matchMedia');
});

function atPath(path: string) {
  window.history.pushState({}, '', path);
}

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
}

async function sheet(): Promise<string> {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  return fs.readFileSync(here + 'queue-card.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
}

function decl(body: string, selector: string, prop: string): string | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  let found: string | undefined;
  while ((m = re.exec(body)) !== null) {
    if (m[1]!.trim().replace(/\s+/g, ' ') !== selector) continue;
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0 && d.slice(0, i).trim() === prop) found = d.slice(i + 1).trim();
    }
  }
  return found;
}

function media(css: string, query: RegExp): string {
  const at = css.search(query);
  expect(at, String(query)).toBeGreaterThanOrEqual(0);
  const open = css.indexOf('{', at);
  let depth = 1;
  let i = open + 1;
  while (depth > 0) {
    if (css[i] === '{') depth++;
    if (css[i] === '}') depth--;
    i++;
  }
  return css.slice(open + 1, i - 1);
}

afterEach(() => atPath('/'));

describe('OL-6 B the card sits clear of the screen edges (058)', () => {
  it('QP-1 at 700px or less it floats with a 16px gutter on left, right and bottom, every corner round', async () => {
    const m = media(await sheet(), /@media\s*\(max-width:\s*700px\)/);
    expect(decl(m, '.queue-card', 'left')).toBe('16px');
    expect(decl(m, '.queue-card', 'right')).toBe('16px');
    expect(decl(m, '.queue-card', 'bottom')).toBe('16px');
    expect(decl(m, '.queue-card', 'width')).toBe('auto');
    expect(decl(m, '.queue-card', 'max-width')).toBe('none');
    expect(decl(m, '.queue-card', 'border-radius')).toBe('16px');
  });
});

describe('OL-6 B the sound buttons are the theme secondary (002)', () => {
  it('QP-2 white fill, 1px line, brand text, the theme radius', async () => {
    const css = await sheet();
    const s = '.queue-card .queue-sound-controls button';
    expect(decl(css, s, 'background')).toBe('var(--xc-surface)');
    expect(decl(css, s, 'border')).toBe('1px solid var(--xc-line)');
    expect(decl(css, s, 'color')).toBe('var(--xc-brand)');
    expect(decl(css, s, 'border-radius')).toBe('var(--xc-radius-md)');
  });
});

// Restated for OL-6 wave B queue-card-f52 by claude/studio-claude-17, on the order of the sub-controller claude/1cc51615
// (owner finding F52): the page no longer decides by pathname inside QueueCard.tsx. /staff/report's main.tsx passes
// AppShell queueCard="folded", which reaches the card as `startFolded` (AppShell.queue-card.test.tsx QM-3, QM-5). The
// behaviour rows stay: folded at the start, the button and a waiting customer open it, no path folds it by itself.
describe('OL-6 B a card told to start folded starts folded (013, F52)', () => {
  it('QP-3 startFolded at 1280px starts minimized', async () => {
    viewport(1280);
    await mountCard({ startFolded: true });
    expectMinimized(true, 'startFolded at mount');
  });

  it('QP-4 the person can open it', async () => {
    viewport(1280);
    await mountCard({ startFolded: true });
    fold().click();
    await flush();
    expectMinimized(false, 'after the button');
  });

  it('QP-5 a waiting customer still opens it', async () => {
    viewport(1280);
    await mountCard({ startFolded: true });
    answer(0);
    await flush();
    expectMinimized(true, 'count 0');
    answer(2);
    await flush();
    expectMinimized(false, 'count 0 to 2');
  });

  it('QP-6 without startFolded a 1280px card starts open on every path, /staff/report included', async () => {
    for (const path of ['/staff', '/desk', '/staff/report']) {
      atPath(path);
      viewport(1280);
      await mountCard();
      expectMinimized(false, path);
      for (const d of disposers.splice(0)) d();
      document.body.innerHTML = '';
    }
  });
});
