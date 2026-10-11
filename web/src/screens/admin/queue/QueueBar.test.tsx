// V2-queue (#471): the admin's customer-queue bar, its dialog and its toast — design
// docs/design/xchange-v3-web.md §12.12 and §12.18.8 (v2: src/devices-public/admin-queue.js:1-24
// of xchange-v2-source@4d0bdbb). Written before the implementation by the ticket's preparer
// (claude/b060e9). Do not edit, delete or weaken; add tests in a file of your own.
//
// The API this file pins:
//
//   web/src/screens/admin/queue/QueueBar.tsx
//     export function QueueBar(props: {
//       branch: Accessor<string>;        // the shell's picked branch id
//       branchLabel: Accessor<string>;   // the picked option's text (v2 `selectedOptions[0].textContent`)
//     }): JSX.Element;
//   web/src/screens/admin/queue/i18n.ts
//     export const th, en              // namespace `admin.queue.*` (W10); th = v2's Thai verbatim (W6)
//
// The bar makes no request of its own. It reads the queue store of #486
// (`web/src/shared/queue/queue-store.ts`: `useQueueStore`, `setQueueBranch`), which this file
// replaces with a fake the test drives — the same writes the real store makes (an answer clears
// `error` then sets `state`; a failure sets `error` and keeps `state`). `QueuePanel` (#486)
// and `QueueSoundControls` (#487) are replaced by recorders: the claim, the tabs and
// the pager are the panel's and are not tested here. `fetch` is watched and must never be called.
//
// "Shown" means connected, with no `hidden` on it or an ancestor, no inline `display: none`, and,
// for a `<dialog>`, its `open` set (jsdom has no `showModal()`; set the attribute, or call
// `showModal()` only where it exists). A part not rendered at all is not shown either.
//
// Retired by #786 (web design §12.18.12, lane controller ruling B): admin shows the shared floating
// `QueueCard` and no in-page bar. Removed: Q1–Q3 and Q5 (the bar's title, status, worker lines
// and sound controls) and Q4 (docking the card). The dialog is now opened through the card's
// `/kiosk-queue.html` link (QF3), so the tests below click such a link where they clicked the
// bar's button, and the assertions on that button (`disabled`, focus on close) and on the bar's
// status line are gone. Their replacements are in QueueBar.widget.test.tsx and
// ../../../shared/ui/QueueCard.float.test.tsx.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// `employee {id, role: 'OWNER'}` → `employeeOf(meOf(…))` with MANAGER (BRANCH_MANAGER), as in
// QueueBar.widget.test.tsx: under ruling A on #1030 the former owner holds no `QueueOperate`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { queueStore, type QueueError, type QueueStore } from '../../../shared/queue/queue-store';
import type { KioskNotifications } from '../../../shared/queue/wire';
import { employeeOf, meOf } from '../../../test-support/me';

type BarModule = typeof import('./QueueBar');
type TextsModule = typeof import('./i18n');

const bars = import.meta.glob<BarModule>('./QueueBar.tsx');
const texts = import.meta.glob<TextsModule>('./i18n.ts');
const shellSources = import.meta.glob<string>('../AdminShell.tsx', {
  query: '?raw',
  import: 'default',
  eager: true,
});

async function barModule(): Promise<BarModule> {
  const load = bars['./QueueBar.tsx'];
  expect(load, 'web/src/screens/admin/queue/QueueBar.tsx exists').toBeDefined();
  return load!();
}

async function textsModule(): Promise<TextsModule> {
  const load = texts['./i18n.ts'];
  expect(load, 'web/src/screens/admin/queue/i18n.ts exists').toBeDefined();
  return load!();
}

// ── fakes ────────────────────────────────────────────────────────────────────

interface FakeStore extends QueueStore {
  setState(v: KioskNotifications | undefined): void;
  setError(v: QueueError | undefined): void;
}

