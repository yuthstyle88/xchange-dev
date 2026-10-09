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
// `error` then sets `state`; a failure sets `error` and keeps `state`). `QueuePanel` (#486),
// `dockQueueCard` and `QueueSoundControls` (#487) are replaced by recorders: the claim, the tabs and
// the pager are the panel's and are not tested here. `fetch` is watched and must never be called.
//
// "Shown" means connected, with no `hidden` on it or an ancestor, no inline `display: none`, and,
// for a `<dialog>`, its `open` set (jsdom has no `showModal()`; set the attribute, or call
// `showModal()` only where it exists). A part not rendered at all is not shown either.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { queueStore, type QueueError, type QueueStore } from '../../../shared/queue/queue-store';
import type { KioskNotifications } from '../../../shared/queue/wire';

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
  docks: 0,
  undocks: 0,
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
  dockQueueCard: () => {
    fake.docks += 1;
    let released = false;
    return () => {
      if (!released) {
        released = true;
        fake.undocks += 1;
      }
    };
  },
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

const EMPLOYEE = { id: 'emp-nok', role: 'OWNER' };

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

function bar(): HTMLElement {
  const el = document.querySelector<HTMLElement>('[aria-label="คิวลูกค้าและผู้ทำรายการ"]');
  expect(el, 'the bar is labelled คิวลูกค้าและผู้ทำรายการ').not.toBeNull();
  return el!;
}

function within(root: Element, text: string): HTMLElement | undefined {
  return [...root.querySelectorAll<HTMLElement>('*')].find(
    (n) => n.textContent?.trim() === text && [...n.children].every((c) => c.textContent?.trim() !== text),
  );
}

function button(root: ParentNode, name: string): HTMLButtonElement | undefined {
  return [...root.querySelectorAll<HTMLButtonElement>('button')].find(
    (b) => (b.getAttribute('aria-label') ?? b.textContent?.trim()) === name,
  );
}

function openButton(): HTMLButtonElement {
  const b = button(bar(), 'เปิดคิว / รับเรื่อง →');
  expect(b, 'the bar has the button เปิดคิว / รับเรื่อง →').toBeDefined();
  return b!;
}

function statusText(): string {
  const s = bar().querySelector('[role="status"]');
  expect(s, "the bar's status line has role=status").not.toBeNull();
  return s!.textContent?.trim() ?? '';
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
  return [...document.querySelectorAll<HTMLElement>('*')].find(
    (n) => n !== bar() && !bar().contains(n) && /^มีคิวลูกค้ารอ \d+ รายการ · /.test(n.textContent?.trim() ?? '') &&
      button(n, 'ดูคิว') !== undefined,
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
  fake.docks = 0;
  fake.undocks = 0;
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
  it('Q1: before any answer it says กำลังตรวจคิว… under the title คิวลูกค้าหน้าร้าน', async () => {
    await mount();
    expect(within(bar(), 'คิวลูกค้าหน้าร้าน'), 'title').toBeDefined();
    expect(statusText()).toBe('กำลังตรวจคิว…');
    expect(shown(openButton())).toBe(true);
  });

  it('Q2: an answer prints the counts, the branch name and every worker line', async () => {
    await mount();
    const started = '2026-10-09T02:15:00.000Z';
    answer('B1', {
      count: 3,
      workingCount: 2,
      latest: 'K9',
      working: [
        { id: 'K1', queue: 'Q-012', staff: 'nok', startedAt: started, status: 'ACCEPTED' },
        { id: 'K2', queue: 'Q-013', staff: 'som', startedAt: null, status: 'ID_CHECKED' },
      ],
    });
    await flush();
    expect(statusText()).toBe('รอรับเรื่อง 3 · กำลังทำรายการ 2 · Silom');
    expect(within(bar(), 'คิวลูกค้า · Silom'), 'title follows the answer').toBeDefined();
    const stamp = new Date(started).toLocaleString('th-TH', { timeZone: 'Asia/Bangkok' });
    const one = within(bar(), `nok กำลังทำรายการ Q-012 · เริ่ม ${stamp}`);
    const two = within(bar(), 'som กำลังทำรายการ Q-013 · เริ่ม ยังไม่มีข้อมูล');
    expect(shown(one), 'first worker line').toBe(true);
    expect(shown(two), 'a worker with no start time reads ยังไม่มีข้อมูล').toBe(true);
  });

  it('Q3: with nobody working, no worker line is shown', async () => {
    await mount();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    expect([...bar().querySelectorAll('*')].some((n) => shown(n) && /กำลังทำรายการ \S+ · เริ่ม/.test(n.textContent ?? ''))).toBe(false);
  });

  it('Q4: the bar docks the queue card while mounted and undocks on unmount', async () => {
    const { dispose } = await mount();
    expect(fake.docks).toBe(1);
    expect(fake.undocks).toBe(0);
    dispose();
    expect(fake.undocks).toBe(1);
  });

  it('Q5: the sound controls render inside the bar', async () => {
    await mount();
    const controls = bar().querySelector('[data-fake="sound-controls"]');
    expect(shown(controls)).toBe(true);
  });

  it('Q6: the bar never calls fetch itself', async () => {
    await mount();
    answer('B1', { count: 2, latest: 'K1' });
    await flush();
    openButton().click();
    await flush();
    expect(fetchSpy).not.toHaveBeenCalled();
  });
});

