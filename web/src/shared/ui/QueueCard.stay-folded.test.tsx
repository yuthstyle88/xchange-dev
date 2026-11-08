// OL-8 staff-queue-pill (audit rows desk/staff 27 + 30; ruling (a) by the sub-controller claude/1cc51615, 2026-11-08
// ICT): on /staff/report and /staff/guide the queue card stays the folded pill even when a customer waits, so it never
// covers the report columns or the guide text; the signal there is the badge count, the waiting border and the sound.
// A click still opens it. Every other page keeps "a waiting customer opens it" (QueueCard.test.tsx, untouched).
// The page asks for it with `queueCard="pill"` (QueueCardMode); AccountBar turns that into `startFolded` +
// `stayFolded` on QueueCard. Harness copied from QueueCard.yield.test.tsx. Written before the fix by claude/d5c44a22.
// Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { SessionStore } from '../session/session';
import type { QueueError, QueueStore } from '../queue/queue-store';
import { makeQueueBoardData, makeQueueBoardWaiting, type Me, type QueueBoardData } from '../testing/fixtures';
import { meOf } from '../../test-support/me';
import { QueueCard } from './QueueCard';

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

const NOK: Me = meOf({ id: 'E7', role: 'FRONT', branch: 'B1', name: 'Nok', branchName: 'B1', branches: [] });

class MockResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

const disposers: Array<() => void> = [];

/** Mounts the card with the given props (the session is always Nok, who holds QueueOperate). */
function mountCard(props: Record<string, unknown>) {
  const session: SessionStore = { me: () => NOK, signOut: async () => {}, dispose: () => {} };
  const host = document.createElement('div');
  document.body.appendChild(host);
  // `stayFolded` is the prop this ticket adds; spread so the file compiles in vitest before it exists.
  const Card = QueueCard as unknown as (p: Record<string, unknown>) => JSX.Element;
  disposers.push(render(() => <Card session={session} {...props} />, host));
}

/** The store answers `count` waiting customers (a new object each time, like a `changed: true` poll). */
function answerWaiting(count: number) {
  const s = fake.stores.get('B1') as FakeStore;
  const waiting = [];
  for (let i = 1; i <= count; i += 1) {
    waiting.push(
      makeQueueBoardWaiting({
        id: `w-${i}`,
        displayNumber: `W${String(i).padStart(3, '0')}`,
        serviceType: 'WALK_IN',
        sequence: i,
        issuedAt: new Date(Date.parse('2026-11-08T02:00:00.000Z') + i * 60_000).toISOString(),
      }),
    );
  }
  s.setError(undefined);
  s.setState(makeQueueBoardData({ waiting, called: [], myCounter: null }));
}

beforeEach(() => {
  fake.stores.clear();
  vi.stubGlobal('ResizeObserver', MockResizeObserver);
});

afterEach(() => {
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
const badge = () => card().querySelector<HTMLElement>('.queue-card-badge')!.textContent;
async function settle() {
  for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));
}

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

/** Reads a file relative to this test's folder (web/src/shared/ui/). */
async function readRel(file: string): Promise<string> {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  return fs.readFileSync(here + file, 'utf8');
}

describe('staff-queue-pill: the card stays a pill on the report and the guide', () => {
  it('SQ-1 startFolded + stayFolded, 0 waiting: the pill (folded), badge 0', async () => {
    mountCard({ startFolded: true, stayFolded: true });
    answerWaiting(0);
    await settle();
    expect(folded()).toBe(true);
    expect(badge()).toBe('0');
  });

  it('SQ-2 startFolded + stayFolded, 0 → 2 waiting: still folded; badge 2 and the waiting class', async () => {
    mountCard({ startFolded: true, stayFolded: true });
    answerWaiting(0);
    await settle();
    answerWaiting(2);
    await settle();
    expect(folded(), 'a waiting customer must not unfold the pill here').toBe(true);
    expect(badge()).toBe('2');
    expect(card().classList.contains('waiting')).toBe(true);
    expect(toggle().getAttribute('aria-expanded')).toBe('false');
  });

  it('SQ-3 startFolded + stayFolded, 1 waiting on the first answer: still folded', async () => {
    mountCard({ startFolded: true, stayFolded: true });
    answerWaiting(1);
    await settle();
    expect(folded()).toBe(true);
    expect(badge()).toBe('1');
  });

  it('SQ-4 startFolded + stayFolded: a click opens it, a second click folds it again', async () => {
    mountCard({ startFolded: true, stayFolded: true });
    answerWaiting(1);
    await settle();
    toggle().click();
    await settle();
    expect(folded()).toBe(false);
    expect(toggle().getAttribute('aria-expanded')).toBe('true');
    toggle().click();
    await settle();
    expect(folded()).toBe(true);
  });

  it('SQ-5 opened by a click, a new waiting customer does not fold it back', async () => {
    mountCard({ startFolded: true, stayFolded: true });
    answerWaiting(1);
    await settle();
    toggle().click();
    await settle();
    answerWaiting(2);
    await settle();
    expect(folded()).toBe(false);
  });

  it('SQ-6 guard: startFolded without stayFolded (admin, other pages): 0 → 2 waiting still unfolds', async () => {
    mountCard({ startFolded: true });
    answerWaiting(0);
    await settle();
    answerWaiting(2);
    await settle();
    expect(folded()).toBe(false);
  });

  it('SQ-7 guard: no props (desk, staff home): 1 waiting opens / keeps the card open', async () => {
    mountCard({});
    answerWaiting(1);
    await settle();
    expect(folded()).toBe(false);
  });
});

describe('staff-queue-pill: the "pill" mode is wired from the page to the card', () => {
  it('SQ-8 QueueCardMode has "pill"', async () => {
    expect(await readRel('QueueCard.tsx')).toContain(
      "export type QueueCardMode = 'open' | 'folded' | 'pill' | 'hidden';",
    );
  });

  it('SQ-9 AccountBar passes startFolded for "folded" and "pill", and stayFolded for "pill" only', async () => {
    const src = (await readRel('AccountBar.tsx')).replace(/\s+/g, ' ');
    expect(src).toContain(
      "<QueueCard session={session()} startFolded={props.queueCard === 'folded' || props.queueCard === 'pill'} stayFolded={props.queueCard === 'pill'} />",
    );
  });

  it('SQ-10 /staff/report and /staff/guide ask for the pill; admin stays "folded"', async () => {
    expect(await readRel('../../screens/staff-report/main.tsx')).toContain('<AppShell title={t(\'title\')} queueCard="pill">');
    const guide = await readRel('../../screens/staff-guide/StaffGuide.tsx');
    expect(guide).toContain('queueCard="pill"');
    expect(guide).not.toContain('queueCard="folded"');
    expect(await readRel('../../screens/admin/AdminShell.tsx')).toContain('queueCard="folded"');
  });

  it('SQ-11 the pill box stays right of the 960 px column (x 1120 at 1280 px): right + max-width <= 160', async () => {
    const css = (await readRel('queue-card.css')).replace(/\/\*[\s\S]*?\*\//g, '');
    const right = Number(/\.queue-card\s*\{[^}]*?right:\s*(\d+)px/.exec(css)?.[1]);
    const max = Number(/\.queue-card\.minimized\s*\{[^}]*?max-width:\s*(\d+)px/.exec(css)?.[1]);
    expect(right + max).toBeLessThanOrEqual(160);
  });
});
