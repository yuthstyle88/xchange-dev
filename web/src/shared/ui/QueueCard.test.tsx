// V1-queue-card (#487): the queue card on every signed-in v3 screen, design
// docs/design/xchange-v3-web.md §12.18.6 rows C1–C9 and §12.18.8 (v2:
// src/devices-public/account-bar.js:16-46 of xchange-v2-source@4d0bdbb), and the sound controls
// of C7. Written before the implementation by the lane controller. Do not edit, delete or weaken;
// add tests in a file of your own.
//
// The API this file pins:
//
//   web/src/shared/ui/QueueCard.tsx
//     export function QueueCard(props: { session: SessionStore }): JSX.Element;
//   web/src/shared/ui/QueueSoundControls.tsx
//     export function QueueSoundControls(): JSX.Element;
//
// The card reads the queue store of #486 through `web/src/shared/queue/queue-store.ts`
// (`useQueueStore(queueBranch)`, or `queueStore(queueBranch())`): this file replaces that module
// with a fake whose stores the test drives — the same writes the real store makes (an answer
// clears `error` then sets `state`; a failure sets `error` and keeps `state`; an unchanged answer
// only clears `error`). So the card makes no request of its own: `fetch` is watched and only
// `/api/v3/views/me` (the shared session, AppShell's) is allowed.
//
// From `SessionStore` the card reads `me()` only (`me().id` for the sound key, `me().permissions` for the
// link). It never calls `signOut()` or `dispose()`.
//
// The card is found by its label, its parts by role and text, never by class or position, except
// class `waiting` (C4, named by the ticket). "Shown" means connected, no `hidden` on it or an
// ancestor, and no computed `display: none` — so a folded card may hide its parts with `hidden`,
// by not rendering them, or with an inline `<style>`; a CSS file import is not loaded by vitest.
// jsdom has no `matchMedia`: tests that need it install a fake answering `max-width` / `min-width`
// queries for a given viewport width; without it the card starts unfolded.
//
// Each test is a fresh page (`vi.resetModules()` then `import()`), because the sound and the dock
// are one per page. Thai is v2's, verbatim (W6).
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `Me` fixture (`kind`/`role`/`username`) → `meOf(…)` and `employee {id, role}` → `employeeOf(…)` (`web/src/test-support/me.ts`, design §3.3: FRONT → TELLER at its branch). Nothing else changes.
// C3 (ruling A, issuecomment on #1030 by claude/ad05bf8d): the link follows `me.permissions` holding
// `QueueOperate`. OWNER (→ EXECUTIVE + ADMIN, neither holds it) moves from the link list to the no-link
// list: an intended outcome change, since the server already refuses its accept/get.
// Added after claude/dell-claude-6's review (#1030, gap 2): two C3 cases where `roles` and `permissions`
// disagree, so a role-name rule cannot pass.
// QU-6 (#1354, rulings issuecomment-6117574257 and issuecomment-6118321787) by the test-writer claude/14cc24bd: the
// store now holds `queue_board`'s `QueueBoardData` (`waiting`/`called`/`myCounter`), so every answer is re-expressed on
// that shape: the count is `waiting.length`, the "latest" key is the newest waiting ticket's id, and the branch name
// comes from the session's `me` (its `branches`), as `queue_board` names no branch. Ruling (a): a session without
// `QueueOperate` gets no card at all, so the C3 "no link" rows and the C6 RATES row now expect no card. Nothing else changes.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { Me } from '../api/generated/common';
import type { SessionStore } from '../session/session';
import type { QueueError, QueueStore } from '../queue/queue-store';
import type { QueueBoardData, QueueBoardWaiting } from '../api/generated/queue_board';
import { meOf } from '../../test-support/me';

// ── fake queue store (#486's module) ─────────────────────────────────────────

interface FakeQueues {
  branch: Accessor<string>;
  setBranch(b: string): void;
  store(b: string): FakeStore;
  /** Forget every store and follow B1 again (the mock may outlive vi.resetModules). */
  reset(): void;
}