// ── the dialog ───────────────────────────────────────────────────────────────

describe('the dialog (admin-queue.js:7-9)', () => {
  it('Q7: the button opens the dialog named by the picked branch and mounts the panel for it', async () => {
    await mount('B1', 'สาขาสีลม');
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    expect(shown(dialog())).toBe(false);
    openButton().click();
    await flush();
    expect(shown(dialog())).toBe(true);
    expect(dialogName()).toBe('คิวลูกค้า · สาขาสีลม');
    const live = livePanels();
    expect(live).toHaveLength(1);
    expect(live[0]).toMatchObject({ branch: 'B1', shown: true });
    expect(shown(dialog()!.querySelector('[data-fake="queue-panel"]'))).toBe(true);
  });

  it('Q8: ปิด × closes the dialog, unmounts the panel and returns focus to the button', async () => {
    await mount();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    openButton().click();
    await flush();
    const close = button(dialog()!, 'ปิด ×');
    expect(close, 'the dialog has ปิด ×').toBeDefined();
    close!.click();
    await flush();
    expect(shown(dialog())).toBe(false);
    expect(livePanels().filter((p) => p.shown)).toHaveLength(0);
    expect(document.activeElement).toBe(openButton());
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
    openButton().click();
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
  it('Q16: offline prints v2\'s sentence, hides workers and toast, disables the button; an answer re-enables it', async () => {
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
    expect(statusText()).toBe('เชื่อมต่อคิวไม่ได้ · กำลังลองใหม่');
    expect(shown(within(bar(), 'nok กำลังทำรายการ Q-1 · เริ่ม ยังไม่มีข้อมูล'))).toBe(false);
    expect(shown(toast())).toBe(false);
    expect(openButton().disabled).toBe(true);

    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    expect(openButton().disabled).toBe(false);
  });

  it('Q17: a session end closes the dialog and keeps the button disabled after later answers', async () => {
    await mount();
    answer('B1', { count: 1, latest: 'K1' });
    await flush();
    openButton().click();
    await flush();
    expect(shown(dialog())).toBe(true);
    store('B1').setError({ kind: 'ended', reason: 'expired' });
    await flush();
    expect(shown(dialog())).toBe(false);
    expect(openButton().disabled).toBe(true);
    answer('B1', { count: 2, latest: 'K2' });
    await flush();
    expect(openButton().disabled, 'locked stays locked (v2 `locked=true`)').toBe(true);
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

  it('Q19: a change resets the bar until the new branch answers, and closes the dialog', async () => {
    const { setBranch, setBranchLabel } = await mount('B1', 'สาขาสีลม');
    answer('B1', {
      count: 1,
      latest: 'K1',
      workingCount: 1,
      working: [{ id: 'K1', queue: 'Q-1', staff: 'nok', startedAt: null, status: 'ACCEPTED' }],
    });
    await flush();
    openButton().click();
    await flush();

    setBranchLabel('สาขาสยาม');
    setBranch('B2');
    await flush();
    expect(statusText()).toBe('กำลังตรวจคิวสาขา…');
    expect(shown(within(bar(), 'nok กำลังทำรายการ Q-1 · เริ่ม ยังไม่มีข้อมูล'))).toBe(false);
    expect(shown(toast())).toBe(false);
    expect(openButton().disabled).toBe(true);
    expect(shown(dialog())).toBe(false);

    answer('B2', { branchName: 'Siam', count: 0 });
    await flush();
    expect(statusText()).toBe('รอรับเรื่อง 0 · กำลังทำรายการ 0 · Siam');
    expect(openButton().disabled).toBe(false);
    openButton().click();
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
