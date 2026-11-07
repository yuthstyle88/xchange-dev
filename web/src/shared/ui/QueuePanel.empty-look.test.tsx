// OL-6 wave B kiosk-queue: the panel's empty state and pager. Written before the implementation by the test-writer
// claude/94389a6b on the sub-controller claude/1cc51615's order. Do not edit, delete or weaken.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { meOf } from '../../test-support/me';
import type { ApiClient, PageParams, ViewAnswer, ViewParams } from '../api/client';
import { setLang } from '../i18n';
import { queueStore } from '../queue/queue-store';
import type { SessionStore } from '../session/session';
import { makeKioskQueueRow, makeQueueBoardData, type Me } from '../testing/fixtures';

// Restated (no-inline-dto guard, design §4.3): the row type is inferred from the fixture factory, never named.
type Row = ReturnType<typeof makeKioskQueueRow>;
import { QueuePanel } from './QueuePanel';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

const ME: Me = meOf({ id: 'E1', role: 'FRONT', branch: 'B-HOME', name: 'S', branchName: 'B-HOME', branches: [] });

function row(n: number): Row {
  return makeKioskQueueRow({
    id: `kq-${n}`,
    queue: `Q${n}`,
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
  });
}

let rows: Row[];
let secondPage: Row[];
const api = {
  view: async <T,>(name: string, _params: ViewParams = {}): Promise<ViewAnswer<T>> => {
    if (name === 'queue_board') {
      return { version: 1, changed: true, data: makeQueueBoardData({ waiting: [], called: [], myCounter: null }) as unknown as T };
    }
    return { version: 1, changed: true, data: rows.slice(0, 51) as unknown as T };
  },
  page: async <R,>(_name: string, _params: PageParams = {}): Promise<R[]> => secondPage as unknown as R[],
  post: async <B, R>(_path: string, _body: B): Promise<R> => ({}) as R,
};

function session(): SessionStore {
  const [me] = createSignal<Me | undefined>(ME);
  return { me, signOut: vi.fn(async () => {}), dispose: () => {} };
}

let branchNo = 3000;
let dispose: (() => void) | undefined;
let host: HTMLDivElement;

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await vi.advanceTimersByTimeAsync(0);
}

function mount(): void {
  branchNo += 1;
  const branch = `QP-EMPTY-${branchNo}`;
  queueStore(branch, { api: api as unknown as ApiClient, session: session() });
  dispose = render(() => <QueuePanel branch={branch} shown={true} api={api as unknown as ApiClient} />, host);
}

const pager = () => host.querySelector('.xc-queue-pager');

describe('QueuePanel empty state and pager (OL-6 wave B)', () => {
  let source: string;

  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    source = fs.readFileSync(here + 'QueuePanel.tsx', 'utf8');
  });

  beforeEach(() => {
    vi.useFakeTimers();
    setLang('th');
    rows = [];
    secondPage = [];
    host = document.createElement('div');
    document.body.append(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.useRealTimers();
  });

  it('QP-1: the panel imports the shared card part', () => {
    expect(source).toContain("import './card.css';");
  });

  it('QP-2: zero rows on page 1: the empty state is a card, with the same text', async () => {
    mount();
    await flush();
    const empty = host.querySelector('.queue-empty');
    expect(empty?.classList.contains('card')).toBe(true);
    expect(empty?.textContent).toBe('ไม่มีรายการในหมวดนี้');
  });

  it('QP-3: zero rows on page 1: no pager', async () => {
    mount();
    await flush();
    expect(host.querySelector('.queue-empty')).not.toBeNull();
    expect(pager()).toBeNull();
  });

  it('QP-4: one row on page 1: the pager shows', async () => {
    rows = [row(1)];
    mount();
    await flush();
    expect(pager()).not.toBeNull();
  });

  it('QP-5: zero rows on page 2: the pager stays, so the user can go back', async () => {
    rows = Array.from({ length: 51 }, (_, i) => row(i + 1));
    secondPage = [];
    mount();
    await flush();
    const next = [...host.querySelectorAll<HTMLButtonElement>('.xc-queue-pager button')].at(-1)!;
    next.click();
    await flush();
    expect(host.querySelectorAll('article')).toHaveLength(0);
    const back = host.querySelector<HTMLButtonElement>('.xc-queue-pager button');
    expect(back).not.toBeNull();
    expect(back?.disabled).toBe(false);
  });
});