interface FakeStore extends Omit<QueueStore, 'state'> {
  state: Accessor<QueueBoardData | undefined>;
  setState(v: QueueBoardData | undefined): void;
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
      const [state, setState] = solid.createSignal<QueueBoardData | undefined>(undefined);
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

/**
 * `count` WAITING tickets in call order (`queue_board`'s `waiting`); the newest one (the latest `issuedAt`) has the id
 * `latest`, so it is the card's "new request" key. The older ones keep the same ids from one answer to the next.
 */
function waiting(count: number, latest: string | null): QueueBoardWaiting[] {
  const list: QueueBoardWaiting[] = [];
  for (let i = 1; i <= count; i += 1) {
    const newest = i === count;
    list.push({
      id: newest && latest !== null ? latest : `older-${i}`,
      displayNumber: `W${String(i).padStart(3, '0')}`,
      serviceType: 'WALK_IN',
      sequence: i,
      issuedAt: new Date(Date.parse('2026-10-08T02:00:00.000Z') + i * 60_000).toISOString(),
    });
  }
  return list;
}

/** The store's answer: a new data object every time, like a `changed: true` poll. */
function answer(branch: string, over: Partial<QueueBoardData> = {}) {
  const s = queues().store(branch);
  s.setError(undefined);
  const data: QueueBoardData = {
    waiting: [],
    called: [],
    myCounter: null,
    ...over,
  };
  s.setState(data);
}

function fail(branch: string, error: QueueError) {
  queues().store(branch).setError(error);
}

/** A `changed: false` poll after a failure: the error clears, `state` stays the same object. */
function recover(branch: string) {
  queues().store(branch).setError(undefined);
}

// ── fake audio, locks, storage, matchMedia ───────────────────────────────────

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

/** A viewport `width` px wide: `matchMedia` answers max-width / min-width queries for it. */
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

// ── the page ─────────────────────────────────────────────────────────────────

// `queue_board` names no branch: the card takes the branch's name from `me` (QU-6).
const NOK_V3 = {
  id: 'E7',
  role: 'FRONT',
  branch: 'B1',
  name: 'Nok',
  branchName: 'Silom',
  branches: [
    { id: 'B1', code: 'B1', name: 'Silom' },
    { id: 'B2', code: 'B2', name: 'Ratchada' },
  ],
};
const NOK: Me = meOf(NOK_V3);

const TH = {
  aria: 'แจ้งเตือนคิวลูกค้า',
  title: 'XC · คิวหน้าร้าน',
  checking: 'กำลังตรวจคิว…',
  checkingBranch: 'กำลังตรวจคิวของสาขา…',
  fold: 'ย่อการแจ้งเตือน',
  open: 'เปิดคิว / รับเรื่อง →',
  waiting: (n: number, name: string) => `มีคิวลูกค้ารอ ${n} รายการ · ${name}`,
  none: (name: string) => `ไม่มีคิวลูกค้ารอ · ${name}`,
  offline: 'เชื่อมต่อไม่ได้ · กำลังลองใหม่',
  expired: 'เซสชันสิ้นสุด กรุณาเข้าสู่ระบบใหม่',
  switched: 'บัญชีเปลี่ยนในหน้าต่างอื่น กรุณาเข้าสู่ระบบใหม่ก่อนทำรายการ',
  signIn: 'เข้าสู่ระบบอีกครั้ง →',
  soundOn: 'เปิดเสียงแจ้งเตือน',
  soundIsOn: 'เสียงเปิด · กดปิด',
  test: 'ทดสอบเสียง',
  hintFirst: 'กดเปิดเสียงครั้งแรกเพื่อให้เบราว์เซอร์อนุญาต',
  hintOn: 'แจ้งเตือนคิวใหม่ · ลดเสียงซ้ำระหว่างแท็บ',
  hintOff: 'ปิดเสียงแล้ว ยังแสดงการแจ้งเตือน',
  hintFail: 'เปิดเสียงไม่ได้ กรุณาตรวจการตั้งค่าเสียงเบราว์เซอร์',
  testOn: 'เสียงทดสอบ · การแจ้งเตือนเปิดอยู่',
  testOff: 'เสียงทดสอบ · กดเปิดเสียงเพื่อรับแจ้งเตือนคิว',
  testFail: 'เล่นเสียงไม่ได้ กรุณาตรวจลำโพงและเบราว์เซอร์',
};

type CardModule = typeof import('./QueueCard');

interface Page {
  card: CardModule;
  i18n: typeof import('../i18n');
  en: typeof import('../i18n/en')['en'];
  calls: { signOut: number; dispose: number };
  setMe(me: Me | undefined): void;
}

const disposers: Array<() => void> = [];
let fetchSpy: ReturnType<typeof vi.fn>;

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

/** A fresh page (its own sound and dock), with the fake session signed in as `me`. */
async function openPage(me: Me | undefined = NOK, lang: 'th' | 'en' = 'th'): Promise<Page> {
  vi.resetModules();
  const i18n = await import('../i18n');
  i18n.setLang(lang);
  const card = await import('./QueueCard');
  queues().reset();
  const { en } = await import('../i18n/en');
  const [meSig, setMe] = createSignal<Me | undefined>(me);
  const calls = { signOut: 0, dispose: 0 };
  const session: SessionStore = {
    me: meSig,
    signOut: async () => {
      calls.signOut += 1;
    },
    dispose: () => {
      calls.dispose += 1;
    },
  };
  const page: Page = { card, i18n, en, calls, setMe: (m) => setMe(() => m) };
  mountCard(page, session);
  return page;
}

function mountCard(page: Page, session: SessionStore) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const { QueueCard } = page.card;
  disposers.push(render(() => <QueueCard session={session} />, host));
}

