// #786 QF8 (owner, 2026-10-10, answer "ก"): the floating queue card keeps the old admin bar's
// "who is working which queue" list, shown only while the card is expanded. v2's floating widget
// (account-bar.js) had no such list: this is a v3 addition by the owner. Written before the
// implementation by the ticket's author (claude/b060e9). Do not edit, delete or weaken; add tests
// in a file of your own.
//
// Where it shows: on every screen that shows the card. `QueueCard` reads the one shared queue store
// (`useQueueStore(queueBranch)`), and the `kiosk_notifications` view carries `working` for every
// staff caller (`api/src/presentation/http/v3/views/kiosk_notifications.rs`, no role filter), so the
// card has the same data on every screen. K1–K6 mount the card alone (any screen with the account
// bar); K7 mounts it next to the real `QueueBar`, as on admin.
//
// Pinned:
//   - [required] the list is an element with class `queue-card-workers` inside the card's `aside`,
//     one `.queue-card-worker` child per `working` entry, in the server's order;
//   - [required] each line reads `<staff> กำลังทำรายการ <queue> · เริ่ม <time>` (en:
//     `<staff> serving <queue> · started <time>`), the old bar's `workerLine` text, with `<time>` =
//     `new Date(startedAt).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' })`, or
//     `ยังไม่มีข้อมูล` / `No data` when `startedAt` is null (the bar's `noData`);
//   - [required] minimized: the lines are not shown (absent, or under an element with `hidden`);
//   - [required] an empty `working` list, or offline (the bar's `showWorkers()`), renders no list element.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `Me` fixture (`kind`/`role`/`username`) → `meOf(…)` and `employee {id, role}` → `employeeOf(…)` (`web/src/test-support/me.ts`, design §3.3: FRONT → TELLER at its branch). Nothing else changes.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { Me } from '../api/generated/common';
import { locale, setLang, t } from '../i18n';
import { queueStore, type QueueError, type QueueStore } from '../queue/queue-store';
import type { KioskNotifications } from '../queue/wire';
import { QueueCard } from './QueueCard';
import { QueueBar } from '../../screens/admin/queue/QueueBar';
import { employeeOf, meOf } from '../../test-support/me';

interface FakeStore extends QueueStore {
  setState(v: KioskNotifications | undefined): void;
  setError(v: QueueError | undefined): void;
}

const fake = vi.hoisted(() => ({ stores: new Map<string, unknown>() }));