const fake = vi.hoisted(() => ({
  stores: new Map<string, unknown>(),
  queueBranchCalls: [] as string[],
  panels: [] as { branch: string; shown: boolean; alive: boolean }[],
  soundControls: 0,
}));

vi.mock('../../../shared/queue/queue-store', async () => {
  const solid = await import('solid-js');
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
    queueStore: (b: string) => store(b),
    useQueueStore: (b: Accessor<string>) => () => store(b()),
    setQueueBranch: (b: string) => {
      fake.queueBranchCalls.push(b);
    },
    queueBranch: () => 'B1',
  };
});

vi.mock('../../../shared/ui/QueuePanel', async () => {
  const solid = await import('solid-js');
  return {
    QueuePanel(props: { branch: string; shown: boolean }): JSX.Element {
      const entry = { branch: props.branch, shown: props.shown, alive: true };
      fake.panels.push(entry);
      solid.createEffect(() => {
        entry.branch = props.branch;
        entry.shown = props.shown;
      });
      solid.onCleanup(() => {
        entry.alive = false;
      });
      const el = document.createElement('div');
      el.setAttribute('data-fake', 'queue-panel');
      return el;
    },
  };
});

vi.mock('../../../shared/ui/QueueCard', () => ({
  QueueCard: () => document.createElement('div'),
}));

vi.mock('../../../shared/ui/QueueSoundControls', () => ({
  QueueSoundControls: () => {
    fake.soundControls += 1;
    const el = document.createElement('div');
    el.setAttribute('data-fake', 'sound-controls');
    return el;
  },
}));

function store(b: string): FakeStore {
  return queueStore(b) as FakeStore;
}

const EMPLOYEE = employeeOf(meOf({ id: 'emp-nok', role: 'MANAGER', branch: 'B1', name: 'Nok' }));

/** A fresh answer object every time, as a `changed: true` poll. */
function answer(b: string, over: Partial<KioskNotifications> = {}) {
  const s = store(b);
  s.setError(undefined);
  s.setState({
    serverNow: '2026-10-09T03:00:00.000Z',
    branchId: b,
    branchName: 'Silom',
    count: 0,
    latest: null,
    workingCount: 0,
    working: [],
    employee: EMPLOYEE,
    ...over,
  });
}

// ── helpers ──────────────────────────────────────────────────────────────────

function shown(el: Element | null | undefined): boolean {
  if (!el || !el.isConnected) return false;
  for (let n: Element | null = el; n; n = n.parentElement) {
    if (n.hasAttribute('hidden')) return false;
    if ((n as HTMLElement).style?.display === 'none') return false;
    if (n instanceof HTMLDialogElement && !n.open) return false;
  }
  return true;
}

function button(root: ParentNode, name: string): HTMLButtonElement | undefined {
  return [...root.querySelectorAll<HTMLButtonElement>('button')].find(
    (b) => (b.getAttribute('aria-label') ?? b.textContent?.trim()) === name,
  );
}

/** Clicks a link to the queue page, as the card's `เปิดคิว / รับเรื่อง →` is (QF3). */
function openQueue(branch = 'B1'): MouseEvent {
  const a = document.createElement('a');
  a.href = `/kiosk-queue.html?branch=${branch}`;
  a.target = '_blank';
  a.textContent = 'เปิดคิว / รับเรื่อง →';
  document.body.append(a);
  const ev = new MouseEvent('click', { bubbles: true, cancelable: true });
  a.dispatchEvent(ev);
  a.remove();
  return ev;
}

function dialog(): HTMLElement | undefined {
  return [...document.querySelectorAll<HTMLElement>('dialog, [role="dialog"]')].find((d) => {
    const labelled = d.getAttribute('aria-labelledby');
    const name = labelled
      ? document.getElementById(labelled)?.textContent?.trim()
      : d.getAttribute('aria-label') ?? '';
    return (name ?? '').startsWith('คิวลูกค้า · ');
  });
}