async function mountControls(): Promise<HTMLElement> {
  const { QueueSoundControls } = await import('./QueueSoundControls');
  const host = document.createElement('div');
  host.setAttribute('data-test', 'controls');
  document.body.appendChild(host);
  disposers.push(render((): JSX.Element => <QueueSoundControls />, host));
  return host;
}

function cards(label = TH.aria): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>('aside')).filter(
    (a) => a.getAttribute('aria-label') === label,
  );
}

function theCard(label = TH.aria): HTMLElement {
  const found = cards(label);
  if (found.length !== 1) throw new Error(`expected one card labelled ${label}, found ${found.length}`);
  return found[0];
}

async function awaitCard(label = TH.aria): Promise<HTMLElement> {
  for (let i = 0; i < 100; i += 1) {
    const list = cards(label);
    if (list.length === 1) return list[0];
    await Promise.resolve();
  }
  return theCard(label);
}

function shown(el: Element | null | undefined): boolean {
  if (!el || !el.isConnected) return false;
  for (let e: Element | null = el; e; e = e.parentElement) {
    if (e.hasAttribute('hidden')) return false;
    if (getComputedStyle(e).display === 'none') return false;
  }
  return true;
}

/** The element inside `root` whose whole text is `text` and that has no element child. */
function leaf(root: Element, text: string): HTMLElement | undefined {
  return Array.from(root.querySelectorAll<HTMLElement>('*')).find(
    (e) => e.children.length === 0 && (e.textContent ?? '').trim() === text,
  );
}

function statusLine(root = theCard()): HTMLElement {
  const s = root.querySelector<HTMLElement>('[role="status"]');
  if (!s) throw new Error('no role=status in the card');
  return s;
}

function foldButton(root = theCard(), label = TH.fold): HTMLButtonElement {
  const b = Array.from(root.querySelectorAll('button')).find((x) => x.getAttribute('aria-label') === label);
  if (!b) throw new Error(`no fold button labelled ${label}`);
  return b;
}

function button(root: Element, text: string): HTMLButtonElement | undefined {
  return Array.from(root.querySelectorAll('button')).find((b) => (b.textContent ?? '').trim() === text);
}

function link(root: Element, text: string): HTMLAnchorElement | undefined {
  return Array.from(root.querySelectorAll('a')).find((a) => (a.textContent ?? '').trim() === text);
}

function folded(root = theCard()): boolean {
  return foldButton(root).getAttribute('aria-expanded') === 'false';
}

beforeAll(async () => {
  await import('../i18n');
  await import('./QueueCard');
  await import('../i18n/en');
  await import('./QueueSoundControls');
}, 60_000);

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
  fetchSpy = vi.fn(async () => new Response('{}', { status: 503 }));
  vi.stubGlobal('fetch', fetchSpy);
});

afterEach(() => {
  while (disposers.length) disposers.pop()?.();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  for (const name of ['AudioContext', 'matchMedia', 'localStorage']) {
    if (Object.getOwnPropertyDescriptor(window, name)?.configurable) {
      delete (window as unknown as Record<string, unknown>)[name];
    }
  }
  delete (navigator as unknown as Record<string, unknown>).locks;
});

/** Requests the card made: anything but the shared session's `me` read. */
function ownRequests(): string[] {
  return fetchSpy.mock.calls
    .map((c) => String((c as unknown[])[0] instanceof Request ? ((c as unknown[])[0] as Request).url : (c as unknown[])[0]))
    .filter((u) => !u.includes('/api/v3/views/me'));
}

