// Specification tests for QU-6 (docs/design/blueprint/queue-tickets.md §4.5, §9.1 row QU-6):
// (b) the queue card shows queue_board's lines.
//
// Written before implementation by test-writer (gemini/studio-11, ordered by
// claude/5edf7bb5, Owner's Rule A). Do not edit, delete or weaken.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor } from 'solid-js';
import { render } from 'solid-js/web';
import type { Me } from '../api/generated/common';
import { setLang } from '../i18n';
import type { QueueBoardData } from '../api/generated/queue_board';
import type { QueueError, QueueStore } from '../queue/queue-store';
import { QueueCard } from './QueueCard';
import { meOf } from '../../test-support/me';

interface FakeStore extends Omit<QueueStore, 'state'> {
  state: Accessor<QueueBoardData | undefined>;
  setState(v: QueueBoardData | undefined): void;
  setError(v: QueueError | undefined): void;
}

const fake = vi.hoisted(() => ({
  stores: new Map<string, unknown>(),
  useQueueStoreCalls: 0,
}));

vi.mock('../queue/queue-store', async () => {
  const solid = await import('solid-js');
  const [branch] = solid.createSignal('B1');
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
    setQueueBranch: () => {},
    useQueueStore: () => {
      fake.useQueueStoreCalls += 1;
      return () => store(branch());
    },
    queueStore: store,
  };
});

const NOK: Me = meOf({
  id: 'emp-nok',
  role: 'FRONT',
  branch: 'B1',
  name: 'Nok',
});

const disposers: Array<() => void> = [];

function store(b = 'B1'): FakeStore {
  const s = fake.stores.get(b) as FakeStore | undefined;
  if (!s) throw new Error(`no fake store for ${b}`);
  return s;
}

function answerBoard(b: string, data: QueueBoardData) {
  const s = store(b);
  s.setError(undefined);
  s.setState(data);
}

function cards(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('aside')).filter(
    (a) => a.getAttribute('aria-label') === 'แจ้งเตือนคิวลูกค้า',
  );
}

function theCard(): HTMLElement {
  const found = cards();
  if (found.length !== 1) throw new Error(`expected 1 card, found ${found.length}`);
  return found[0];
}

async function flush() {
  for (let i = 0; i < 20; i += 1) await Promise.resolve();
}

function shown(el: Element | null | undefined): boolean {
  if (!el || !el.isConnected) return false;
  for (let e: Element | null = el; e; e = e.parentElement) {
    if (e.hasAttribute('hidden')) return false;
    if (getComputedStyle(e).display === 'none') return false;
  }
  return true;
}

let fetchSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  fake.useQueueStoreCalls = 0;
  fetchSpy = vi.fn(async () => new Response('{}', { status: 200 }));
  vi.stubGlobal('fetch', fetchSpy);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('QueueCard on queue_board data (QU-6 item b)', () => {
  beforeEach(() => {
    setLang('th');
    fake.stores.clear();
    const host = document.createElement('div');
    document.body.appendChild(host);
    const [me] = createSignal<Me | undefined>(NOK);
    const session = {
      me,
      signOut: async () => {},
      dispose: () => {},
    };
    disposers.push(render(() => <QueueCard session={session} />, host));
  });

  afterEach(() => {
    while (disposers.length) disposers.pop()!();
    document.body.innerHTML = '';
  });

  it('shows the waiting ticket count from queue_board.waiting on the card badge', async () => {
    answerBoard('B1', {
      waiting: [
        { id: 'w-1', displayNumber: 'W001', serviceType: 'WALK_IN', sequence: 1, issuedAt: '2026-10-12T01:00:00Z' },
        { id: 'w-2', displayNumber: 'B001', serviceType: 'BOOKING', sequence: 1, issuedAt: '2026-10-12T01:05:00Z' },
      ],
      called: [],
      myCounter: { status: 'ACTIVE' },
    });
    await flush();

    const card = theCard();
    const badge = card.querySelector('.queue-card-badge');
    expect(badge?.textContent?.trim()).toBe('2');
  });

  it('sets the .waiting class when queue_board.waiting has tickets', async () => {
    answerBoard('B1', {
      waiting: [
        { id: 'w-1', displayNumber: 'W001', serviceType: 'WALK_IN', sequence: 1, issuedAt: '2026-10-12T01:00:00Z' },
      ],
      called: [],
      myCounter: null,
    });
    await flush();

    const card = theCard();
    expect(card.classList.contains('waiting')).toBe(true);
  });

  it('renders called ticket lines from queue_board.called in the card workers section', async () => {
    answerBoard('B1', {
      waiting: [],
      called: [
        {
          id: 'c-1',
          displayNumber: 'W005',
          serviceType: 'WALK_IN',
          status: 'SERVING',
          tellerId: 'emp-nok',
          calledAt: '2026-10-12T01:10:00Z',
        },
      ],
      myCounter: { status: 'ACTIVE' },
    });
    await flush();

    const card = theCard();
    const workerLines = card.querySelectorAll('.queue-card-worker');
    expect(workerLines.length).toBe(1);
    expect(workerLines[0]?.textContent).toContain('W005');
  });
});

describe('QueueCard mounts only for QueueOperate (QU-6 item 3)', () => {
  let host: HTMLDivElement;

  beforeEach(() => {
    setLang('th');
    fake.stores.clear();
    fake.useQueueStoreCalls = 0;
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    while (disposers.length) disposers.pop()!();
    host.remove();
    document.body.innerHTML = '';
  });

  it('mounts and is shown when session holds QueueOperate', async () => {
    const [me] = createSignal<Me | undefined>(NOK);
    const session = {
      me,
      signOut: async () => {},
      dispose: () => {},
    };
    disposers.push(render(() => <QueueCard session={session} />, host));
    await flush();

    const list = cards();
    expect(list.length).toBe(1);
    expect(shown(list[0])).toBe(true);
  });

  it('is not mounted and makes no queue_board request without QueueOperate', async () => {
    const noOperate = meOf({
      id: 'emp-rates',
      role: 'RATES',
      branch: 'B1',
      name: 'Rates User',
    });
    const [me] = createSignal<Me | undefined>(noOperate);
    const session = {
      me,
      signOut: async () => {},
      dispose: () => {},
    };
    fake.useQueueStoreCalls = 0;
    fetchSpy.mockClear();

    disposers.push(render(() => <QueueCard session={session} />, host));
    await flush();

    expect(cards().length).toBe(0);
    const requestedUrls = fetchSpy.mock.calls.map((c) =>
      String((c as unknown[])[0] instanceof Request ? ((c as unknown[])[0] as Request).url : (c as unknown[])[0]),
    );
    expect(requestedUrls.some((u) => u.includes('queue_board'))).toBe(false);
    expect(fake.useQueueStoreCalls).toBe(0);
  });
});

