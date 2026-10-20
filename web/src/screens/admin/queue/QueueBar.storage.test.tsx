import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { queueStore, type QueueError, type QueueStore } from '../../../shared/queue/queue-store';
import {
  makeQueueBoardData,
  makeQueueBoardWaiting,
  type QueueBoardData,
} from '../../../shared/testing/fixtures';
import { meOf } from '../../../test-support/me';
import { QueueBar } from './QueueBar';

interface FakeStore extends Omit<QueueStore, 'state'> {
  state: Accessor<QueueBoardData | undefined>;
  setState(v: QueueBoardData | undefined): void;
  setError(v: QueueError | undefined): void;
}

const fake = vi.hoisted(() => ({
  stores: new Map<string, unknown>(),
  queueBranchCalls: [] as string[],
  panels: [] as { branch: string; shown: boolean; alive: boolean }[],
}));

const shared = vi.hoisted(() => ({ setMe: undefined as undefined | ((me: unknown) => void) }));

vi.mock('../../../shared/session/session', async (importOriginal) => {
  const original = await importOriginal<typeof import('../../../shared/session/session')>();
  const { createSignal: signal } = await import('solid-js');
  const [me, setMe] = signal<unknown>(undefined);
  shared.setMe = (value) => setMe(() => value);
  const session = { me, signOut: async () => {}, dispose: () => {} };
  return { ...original, sharedSession: () => session };
});

vi.mock('../../../shared/queue/queue-store', async () => {
  const solid = await import('solid-js');
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

function store(b: string): FakeStore {
  return queueStore(b) as FakeStore;
}

function waitingTicket(id: string, seq = 1) {
  return makeQueueBoardWaiting({
    id,
    displayNumber: `W00${seq}`,
    serviceType: 'WALK_IN',
    sequence: seq,
    issuedAt: '2026-10-09T03:00:00.000Z',
  });
}

function answer(b: string, over: Partial<QueueBoardData> = {}) {
  const s = store(b);
  s.setError(undefined);
  const data = makeQueueBoardData({
    waiting: [],
    called: [],
    myCounter: null,
    ...over,
  });
  s.setState(data);
}

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

function toast(): HTMLElement | undefined {
  const isToast = (n: Element) =>
    /^มีคิวลูกค้ารอ \d+ รายการ · /.test(n.textContent?.trim() ?? '') &&
    button(n, 'ดูคิว') !== undefined;
  return [...document.querySelectorAll<HTMLElement>('*')].find(
    (n) => isToast(n) && [...n.children].every((c) => !isToast(c)),
  );
}

const flush = () => new Promise<void>((r) => setTimeout(r, 0));

let disposers: (() => void)[] = [];

const ME = meOf({
  id: 'emp-nok',
  role: 'MANAGER',
  branch: 'B1',
  name: 'Nok',
  branchName: 'Silom',
});

async function mount(start = 'B1', label = 'Silom') {
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
  sessionStorage.clear();
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => new Response('{}', { status: 500 })),
  );
  shared.setMe?.(ME);
});