// ── C1 ───────────────────────────────────────────────────────────────────────

describe('QueueCard C1: the card before the first answer', () => {
  it('an <aside> labelled แจ้งเตือนคิวลูกค้า: badge —, title XC · คิวหน้าร้าน, a polite status กำลังตรวจคิว…, fold button −', async () => {
    await openPage();
    const card = await awaitCard();
    expect(card.tagName).toBe('ASIDE');
    expect(leaf(card, '—')).toBeDefined();
    expect(leaf(card, TH.title)).toBeDefined();
    const status = statusLine(card);
    expect(status.getAttribute('aria-live')).toBe('polite');
    expect(status.textContent?.trim()).toBe(TH.checking);
    const fold = foldButton(card);
    expect(fold.textContent?.trim()).toBe('−');
    expect(fold.getAttribute('aria-expanded')).toBe('true');
    expect(card.classList.contains('waiting')).toBe(false);
  });

  it('makes no request of its own and never signs out', async () => {
    const page = await openPage();
    await settle();
    answer('B1', { waiting: waiting(2, 'kq-1') });
    fail('B1', { kind: 'ended', reason: 'expired' });
    await settle();
    expect(ownRequests()).toEqual([]);
    expect(page.calls).toEqual({ signOut: 0, dispose: 0 });
  });
});

// ── C2 ───────────────────────────────────────────────────────────────────────

describe('QueueCard C2: folding', () => {
  it('a viewport of 700 px at mount: folded, +, aria-expanded false; link, sound buttons and hint hidden', async () => {
    viewport(700);
    await openPage();
    await settle();
    const card = theCard();
    const fold = foldButton(card);
    expect(fold.textContent?.trim()).toBe('+');
    expect(fold.getAttribute('aria-expanded')).toBe('false');
    expect(shown(link(card, TH.open))).toBe(false);
    expect(shown(button(card, TH.soundOn))).toBe(false);
    expect(shown(button(card, TH.test))).toBe(false);
    expect(shown(leaf(card, TH.hintFirst))).toBe(false);
    // the top row stays
    expect(shown(statusLine(card))).toBe(true);
    expect(shown(fold)).toBe(true);
    expect(shown(leaf(card, TH.title))).toBe(true);
  });

  it('a viewport of 701 px at mount: unfolded, and everything is shown', async () => {
    viewport(701);
    await openPage();
    await settle();
    const card = theCard();
    expect(folded(card)).toBe(false);
    expect(shown(link(card, TH.open))).toBe(true);
    expect(shown(button(card, TH.soundOn))).toBe(true);
    expect(shown(button(card, TH.test))).toBe(true);
    expect(shown(leaf(card, TH.hintFirst))).toBe(true);
  });

  it('a browser without matchMedia: unfolded, nothing throws', async () => {
    expect('matchMedia' in window && typeof window.matchMedia === 'function').toBe(false);
    await openPage();
    await settle();
    expect(folded()).toBe(false);
  });

  it('the fold button folds and unfolds', async () => {
    await openPage();
    await settle();
    const card = theCard();
    foldButton(card).click();
    await settle();
    expect(foldButton(card).textContent?.trim()).toBe('+');
    expect(folded(card)).toBe(true);
    expect(shown(link(card, TH.open))).toBe(false);
    expect(shown(button(card, TH.soundOn))).toBe(false);
    foldButton(card).click();
    await settle();
    expect(foldButton(card).textContent?.trim()).toBe('−');
    expect(folded(card)).toBe(false);
    expect(shown(link(card, TH.open))).toBe(true);
    expect(shown(button(card, TH.soundOn))).toBe(true);
  });
});

// ── C3 ───────────────────────────────────────────────────────────────────────

