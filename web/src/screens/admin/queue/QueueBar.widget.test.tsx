// #786 (web design §12.18.12, QF1–QF3): on admin the shared floating `QueueCard` is the queue notice.
// `QueueBar` no longer docks it and renders no in-page card; it keeps the link interception, the
// toast and the dialog, and the card's `เปิดคิว / รับเรื่อง →` link opens that dialog. Written before
// the implementation by the ticket's author (claude/b060e9). Do not edit, delete or weaken; add
// tests in a file of your own. QueueBar.test.tsx and the QueueCard tests stay as they are.
//
// This file renders the real `QueueCard` (as `AccountBar` does on every screen, admin included) next
// to the real `QueueBar`, with the queue store and the panel replaced by fakes. Nothing here imports
// AdminShell: the card reaches admin through AppShell → AccountBar, which #786 does not change, so
// the only thing that hid it on admin was the dock.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { Me } from '../../../shared/api/generated/common';
import { setLang } from '../../../shared/i18n';
import { queueStore, type QueueError, type QueueStore } from '../../../shared/queue/queue-store';
import type { KioskNotifications } from '../../../shared/queue/wire';
import { QueueCard } from '../../../shared/ui/QueueCard';
import { QueueBar } from './QueueBar';

interface FakeStore extends QueueStore {
  setState(v: KioskNotifications | undefined): void;
  setError(v: QueueError | undefined): void;
}

const fake = vi.hoisted(() => ({ stores: new Map<string, unknown>() }));

vi.mock('../../../shared/queue/queue-store', async () => {
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

vi.mock('../../../shared/ui/QueuePanel', () => ({
  QueuePanel(_props: { branch: string; shown: boolean }): JSX.Element {
    const el = document.createElement('div');
    el.setAttribute('data-fake', 'queue-panel');
    return el;
  },
}));

const OWNER: Me = { kind: 'staff', id: 'emp-nok', role: 'OWNER', branch: 'B1', name: 'Nok', username: 'nok' };

function store(b: string): FakeStore {
  return queueStore(b) as FakeStore;
}

function answer(b: string, over: Partial<KioskNotifications> = {}) {
  const s = store(b);
  s.setError(undefined);
  s.setState({
    serverNow: '2026-10-10T03:00:00.000Z',
    branchId: b,
    branchName: 'Silom',
    count: 0,
    latest: null,
    workingCount: 0,
    working: [],
    employee: { id: OWNER.id, role: OWNER.role },
    ...over,
  });
}

const flush = () => new Promise<void>((r) => setTimeout(r, 0));
let disposers: (() => void)[] = [];

/** The card first (it sits in the shell header), then the bar, as on the admin page. */
async function mountAdmin(start = 'B1', label = 'สาขาสีลม') {
  const [branch, setBranch] = createSignal(start);
  const [branchLabel, setBranchLabel] = createSignal(label);
  const host = document.createElement('div');
  document.body.append(host);
  const session = { me: () => OWNER, signOut: async () => {}, dispose: () => {} };
  const dispose = render(
    () => (
      <>
        <QueueCard session={session} />
        <QueueBar branch={branch} branchLabel={branchLabel} />
      </>
    ),
    host,
  );
  disposers.push(() => {
    dispose();
    host.remove();
  });
  await flush();
  return { setBranch, setBranchLabel };
}

function card(): HTMLElement | undefined {
  return [...document.querySelectorAll<HTMLElement>('aside')].find(
    (a) => a.getAttribute('aria-label') === 'แจ้งเตือนคิวลูกค้า',
  );
}

function cardLink(): HTMLAnchorElement {
  const c = card();
  expect(c, 'the queue card is on the page (not docked)').toBeDefined();
  const a = [...c!.querySelectorAll<HTMLAnchorElement>('a')].find((x) => x.textContent?.trim() === 'เปิดคิว / รับเรื่อง →');
  expect(a, "the card's link เปิดคิว / รับเรื่อง →").toBeDefined();
  return a!;
}

function click(a: HTMLAnchorElement): MouseEvent {
  const ev = new MouseEvent('click', { bubbles: true, cancelable: true });
  a.dispatchEvent(ev);
  return ev;
}

function dialog(): HTMLDialogElement | null {
  return document.querySelector<HTMLDialogElement>('dialog[aria-labelledby]');
}

function dialogOpen(): boolean {
  const d = dialog();
  return !!d && (d.open || d.hasAttribute('open'));
}

function dialogName(): string {
  const id = dialog()?.getAttribute('aria-labelledby');
  return (id ? document.getElementById(id)?.textContent : '')?.trim() ?? '';
}

function queueLink(branch = 'B1'): HTMLAnchorElement {
  const a = document.createElement('a');
  a.href = `/kiosk-queue.html?branch=${branch}`;
  a.textContent = 'คิวลูกค้า';
  document.body.append(a);
  return a;
}

beforeEach(() => {
  setLang('th');
  fake.stores.clear();
  sessionStorage.clear();
  Reflect.deleteProperty(window, 'matchMedia');
  vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 500 })));
});