afterEach(() => {
  for (const d of disposers.splice(0)) d();
  document.body.innerHTML = '';
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('QueueBar blocked sessionStorage handling (#2372)', () => {
  it('row 1: getItem throws, new latest K1 -> Toast shown, no error escapes', async () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('The operation is insecure.', 'SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('The operation is insecure.', 'SecurityError');
    });

    await mount();
    expect(() => {
      answer('B1', { waiting: [waitingTicket('K1', 1)] });
    }).not.toThrow();
    await flush();

    const t1 = toast();
    expect(shown(t1)).toBe(true);
    expect(t1!.textContent).toContain('มีคิวลูกค้ารอ 1 รายการ · Silom');
  });

  it('row 2: getItem and setItem throw, the same K1 polled again -> No second toast (memory fallback)', async () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('The operation is insecure.', 'SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('The operation is insecure.', 'SecurityError');
    });

    await mount();
    answer('B1', { waiting: [waitingTicket('K1', 1)] });
    await flush();
    const t1 = toast();
    expect(shown(t1)).toBe(true);

    button(t1!, 'ปิด')!.click();
    await flush();
    expect(shown(toast())).toBe(false);

    // Same K1 polled again while storage throws -> no second toast
    answer('B1', { waiting: [waitingTicket('K1', 1)] });
    await flush();
    expect(shown(toast())).toBe(false);
  });

  it('row 3: Storage throws, then K2 arrives -> Toast shown again for K2', async () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('The operation is insecure.', 'SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('The operation is insecure.', 'SecurityError');
    });

    await mount();
    answer('B1', { waiting: [waitingTicket('K1', 1)] });
    await flush();
    const t1 = toast();
    expect(shown(t1)).toBe(true);

    button(t1!, 'ปิด')!.click();
    await flush();
    expect(shown(toast())).toBe(false);

    // New latest K2 arrives while storage throws -> toast shown again for K2
    answer('B1', { waiting: [waitingTicket('K2', 2), waitingTicket('K1', 1)] });
    await flush();
    const t2 = toast();
    expect(shown(t2)).toBe(true);
    expect(t2!.textContent).toContain('มีคิวลูกค้ารอ 2 รายการ · Silom');
  });

  it('row 4: Q11 toast once, key stored in sessionStorage (and falls back cleanly if setItem throws)', async () => {
    await mount();
    answer('B1', { waiting: [waitingTicket('K1', 1)] });
    await flush();
    const t1 = toast();
    expect(shown(t1)).toBe(true);
    expect(sessionStorage.getItem('xc.queue.popup.emp-nok.B1')).toBe('K1');

    button(t1!, 'ปิด')!.click();
    await flush();
    expect(shown(toast())).toBe(false);

    // If setItem throws on the next ticket K2, toast still appears once and remembers K2 in memory
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('QuotaExceededError', 'QuotaExceededError');
    });

    answer('B1', { waiting: [waitingTicket('K2', 2), waitingTicket('K1', 1)] });
    await flush();
    const t2 = toast();
    expect(shown(t2)).toBe(true);
    expect(t2!.textContent).toContain('มีคิวลูกค้ารอ 2 รายการ · Silom');

    button(t2!, 'ปิด')!.click();
    await flush();
    answer('B1', { waiting: [waitingTicket('K2', 2), waitingTicket('K1', 1)] });
    await flush();
    expect(shown(toast())).toBe(false);
  });

  it('row 5: Q12 already toasted in this tab does not toast after reload, plus §9 edge cases when storage throws', async () => {
    sessionStorage.setItem('xc.queue.popup.emp-nok.B1', 'K1');
    const { setBranch, setBranchLabel } = await mount('B1', 'Silom');
    answer('B1', { waiting: [waitingTicket('K1', 1)] });
    await flush();
    expect(shown(toast())).toBe(false);

    // Block storage for §9 edge cases
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('The operation is insecure.', 'SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('The operation is insecure.', 'SecurityError');
    });

    // Edge case 1: storageKey null without me -> no toast
    shared.setMe?.(undefined);
    answer('B1', { waiting: [waitingTicket('K2', 2)] });
    await flush();
    expect(shown(toast())).toBe(false);

    // Restore me -> K2 toasts on B1
    shared.setMe?.(ME);
    answer('B1', { waiting: [waitingTicket('K2', 2)] });
    await flush();
    expect(shown(toast())).toBe(true);

    // Edge case 2: count 0 hides the toast
    answer('B1', { waiting: [] });
    await flush();
    expect(shown(toast())).toBe(false);

    // Edge case 3: branch switch uses a different key -> K2 on B2 still toasts
    setBranch('B2');
    setBranchLabel('Asok');
    await flush();
    answer('B2', { waiting: [waitingTicket('K2', 1)] });
    await flush();
    const tB2 = toast();
    expect(shown(tB2)).toBe(true);
    expect(tB2!.textContent).toContain('มีคิวลูกค้ารอ 1 รายการ · Asok');
  });
});