describe('QueueCard C3: the link to the queue page', () => {
  for (const role of ['FRONT', 'BACKEND', 'MANAGER']) {
    it(`${role}: เปิดคิว / รับเรื่อง → to /kiosk-queue.html?branch=<branch>, in a new tab`, async () => {
      await openPage(meOf({ ...NOK_V3, role }));
      await settle();
      const a = link(theCard(), TH.open);
      expect(shown(a)).toBe(true);
      expect(a?.getAttribute('href')).toBe('/kiosk-queue.html?branch=B1');
      expect(a?.getAttribute('target')).toBe('_blank');
      expect(a?.getAttribute('rel') ?? '').toContain('noopener');
    });
  }

  // QU-6 ruling (a): `queue_board` is read only with QueueOperate, so a session without it gets no card at all (no link).
  for (const role of ['OWNER', 'RATES', 'MONITOR', 'FINANCE', 'SYSTEM', 'DOCUMENTS', 'DOCUMENT_REVIEW']) {
    it(`${role}: no card, so no link`, async () => {
      const who = meOf({ ...NOK_V3, role });
      await openPage(who);
      await settle();
      answer('B1', { waiting: waiting(1, 'kq-1') });
      await settle();
      expect(cards()).toEqual([]);
      expect(Array.from(document.querySelectorAll('a')).filter((a) => shown(a) && (a.textContent ?? '').trim() === TH.open)).toEqual([]);
    });
  }

  // #1030 §3.10 (gap 2 of claude/dell-claude-6's review): the link follows `permissions`, never a role
  // name. Hand-built `me`s, because `meOf` always derives the permissions from the roles.
  it('a TELLER row at another branch whose permissions lack QueueOperate: no card, so no link (QU-6 ruling (a))', async () => {
    await openPage({ ...NOK, roles: [{ role: 'TELLER', branchId: 'B2' }], permissions: ['SignedIn'] } as Me);
    await settle();
    expect(cards()).toEqual([]);
  });

  it('a FINANCE row whose permissions hold QueueOperate: the link', async () => {
    await openPage({ ...NOK, roles: [{ role: 'FINANCE', branchId: 'B1' }], permissions: ['QueueOperate', 'SignedIn'] } as Me);
    await settle();
    expect(shown(link(theCard(), TH.open))).toBe(true);
  });

  it('the branch is encoded, and the link follows a branch change', async () => {
    await openPage();
    queues().setBranch('B 1&x');
    await settle();
    expect(link(theCard(), TH.open)?.getAttribute('href')).toBe('/kiosk-queue.html?branch=B%201%26x');
    queues().setBranch('B2');
    await settle();
    expect(link(theCard(), TH.open)?.getAttribute('href')).toBe('/kiosk-queue.html?branch=B2');
  });
});

// ── C4 ───────────────────────────────────────────────────────────────────────

describe('QueueCard C4: each answer', () => {
  it('count 0 → 2 while folded: badge 2, class waiting, the waiting line, title XC · <name>, and it unfolds', async () => {
    viewport(390);
    await openPage();
    await settle();
    expect(folded()).toBe(true);
    answer('B1', { waiting: waiting(0, null) });
    await settle();
    expect(folded()).toBe(true);
    answer('B1', { waiting: waiting(2, 'kq-2') });
    await settle();
    const card = theCard();
    expect(leaf(card, '2')).toBeDefined();
    expect(card.classList.contains('waiting')).toBe(true);
    expect(statusLine(card).textContent?.trim()).toBe(TH.waiting(2, 'Silom'));
    expect(leaf(card, 'XC · Silom')).toBeDefined();
    expect(leaf(card, TH.title)).toBeUndefined();
    expect(folded(card)).toBe(false);
    expect(foldButton(card).textContent?.trim()).toBe('−');
  });

  it('nothing waiting: badge 0, no class waiting, ไม่มีคิวลูกค้ารอ · <name>, and it stays folded', async () => {
    viewport(390);
    await openPage();
    await settle();
    answer('B1', { waiting: waiting(0, null) });
    await settle();
    const card = theCard();
    expect(leaf(card, '0')).toBeDefined();
    expect(card.classList.contains('waiting')).toBe(false);
    expect(statusLine(card).textContent?.trim()).toBe(TH.none('Silom'));
    expect(folded(card)).toBe(true);
  });

  it('the same line again does not unfold it again; a new line with a waiting count does', async () => {
    await openPage();
    await settle();
    answer('B1', { waiting: waiting(2, 'kq-2') });
    await settle();
    foldButton().click();
    await settle();
    expect(folded()).toBe(true);

    answer('B1', { waiting: waiting(2, 'kq-2') });
    await settle();
    expect(folded()).toBe(true);
    // one request served and another arrived: same count, same line, new latest — still folded
    answer('B1', { waiting: waiting(2, 'kq-3') });
    await settle();
    expect(folded()).toBe(true);
    expect(statusLine().textContent?.trim()).toBe(TH.waiting(2, 'Silom'));

    answer('B1', { waiting: waiting(3, 'kq-4') });
    await settle();
    expect(folded()).toBe(false);
    expect(statusLine().textContent?.trim()).toBe(TH.waiting(3, 'Silom'));
  });

  it('a waiting count that drops to 0 keeps it folded', async () => {
    await openPage();
    await settle();
    answer('B1', { waiting: waiting(1, 'kq-1') });
    await settle();
    foldButton().click();
    await settle();
    answer('B1', { waiting: waiting(0, null) });
    await settle();
    expect(folded()).toBe(true);
    expect(theCard().classList.contains('waiting')).toBe(false);
  });

  it('a branch change: กำลังตรวจคิวของสาขา… until that branch answers', async () => {
    await openPage();
    await settle();
    answer('B1', { waiting: waiting(1, 'kq-1') });
    await settle();
    queues().setBranch('B2');
    await settle();
    expect(statusLine().textContent?.trim()).toBe(TH.checkingBranch);
    answer('B2', { waiting: waiting(0, null) });
    await settle();
    expect(statusLine().textContent?.trim()).toBe(TH.none('Ratchada'));
    expect(leaf(theCard(), 'XC · Ratchada')).toBeDefined();
    expect(leaf(theCard(), '0')).toBeDefined();
  });
});