afterEach(() => {
  for (const d of disposers.splice(0)) d();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
});

describe('admin shows the floating card (QF1, QF2)', () => {
  it('W1: with QueueBar mounted, the queue card is on the page, once', async () => {
    await mountAdmin();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    const cards = [...document.querySelectorAll('aside')].filter(
      (a) => a.getAttribute('aria-label') === 'แจ้งเตือนคิวลูกค้า',
    );
    expect(cards, 'QueueBar does not dock the card').toHaveLength(1);
  });

  it('W2: QueueBar renders no in-page card: no aside.admin-queue-bar, nothing labelled คิวลูกค้าและผู้ทำรายการ', async () => {
    await mountAdmin();
    answer('B1', {
      count: 1,
      latest: 'K1',
      workingCount: 1,
      working: [{ id: 'K1', queue: 'Q-1', staff: 'nok', startedAt: null, status: 'ACCEPTED' }],
    });
    await flush();
    expect(document.querySelector('.admin-queue-bar')).toBeNull();
    expect(document.querySelector('[aria-label="คิวลูกค้าและผู้ทำรายการ"]')).toBeNull();
    const opens = [...document.querySelectorAll('a, button')].filter((n) => n.textContent?.trim() === 'เปิดคิว / รับเรื่อง →');
    expect(opens.map((n) => n.tagName), "the only เปิดคิว / รับเรื่อง → is the card's link").toEqual(['A']);
  });
});

describe("the card's action opens the queue dialog (QF3)", () => {
  it("W3: the card's link is caught and opens the dialog named by the picked branch", async () => {
    await mountAdmin('B1', 'สาขาสีลม');
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    expect(dialogOpen()).toBe(false);
    const ev = click(cardLink());
    await flush();
    expect(ev.defaultPrevented, 'no new tab').toBe(true);
    expect(dialogOpen()).toBe(true);
    expect(dialogName()).toBe('คิวลูกค้า · สาขาสีลม');
    expect(dialog()!.querySelector('[data-fake="queue-panel"]')).not.toBeNull();
  });

  it('W4: ปิด × returns focus to the link that opened the dialog', async () => {
    await mountAdmin();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    const a = cardLink();
    a.focus();
    click(a);
    await flush();
    const close = [...dialog()!.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'ปิด ×');
    expect(close, 'the dialog has ปิด ×').toBeDefined();
    close!.focus();
    close!.click();
    await flush();
    expect(dialogOpen()).toBe(false);
    expect(document.activeElement, "focus is back on the card's link").toBe(cardLink());
  });

  it('W5: offline, the link is caught but opens nothing; after an answer it opens the dialog', async () => {
    await mountAdmin();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    store('B1').setError({ kind: 'offline' });
    await flush();
    const ev = click(cardLink());
    await flush();
    expect(ev.defaultPrevented).toBe(true);
    expect(dialogOpen(), 'offline').toBe(false);
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    click(cardLink());
    await flush();
    expect(dialogOpen(), 'online again').toBe(true);
  });

  it('W6: a branch change: the link opens nothing until the new branch answers', async () => {
    const { setBranch, setBranchLabel } = await mountAdmin('B1', 'สาขาสีลม');
    answer('B1', { count: 0 });
    await flush();
    setBranchLabel('สาขาสยาม');
    setBranch('B2');
    await flush();
    click(queueLink('B2'));
    await flush();
    expect(dialogOpen(), 'B2 not answered yet').toBe(false);
    answer('B2', { branchName: 'Siam', count: 0 });
    await flush();
    click(queueLink('B2'));
    await flush();
    expect(dialogOpen()).toBe(true);
    expect(dialogName()).toBe('คิวลูกค้า · สาขาสยาม');
  });

  it('W7: after a session end a queue link opens nothing, even after later answers (v2 `locked`)', async () => {
    await mountAdmin();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    store('B1').setError({ kind: 'ended', reason: 'expired' });
    await flush();
    answer('B1', { count: 2, latest: 'K2' });
    await flush();
    const ev = click(queueLink());
    await flush();
    expect(ev.defaultPrevented).toBe(true);
    expect(dialogOpen()).toBe(false);
  });
});