function dialogName(): string {
  const d = dialog();
  const labelled = d?.getAttribute('aria-labelledby');
  return (labelled ? document.getElementById(labelled)?.textContent : d?.getAttribute('aria-label'))?.trim() ?? '';
}

function toast(): HTMLElement | undefined {
  const isToast = (n: Element) =>
    /^มีคิวลูกค้ารอ \d+ รายการ · /.test(n.textContent?.trim() ?? '') && button(n, 'ดูคิว') !== undefined;
  // the innermost match, so a wrapper that only starts with the toast's text is not taken for it
  return [...document.querySelectorAll<HTMLElement>('*')].find(
    (n) => isToast(n) && [...n.children].every((c) => !isToast(c)),
  );
}

function livePanels() {
  return fake.panels.filter((p) => p.alive);
}

const flush = () => new Promise<void>((r) => setTimeout(r, 0));

let disposers: (() => void)[] = [];
let fetchSpy: ReturnType<typeof vi.fn>;

async function mount(start = 'B1', label = 'สาขาสีลม') {
  const { QueueBar } = await barModule();
  const [branch, setBranch] = createSignal(start);
  const [branchLabel, setBranchLabel] = createSignal(label);
  const host = document.createElement('div');
  document.body.append(host);
  const dispose = render(() => <QueueBar branch={branch} branchLabel={branchLabel} />, host);
  disposers.push(() => {
    dispose();
    host.remove();
  });
  await flush();
  return { setBranch, setBranchLabel, dispose };
}

beforeEach(() => {
  setLang('th');
  fake.stores.clear();
  fake.queueBranchCalls.length = 0;
  fake.panels.length = 0;
  fake.soundControls = 0;
  sessionStorage.clear();
  fetchSpy = vi.fn(async () => new Response('{}', { status: 500 }));
  vi.stubGlobal('fetch', fetchSpy);
});

afterEach(() => {
  for (const d of disposers.splice(0)) d();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
});

// ── texts ────────────────────────────────────────────────────────────────────

describe('texts (W6, W10)', () => {
  it('Q0: the table holds v2\'s Thai verbatim, with an en twin for every key', async () => {
    const { th, en } = await textsModule();
    const values = Object.values(th);
    for (const v2 of [
      'คิวลูกค้าและผู้ทำรายการ',
      'คิวลูกค้าหน้าร้าน',
      'กำลังตรวจคิว…',
      'กำลังตรวจคิวสาขา…',
      'เปิดคิว / รับเรื่อง →',
      'ปิด ×',
      'ดูคิว',
      'ปิด',
      'เชื่อมต่อคิวไม่ได้ · กำลังลองใหม่',
      'ยังไม่มีข้อมูล',
    ]) {
      expect(values, `th holds ${v2}`).toContain(v2);
    }
    // the templated lines, as v2 builds them (admin-queue.js:14-16), with named placeholders
    expect(values.some((v) => /^รอรับเรื่อง \{\w+\} · กำลังทำรายการ \{\w+\} · \{\w+\}$/.test(v))).toBe(true);
    expect(values.some((v) => /^\{\w+\} กำลังทำรายการ \{\w+\} · เริ่ม \{\w+\}$/.test(v))).toBe(true);
    expect(values.some((v) => /^มีคิวลูกค้ารอ \{\w+\} รายการ · \{\w+\}$/.test(v))).toBe(true);
    expect(values.some((v) => /^คิวลูกค้า · \{\w+\}$/.test(v))).toBe(true);
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
  });
});

// ── the bar ──────────────────────────────────────────────────────────────────

describe('the bar (admin-queue.js:6, :12-16)', () => {
  it('Q6: the bar never calls fetch itself', async () => {
    await mount();
    answer('B1', { count: 2, latest: 'K1' });
    await flush();
    openQueue();
    await flush();
    expect(fetchSpy).not.toHaveBeenCalled();
  });
});