// ── C5 ───────────────────────────────────────────────────────────────────────

describe('QueueCard C5: offline', () => {
  it('badge !, no class waiting, เชื่อมต่อไม่ได้ · กำลังลองใหม่; it does not unfold', async () => {
    await openPage();
    await settle();
    answer('B1', { waiting: waiting(2, 'kq-2') });
    await settle();
    foldButton().click();
    await settle();
    fail('B1', { kind: 'offline' });
    await settle();
    const card = theCard();
    expect(leaf(card, '!')).toBeDefined();
    expect(leaf(card, '2')).toBeUndefined();
    expect(card.classList.contains('waiting')).toBe(false);
    expect(statusLine(card).textContent?.trim()).toBe(TH.offline);
    expect(folded(card)).toBe(true);
  });

  it('the next success clears it — even an unchanged one — and the waiting line is new again, so it unfolds (account-bar.js:45 last=\'\')', async () => {
    await openPage();
    await settle();
    answer('B1', { waiting: waiting(2, 'kq-2') });
    await settle();
    foldButton().click();
    fail('B1', { kind: 'offline' });
    await settle();
    recover('B1');
    await settle();
    const card = theCard();
    expect(statusLine(card).textContent?.trim()).toBe(TH.waiting(2, 'Silom'));
    expect(leaf(card, '2')).toBeDefined();
    expect(card.classList.contains('waiting')).toBe(true);
    expect(folded(card)).toBe(false);
  });
});

// ── C6 ───────────────────────────────────────────────────────────────────────

describe('QueueCard C6: the session ended', () => {
  it('expired: เซสชันสิ้นสุด กรุณาเข้าสู่ระบบใหม่ and เข้าสู่ระบบอีกครั้ง → to /login', async () => {
    const page = await openPage();
    await settle();
    answer('B1', { waiting: waiting(1, 'kq-1') });
    await settle();
    fail('B1', { kind: 'ended', reason: 'expired' });
    await settle();
    const card = theCard();
    expect(statusLine(card).textContent?.trim()).toBe(TH.expired);
    const a = link(card, TH.signIn);
    expect(shown(a)).toBe(true);
    expect(a?.getAttribute('href')).toBe('/login');
    expect(shown(link(card, TH.open))).toBe(false);
    expect(page.calls.signOut).toBe(0);
  });

  it('switched: บัญชีเปลี่ยนในหน้าต่างอื่น กรุณาเข้าสู่ระบบใหม่ก่อนทำรายการ, with the same link', async () => {
    await openPage();
    await settle();
    fail('B1', { kind: 'ended', reason: 'switched' });
    await settle();
    const card = theCard();
    expect(statusLine(card).textContent?.trim()).toBe(TH.switched);
    expect(link(card, TH.signIn)?.getAttribute('href')).toBe('/login');
  });

  it('a role without QueueOperate (RATES) has no card, so no sign-in link either (QU-6 ruling (a))', async () => {
    await openPage(meOf({ ...NOK_V3, role: 'RATES' }));
    await settle();
    fail('B1', { kind: 'ended', reason: 'expired' });
    await settle();
    expect(cards()).toEqual([]);
  });
});

// ── C7 ───────────────────────────────────────────────────────────────────────