vi.mock('../queue/queue-store', async () => {
  const solid = await import('solid-js');
  const [branch, setBranch] = solid.createSignal('B1');
  const store = (b: string): FakeStore => {
    let s = fake.stores.get(b) as FakeStore | undefined;
    if (!s) {
      const [state, setState] = solid.createSignal<KioskNotifications | undefined>(undefined);
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

vi.mock('./QueuePanel', () => ({
  QueuePanel(_props: { branch: string; shown: boolean }): JSX.Element {
    const el = document.createElement('div');
    el.setAttribute('data-fake', 'queue-panel');
    return el;
  },
}));

const NOK_V3 = { id: 'E7', role: 'FRONT', branch: 'B1', name: 'Nok', branchName: 'B1', branches: [] };
const NOK: Me = meOf(NOK_V3);

// 02:15Z is 09:15 in Bangkok: a stamp printed in UTC (browserInUtc) reads 2:15, not 9:15.
const STARTED = '2026-10-10T02:15:00.000Z';
const TWO = [
  { id: 'K1', queue: 'Q-1', staff: 'nok', startedAt: STARTED, status: 'ACCEPTED' },
  { id: 'K2', queue: 'Q-2', staff: 'ploy', startedAt: null, status: 'ACCEPTED' },
];

function answer(over: Partial<KioskNotifications> = {}) {
  const s = queueStore('B1') as FakeStore;
  s.setError(undefined);
  s.setState({
    serverNow: '2026-10-10T03:00:00.000Z',
    branchId: 'B1',
    branchName: 'Silom',
    count: 1,
    latest: 'K9',
    workingCount: 0,
    working: [],
    employee: employeeOf(NOK),
    ...over,
  });
}

function viewport(width: number) {
  const mm = (query: string) => {
    const max = /max-width:\s*(\d+)px/.exec(query);
    return {
      matches: max ? width <= Number(max[1]) : false,
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

const session = { me: () => NOK, signOut: async () => {}, dispose: () => {} };

async function mountCard() {
  const host = document.createElement('div');
  document.body.append(host);
  disposers.push(render(() => <QueueCard session={session} />, host));
  await flush();
}

async function mountAdmin() {
  const [branch] = createSignal('B1');
  const [label] = createSignal('สาขาสีลม');
  const host = document.createElement('div');
  document.body.append(host);
  disposers.push(
    render(
      () => (
        <>
          <QueueCard session={session} />
          <QueueBar branch={branch} branchLabel={label} />
        </>
      ),
      host,
    ),
  );
  await flush();
}

function card(): HTMLElement {
  const all = [...document.querySelectorAll<HTMLElement>('aside')].filter(
    (a) => a.getAttribute('aria-label') === t('queue.card.aria'),
  );
  expect(all, `one card labelled ${t('queue.card.aria')}`).toHaveLength(1);
  return all[0]!;
}

function fold(): HTMLButtonElement {
  const b = card().querySelector<HTMLButtonElement>('button[aria-expanded]');
  expect(b, 'the fold button').not.toBeNull();
  return b!;
}

function list(): HTMLElement | null {
  return card().querySelector<HTMLElement>('.queue-card-workers');
}

function lines(): HTMLElement[] {
  return [...card().querySelectorAll<HTMLElement>('.queue-card-workers .queue-card-worker')];
}

/** Shown = no element from `el` up to the card carries `hidden`. */
function shown(el: HTMLElement): boolean {
  for (let n: HTMLElement | null = el; n; n = n.parentElement) {
    if (n.hidden || n.hasAttribute('hidden')) return false;
    if (n === card()) return true;
  }
  return false;
}

const realToLocaleString = Date.prototype.toLocaleString;

/** The stamp the line must carry, computed with the real `toLocaleString`. */
function stamp(iso: string): string {
  return realToLocaleString.call(new Date(iso), locale(), { timeZone: 'Asia/Bangkok' });
}

/**
 * Run the page as a browser whose own zone is UTC: a `toLocaleString` call that names no
 * `timeZone` prints UTC, so a stamp that forgets `Asia/Bangkok` reads 2:15 instead of 9:15
 * whatever zone the test machine is in.
 */
function browserInUtc() {
  vi.spyOn(Date.prototype, 'toLocaleString').mockImplementation(function (
    this: Date,
    loc?: Intl.LocalesArgument,
    opts?: Intl.DateTimeFormatOptions,
  ) {
    return realToLocaleString.call(this, loc, { timeZone: 'UTC', ...(opts ?? {}) });
  });
}

beforeEach(() => {
  setLang('th');
  fake.stores.clear();
  sessionStorage.clear();
  Reflect.deleteProperty(window, 'matchMedia');
  browserInUtc();
  vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 500 })));
});

afterEach(() => {
  for (const d of disposers.splice(0)) d();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  Reflect.deleteProperty(window, 'matchMedia');
});

describe('the workers list in the expanded card (QF8)', () => {
  it('K1: expanded, one line per working entry, in order, with the workerLine text and the Bangkok stamp', async () => {
    await mountCard();
    answer({ workingCount: 2, working: TWO });
    await flush();
    expect(fold().getAttribute('aria-expanded'), 'expanded').toBe('true');
    expect(list(), 'the list element .queue-card-workers').not.toBeNull();
    const got = lines();
    expect(got.map((l) => l.textContent?.trim())).toEqual([
      `nok กำลังทำรายการ Q-1 · เริ่ม ${stamp(STARTED)}`,
      'ploy กำลังทำรายการ Q-2 · เริ่ม ยังไม่มีข้อมูล',
    ]);
    expect(got[0]!.textContent, 'Bangkok time, not UTC').toMatch(/\b0?9[:.]15/);
    expect(got.every(shown), 'every line is shown').toBe(true);
  });

  it('K2: the English twin', async () => {
    setLang('en');
    await mountCard();
    answer({ workingCount: 2, working: TWO });
    await flush();
    expect(lines().map((l) => l.textContent?.trim())).toEqual([
      `nok serving Q-1 · started ${stamp(STARTED)}`,
      'ploy serving Q-2 · started No data',
    ]);
  });

  it('K3: minimized by the button, the lines are not shown; expanding shows them again', async () => {
    await mountCard();
    answer({ workingCount: 2, working: TWO });
    await flush();
    fold().click();
    await flush();
    expect(fold().getAttribute('aria-expanded')).toBe('false');
    expect(lines().filter(shown), 'no line is shown while minimized').toEqual([]);
    fold().click();
    await flush();
    expect(lines().filter(shown)).toHaveLength(2);
  });

  it('K4: a phone starts minimized with the lines not shown (no waiting count to unfold it)', async () => {
    viewport(700);
    await mountCard();
    answer({ count: 0, latest: null, workingCount: 1, working: [TWO[0]!] });
    await flush();
    expect(fold().getAttribute('aria-expanded')).toBe('false');
    expect(lines().filter(shown)).toEqual([]);
    fold().click();
    await flush();
    expect(lines().filter(shown), 'expanded by the user').toHaveLength(1);
  });

  it('K5: an empty working list renders no list element, and the list goes when working empties', async () => {
    await mountCard();
    answer({ workingCount: 0, working: [] });
    await flush();
    expect(list(), 'no .queue-card-workers for an empty list').toBeNull();
    answer({ workingCount: 1, working: [TWO[0]!] });
    await flush();
    expect(lines()).toHaveLength(1);
    answer({ workingCount: 0, working: [] });
    await flush();
    expect(list(), 'gone once working is empty again').toBeNull();
  });

  it('K6: offline renders no list element (the bar\'s showWorkers rule); the next answer brings it back', async () => {
    await mountCard();
    answer({ workingCount: 1, working: [TWO[0]!] });
    await flush();
    (queueStore('B1') as FakeStore).setError({ kind: 'offline' });
    await flush();
    expect(list(), 'offline').toBeNull();
    answer({ workingCount: 1, working: [TWO[0]!] });
    await flush();
    expect(lines()).toHaveLength(1);
  });

  it('K7: on admin (the card next to QueueBar) the lines show in the card, once on the page', async () => {
    await mountAdmin();
    answer({ workingCount: 2, working: TWO });
    await flush();
    expect(lines().map((l) => l.textContent?.trim())).toEqual([
      `nok กำลังทำรายการ Q-1 · เริ่ม ${stamp(STARTED)}`,
      'ploy กำลังทำรายการ Q-2 · เริ่ม ยังไม่มีข้อมูล',
    ]);
    const anywhere = [...document.querySelectorAll('*')].filter(
      (n) => n.children.length === 0 && /กำลังทำรายการ Q-1 · เริ่ม/.test(n.textContent ?? ''),
    );
    expect(anywhere, 'the Q-1 line is on the page exactly once, in the card').toHaveLength(1);
    expect(card().contains(anywhere[0]!)).toBe(true);
  });
});