// ── the dialog ───────────────────────────────────────────────────────────────

describe('the dialog (admin-queue.js:7-9)', () => {
  it('Q7: the queue link opens the dialog named by the picked branch and mounts the panel for it', async () => {
    await mount('B1', 'สาขาสีลม');
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    expect(shown(dialog())).toBe(false);
    openQueue();
    await flush();
    expect(shown(dialog())).toBe(true);
    expect(dialogName()).toBe('คิวลูกค้า · สาขาสีลม');
    const live = livePanels();
    expect(live).toHaveLength(1);
    expect(live[0]).toMatchObject({ branch: 'B1', shown: true });
    expect(shown(dialog()!.querySelector('[data-fake="queue-panel"]'))).toBe(true);
  });

  it('Q8: ปิด × closes the dialog and unmounts the panel', async () => {
    await mount();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    openQueue();
    await flush();
    const close = button(dialog()!, 'ปิด ×');
    expect(close, 'the dialog has ปิด ×').toBeDefined();
    close!.click();
    await flush();
    expect(shown(dialog())).toBe(false);
    expect(livePanels().filter((p) => p.shown)).toHaveLength(0);
  });

  it('Q9: a click on a link to /kiosk-queue.html opens the dialog instead of navigating', async () => {
    await mount();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    const a = document.createElement('a');
    a.href = '/kiosk-queue.html?branch=B1';
    a.textContent = 'คิวลูกค้า';
    document.body.append(a);
    const ev = new MouseEvent('click', { bubbles: true, cancelable: true });
    a.dispatchEvent(ev);
    await flush();
    expect(ev.defaultPrevented).toBe(true);
    expect(shown(dialog())).toBe(true);
  });

  it('Q10: after unmount a /kiosk-queue.html link navigates again (the listener is removed)', async () => {
    const { dispose } = await mount();
    dispose();
    const a = document.createElement('a');
    a.href = '/kiosk-queue.html';
    document.body.append(a);
    const ev = new MouseEvent('click', { bubbles: true, cancelable: true });
    a.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
  });
});

// ── the toast ────────────────────────────────────────────────────────────────

describe('the toast (admin-queue.js:10, :16-17)', () => {
  it('Q11: a new request while the dialog is closed shows the toast once and stores its key', async () => {
    await mount();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    const t1 = toast();
    expect(shown(t1)).toBe(true);
    expect(t1!.textContent).toContain('มีคิวลูกค้ารอ 1 รายการ · Silom');
    expect(sessionStorage.getItem('xc.queue.popup.emp-nok.B1')).toBe('K1');

    button(t1!, 'ปิด')!.click();
    await flush();
    expect(shown(toast())).toBe(false);

    answer('B1', { count: 1, latest: 'K1' }); // same request, a new poll
    await flush();
    expect(shown(toast()), 'the same latest does not toast twice').toBe(false);

    answer('B1', { count: 2, latest: 'K2' });
    await flush();
    expect(shown(toast())).toBe(true);
    expect(toast()!.textContent).toContain('มีคิวลูกค้ารอ 2 รายการ · Silom');
  });

  it('Q12: a request already toasted in this tab (sessionStorage) does not toast after a reload', async () => {
    sessionStorage.setItem('xc.queue.popup.emp-nok.B1', 'K1');
    await mount();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    expect(shown(toast())).toBe(false);
  });

  it('Q13: ดูคิว hides the toast and opens the dialog', async () => {
    await mount();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    button(toast()!, 'ดูคิว')!.click();
    await flush();
    expect(shown(toast())).toBe(false);
    expect(shown(dialog())).toBe(true);
  });

  it('Q14: no toast while the dialog is open; the key is still stored', async () => {
    await mount();
    answer('B1', { count: 0 });
    await flush();
    openQueue();
    await flush();
    answer('B1', { count: 1, latest: 'K3' });
    await flush();
    expect(shown(toast())).toBe(false);
    expect(sessionStorage.getItem('xc.queue.popup.emp-nok.B1')).toBe('K3');
  });

  it('Q15: when the waiting count falls to 0 the toast hides', async () => {
    await mount();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    expect(shown(toast())).toBe(true);
    answer('B1', { count: 0, latest: 'K1' });
    await flush();
    expect(shown(toast())).toBe(false);
  });
});