describe('QueueSoundControls C7 (account-bar.js:28-30, :35-36)', () => {
  it('the card carries the controls: เปิดเสียงแจ้งเตือน (aria-pressed false), ทดสอบเสียง and the first hint', async () => {
    await openPage();
    await settle();
    const card = theCard();
    const sound = button(card, TH.soundOn);
    expect(sound?.getAttribute('aria-pressed')).toBe('false');
    expect(sound?.getAttribute('type')).toBe('button');
    expect(button(card, TH.test)?.getAttribute('type')).toBe('button');
    expect(shown(leaf(card, TH.hintFirst))).toBe(true);
  });

  it('pressing it: sound on, the button says เสียงเปิด · กดปิด (aria-pressed true), the hint changes, one chime', async () => {
    await openPage();
    await settle();
    const card = theCard();
    button(card, TH.soundOn)?.click();
    await settle();
    const on = button(card, TH.soundIsOn);
    expect(on?.getAttribute('aria-pressed')).toBe('true');
    expect(leaf(card, TH.hintOn)).toBeDefined();
    expect(audio.chimes()).toBe(1);
  });

  it('pressing it again: off, the first text back, ปิดเสียงแล้ว ยังแสดงการแจ้งเตือน, no chime', async () => {
    await openPage();
    await settle();
    const card = theCard();
    button(card, TH.soundOn)?.click();
    await settle();
    button(card, TH.soundIsOn)?.click();
    await settle();
    expect(button(card, TH.soundOn)?.getAttribute('aria-pressed')).toBe('false');
    expect(leaf(card, TH.hintOff)).toBeDefined();
    expect(audio.chimes()).toBe(1);
  });

  it('the AudioContext does not reach running: เปิดเสียงไม่ได้…, sound stays off', async () => {
    audio.resumeTo = 'suspended';
    await openPage();
    await settle();
    const card = theCard();
    button(card, TH.soundOn)?.click();
    await settle();
    expect(leaf(card, TH.hintFail)).toBeDefined();
    expect(button(card, TH.soundOn)?.getAttribute('aria-pressed')).toBe('false');
    expect(button(card, TH.soundIsOn)).toBeUndefined();
    expect(audio.chimes()).toBe(0);
  });

  it('ทดสอบเสียง with sound off: one chime, the off test hint, sound still off', async () => {
    await openPage();
    await settle();
    const card = theCard();
    button(card, TH.test)?.click();
    await settle();
    expect(audio.chimes()).toBe(1);
    expect(leaf(card, TH.testOff)).toBeDefined();
    expect(button(card, TH.soundOn)?.getAttribute('aria-pressed')).toBe('false');
  });

  it('ทดสอบเสียง with sound on: the on test hint', async () => {
    await openPage();
    await settle();
    const card = theCard();
    button(card, TH.soundOn)?.click();
    await settle();
    button(card, TH.test)?.click();
    await settle();
    expect(audio.chimes()).toBe(2);
    expect(leaf(card, TH.testOn)).toBeDefined();
  });

  it('ทดสอบเสียง when audio cannot start: เล่นเสียงไม่ได้ กรุณาตรวจลำโพงและเบราว์เซอร์', async () => {
    audio.resumeTo = 'suspended';
    await openPage();
    await settle();
    const card = theCard();
    button(card, TH.test)?.click();
    await settle();
    expect(leaf(card, TH.testFail)).toBeDefined();
    expect(audio.chimes()).toBe(0);
  });

  it('two controls on one page share one sound (one per page — the card and #471’s bar)', async () => {
    await openPage();
    await settle();
    const other = await mountControls();
    button(theCard(), TH.soundOn)?.click();
    await settle();
    expect(button(other, TH.soundIsOn)?.getAttribute('aria-pressed')).toBe('true');
    expect(leaf(other, TH.hintOn)).toBeDefined();
    button(other, TH.soundIsOn)?.click();
    await settle();
    expect(button(theCard(), TH.soundOn)?.getAttribute('aria-pressed')).toBe('false');
    expect(audio.chimes()).toBe(1);
  });
});

// ── C9 from the card ─────────────────────────────────────────────────────────

describe('QueueCard C9: it announces each answer', () => {
  it('sound on and a new latest: one chime, stored at xc.queue.sound.<me().id>.<branch>; the same latest again: none', async () => {
    await openPage();
    await settle();
    button(theCard(), TH.soundOn)?.click();
    await settle();
    expect(audio.chimes()).toBe(1);

    answer('B1', { waiting: waiting(1, 'kq-1') });
    await settle();
    expect(audio.chimes()).toBe(2);
    expect(storage.map.get('xc.queue.sound.E7.B1')).toBe('kq-1');

    answer('B1', { waiting: waiting(2, 'kq-1') });
    await settle();
    expect(audio.chimes()).toBe(2);

    answer('B1', { waiting: waiting(2, 'kq-2') });
    await settle();
    expect(audio.chimes()).toBe(3);
    expect(storage.map.get('xc.queue.sound.E7.B1')).toBe('kq-2');
  });

  it('sound off: an answer chimes nothing and stores nothing', async () => {
    await openPage();
    await settle();
    answer('B1', { waiting: waiting(1, 'kq-1') });
    await settle();
    expect(audio.chimes()).toBe(0);
    expect(storage.map.has('xc.queue.sound.E7.B1')).toBe(false);
  });

  it('the key follows the branch the card follows', async () => {
    await openPage();
    await settle();
    button(theCard(), TH.soundOn)?.click();
    await settle();
    queues().setBranch('B2');
    await settle();
    answer('B2', { waiting: waiting(1, 'kq-7') });
    await settle();
    expect(storage.map.get('xc.queue.sound.E7.B2')).toBe('kq-7');
    expect(storage.map.has('xc.queue.sound.E7.B1')).toBe(false);
  });

  it('switching sound on while a request already waits announces it, as v2’s next poll did (:46 → :37)', async () => {
    await openPage();
    await settle();
    answer('B1', { waiting: waiting(1, 'kq-1') });
    await settle();
    button(theCard(), TH.soundOn)?.click();
    await settle();
    // the switch-on chime, then the waiting request
    expect(audio.chimes()).toBe(2);
    expect(storage.map.get('xc.queue.sound.E7.B1')).toBe('kq-1');
  });
});

// ── docking (§12.18.8): retired by #833 ───────────────────────────────────────
// #786 removed the only caller (admin's QueueBar); the shared card now floats on every screen, so
// the dock export and its three tests are gone. The card always renders where it is mounted.

// ── language (W10) ───────────────────────────────────────────────────────────

describe('QueueCard texts follow the language (W10)', () => {
  it('lang en: the label, the lines, the link and the sound button come from en; back to th live', async () => {
    const page = await openPage(NOK, 'en');
    const { en } = page;
    await settle();
    const card = theCard(en['queue.card.aria']);
    expect(statusLine(card).textContent?.trim()).toBe(en['queue.card.checking']);
    expect(foldButton(card, en['queue.card.foldAria'])).toBeDefined();
    expect(shown(link(card, en['queue.card.openQueue']))).toBe(true);
    expect(button(card, en['queue.sound.enable'])).toBeDefined();
    expect(leaf(card, en['queue.sound.hintInitial'])).toBeDefined();

    answer('B1', { waiting: waiting(2, 'kq-2') });
    await settle();
    expect(statusLine(card).textContent?.trim()).toBe(
      page.i18n.fill(en['queue.card.hasWaiting'], { count: 2, branchName: 'Silom' }),
    );
    fail('B1', { kind: 'offline' });
    await settle();
    expect(statusLine(card).textContent?.trim()).toBe(en['queue.card.offline']);
    fail('B1', { kind: 'ended', reason: 'switched' });
    await settle();
    expect(statusLine(card).textContent?.trim()).toBe(en['queue.card.endedSwitched']);
    expect(link(card, en['queue.card.signInAgain'])?.getAttribute('href')).toBe('/login');

    page.i18n.setLang('th');
    await settle();
    const thCard = theCard(TH.aria);
    expect(statusLine(thCard).textContent?.trim()).toBe(TH.switched);
    expect(link(thCard, TH.signIn)).toBeDefined();
  });

  it('lang en, nothing waiting and expired', async () => {
    const page = await openPage(NOK, 'en');
    const { en } = page;
    await settle();
    answer('B1', { waiting: waiting(0, null) });
    await settle();
    const card = theCard(en['queue.card.aria']);
    expect(statusLine(card).textContent?.trim()).toBe(page.i18n.fill(en['queue.card.noWaiting'], { branchName: 'Silom' }));
    fail('B1', { kind: 'ended', reason: 'expired' });
    await settle();
    expect(statusLine(card).textContent?.trim()).toBe(en['queue.card.endedExpired']);
  });
});