// ── errors and the session ───────────────────────────────────────────────────

describe('errors (admin-queue.js:19, account-bar.js lock)', () => {
  it('Q16: offline hides the toast', async () => {
    await mount();
    answer('B1', {
      count: 1,
      latest: 'K1',
      workingCount: 1,
      working: [{ id: 'K1', queue: 'Q-1', staff: 'nok', startedAt: null, status: 'ACCEPTED' }],
    });
    await flush();
    store('B1').setError({ kind: 'offline' });
    await flush();
    expect(shown(toast())).toBe(false);
  });

  it('Q17: a session end closes the dialog', async () => {
    await mount();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    openQueue();
    await flush();
    expect(shown(dialog())).toBe(true);
    store('B1').setError({ kind: 'ended', reason: 'expired' });
    await flush();
    expect(shown(dialog())).toBe(false);
  });
});

// ── branch change ────────────────────────────────────────────────────────────

describe('branch change (admin-queue.js:14, :20, :22)', () => {
  it('Q18: the bar tells the store its branch on mount and on every change', async () => {
    const { setBranch } = await mount('B1');
    expect(fake.queueBranchCalls.at(-1)).toBe('B1');
    setBranch('B2');
    await flush();
    expect(fake.queueBranchCalls.at(-1)).toBe('B2');
  });

  it('Q19: a change closes the dialog and hides the toast; once the new branch answers the dialog opens for it', async () => {
    const { setBranch, setBranchLabel } = await mount('B1', 'สาขาสีลม');
    answer('B1', {
      count: 1,
      latest: 'K1',
      workingCount: 1,
      working: [{ id: 'K1', queue: 'Q-1', staff: 'nok', startedAt: null, status: 'ACCEPTED' }],
    });
    await flush();
    openQueue();
    await flush();

    setBranchLabel('สาขาสยาม');
    setBranch('B2');
    await flush();
    expect(shown(toast())).toBe(false);
    expect(shown(dialog())).toBe(false);

    answer('B2', { branchName: 'Siam', count: 0 });
    await flush();
    openQueue('B2');
    await flush();
    expect(dialogName()).toBe('คิวลูกค้า · สาขาสยาม');
    expect(livePanels().at(-1)).toMatchObject({ branch: 'B2', shown: true });
  });

  it('Q20: the toast key is per branch: a latest toasted on B1 still toasts on B2', async () => {
    const { setBranch } = await mount('B1');
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    setBranch('B2');
    await flush();
    answer('B2', { branchName: 'Siam', count: 1, latest: 'K1' });
    await flush();
    expect(shown(toast())).toBe(true);
    expect(sessionStorage.getItem('xc.queue.popup.emp-nok.B2')).toBe('K1');
  });
});

// ── the shell mounts it ──────────────────────────────────────────────────────

describe('the shell (design §12.12)', () => {
  it('Q21: AdminShell imports QueueBar from ./queue/QueueBar and renders it', () => {
    const src = shellSources['../AdminShell.tsx'];
    expect(src, 'web/src/screens/admin/AdminShell.tsx exists').toBeDefined();
    expect(src).toMatch(/import\s*\{\s*QueueBar\s*\}\s*from\s*['"]\.\/queue\/QueueBar['"]/);
    expect(src).toMatch(/<QueueBar\b[^>]*\bbranch=\{/);
    expect(src).toMatch(/<QueueBar\b[^>]*\bbranchLabel=\{/);
  });
});
