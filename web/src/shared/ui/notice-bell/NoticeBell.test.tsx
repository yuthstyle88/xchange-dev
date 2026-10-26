// #5099: the live unread-notice badge and toast, behaviour table rows 1–7.
// Written before the implementation by the test-writer claude/studio-claude-4. Do not edit, delete or weaken;
// add tests in a file of your own.
//
// The API this file pins ([proposed] in the ticket, fixed here so the worker follows it):
//
//   web/src/shared/ui/notice-bell/NoticeBell.tsx
//     export function NoticeBell(props: {
//       client: Pick<ApiClient, 'view'>; // the api client; AccountBar passes the shared one
//       branch: string | null;           // the signed-in user's branch; null = signed out (no polling)
//       intervalMs?: number;             // default 10_000 (ticket §3)
//     }): JSX.Element;
//
// It polls with the existing `ViewPoller` (`../../live/view-poller`) on view `my_notifications` with
// `{ branch, filter: { unread: true } }` (client.ts:99-109 turns `filter` into `unread=true`) and `since`.
//
// DOM it must render:
//   - `<a data-testid="notice-bell" href="/#notices">`: the link to the Notices page, always rendered while signed in.
//   - `[data-testid="notice-bell-count"]` inside it: the unread count as its text. Hidden at 0 = not rendered,
//     or `hidden` on it / an ancestor, or computed `display: none`.
//   - `[data-testid="notice-toast"]`: one element per toast, holding the notice's `title` and `body`; gone within 30 s.
//     Rendered inside the component (no global toast sink). A CSS import is not loaded by vitest.
//
// Toast rule (ticket §3): only items whose `seq` is greater than the highest `seq` seen so far AND whose
// `object?.objectType === 'cash_count'`. The first answer only sets the highest seq; it never toasts.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import type { ViewParams } from '../../api/client';
import {
  makeMyNotificationItem,
  makeMyNotificationObject,
  makeMyNotificationsData,
  makeViewAnswer,
  type MyNotificationItem,
  type MyNotificationsData,
  type ViewAnswer,
} from '../../testing/fixtures';
import { NoticeBell } from './NoticeBell';

interface Seen {
  at: number;
  name: string;
  params: ViewParams;
}

/** A fake `ApiClient` whose `view` answers from a script, one entry per call; the last entry repeats. */
function fakeClient(script: Array<ViewAnswer<MyNotificationsData | undefined> | Error>) {
  const seen: Seen[] = [];
  let i = 0;
  const client = {
    view: async <U,>(name: string, params: ViewParams = {}): Promise<ViewAnswer<U>> => {
      seen.push({ at: Date.now(), name, params: { ...params } });
      const next = script[Math.min(i, script.length - 1)]!;
      i += 1;
      if (next instanceof Error) throw next;
      return makeViewAnswer(next.data as unknown as U, { version: next.version, changed: next.changed });
    },
  };
  return { client, seen };
}

function item(seq: number, objectType: string | null, title = `title ${seq}`, body = `body ${seq}`) {
  return makeMyNotificationItem({
    id: `n-${seq}`,
    seq,
    title,
    body,
    ...(objectType === null ? {} : { object: makeMyNotificationObject({ objectType, objectId: `obj-${seq}` }) }),
  });
}

function answer(items: MyNotificationItem[], version: number) {
  return makeViewAnswer<MyNotificationsData | undefined>(makeMyNotificationsData({ items }), { version, changed: true });
}

function unchanged(version: number) {
  return makeViewAnswer<MyNotificationsData | undefined>(undefined, { version, changed: false });
}

function isShown(el: Element | null): boolean {
  for (let e: Element | null = el; e; e = e.parentElement) {
    if (!e.isConnected) return false;
    if (e.hasAttribute('hidden')) return false;
    if (getComputedStyle(e).display === 'none') return false;
  }
  return el !== null;
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function bell(): HTMLAnchorElement | null {
  return host.querySelector<HTMLAnchorElement>('[data-testid="notice-bell"]');
}

/** The badge text when shown, or null when hidden / absent. */
function shownCount(): string | null {
  const el = host.querySelector('[data-testid="notice-bell-count"]');
  return isShown(el) ? (el!.textContent ?? '').trim() : null;
}

function toasts(): Element[] {
  return Array.from(host.querySelectorAll('[data-testid="notice-toast"]')).filter(isShown);
}

function mount(client: ReturnType<typeof fakeClient>['client'], branch: () => string | null, intervalMs?: number) {
  dispose = render(
    () => <NoticeBell client={client} branch={branch()} {...(intervalMs === undefined ? {} : { intervalMs })} />,
    host,
  );
}

let hidden = false;

beforeEach(() => {
  vi.useFakeTimers({ now: 0 });
  hidden = false;
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => (hidden ? 'hidden' : 'visible'),
  });
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.useRealTimers();
});

describe('NoticeBell (#5099)', () => {
  it('polls my_notifications with unread=true, the branch and since, every 10 s by default', async () => {
    const { client, seen } = fakeClient([answer([item(1, 'cash_count')], 5), unchanged(5)]);
    mount(client, () => 'XC-B01');
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(1);
    expect(seen[0]!.name).toBe('my_notifications');
    expect(seen[0]!.params.branch).toBe('XC-B01');
    expect((seen[0]!.params.filter as { unread?: boolean } | undefined)?.unread).toBe(true);
    expect(seen[0]!.params.since).toBeUndefined();

    await vi.advanceTimersByTimeAsync(9_999);
    expect(seen).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(seen).toHaveLength(2);
    expect(seen[1]!.params.since).toBe(5);
    expect((seen[1]!.params.filter as { unread?: boolean } | undefined)?.unread).toBe(true);
  });

  it('row 1 (#5125): bell rendered on any page has href="/#notices"', async () => {
    const { client } = fakeClient([]);
    mount(client, () => 'XC-B01', 1000);
    expect(bell(), 'the bell link').not.toBeNull();
    expect(bell()!.getAttribute('href')).toBe('/#notices');
  });

  it('row 1: first poll with 3 unread shows badge 3 linking to /#notices, and no toast (even for cash_count items)', async () => {
    const { client } = fakeClient([answer([item(1, 'cash_count'), item(2, 'cash_count'), item(3, 'other')], 1)]);
    mount(client, () => 'XC-B01', 1000);
    await vi.advanceTimersByTimeAsync(0);
    expect(bell(), 'the bell link').not.toBeNull();
    expect(bell()!.getAttribute('href')).toBe('/#notices');
    expect(shownCount()).toBe('3');
    expect(toasts()).toHaveLength(0);
  });

  it('row 2: a changed: false poll keeps the badge (same node, same text) and shows no toast', async () => {
    const { client, seen } = fakeClient([
      answer([item(1, 'cash_count'), item(2, 'other'), item(3, 'other')], 1),
      unchanged(1),
    ]);
    mount(client, () => 'XC-B01', 1000);
    await vi.advanceTimersByTimeAsync(0);
    const before = host.querySelector('[data-testid="notice-bell-count"]');
    expect(shownCount()).toBe('3');

    await vi.advanceTimersByTimeAsync(3000);
    expect(seen.length).toBeGreaterThanOrEqual(3);
    expect(host.querySelector('[data-testid="notice-bell-count"]'), 'badge node not re-rendered').toBe(before);
    expect(shownCount()).toBe('3');
    expect(toasts()).toHaveLength(0);
  });

  it('row 3: a new cash_count item adds 1 to the badge and shows one toast with its title and body, gone within 30 s', async () => {
    const first = [item(1, 'other'), item(2, 'cash_count'), item(3, 'other')];
    const { client } = fakeClient([
      answer(first, 1),
      // the server re-sends the whole unread list; the new item is seq 4. Repeats afterwards (version moves).
      answer([...first, item(4, 'cash_count', 'Close count waiting', 'Shift SH-1 close count needs certifying')], 2),
    ]);
    mount(client, () => 'XC-B01', 1000);
    await vi.advanceTimersByTimeAsync(0);
    expect(shownCount()).toBe('3');
    expect(toasts()).toHaveLength(0);

    await vi.advanceTimersByTimeAsync(1000);
    expect(shownCount()).toBe('4');
    const shown = toasts();
    expect(shown).toHaveLength(1);
    expect(shown[0]!.textContent).toContain('Close count waiting');
    expect(shown[0]!.textContent).toContain('Shift SH-1 close count needs certifying');
    expect(shown[0]!.textContent, 'only the new item toasts').not.toContain('title 2');

    // later answers repeat the same items: seq 4 is no longer new, so no second toast; the first one goes away.
    await vi.advanceTimersByTimeAsync(30_000);
    expect(shownCount()).toBe('4');
    expect(toasts()).toHaveLength(0);
  });

  it('row 4: a new item of another objectType (or none) adds 1 to the badge and shows no toast', async () => {
    const first = [item(1, 'cash_count'), item(2, 'cash_count'), item(3, 'cash_count')];
    const { client } = fakeClient([
      answer(first, 1),
      answer([...first, item(4, 'transaction')], 2),
      answer([...first, item(4, 'transaction'), item(5, null)], 3),
    ]);
    mount(client, () => 'XC-B01', 1000);
    await vi.advanceTimersByTimeAsync(0);
    expect(shownCount()).toBe('3');

    await vi.advanceTimersByTimeAsync(1000);
    expect(shownCount()).toBe('4');
    expect(toasts()).toHaveLength(0);

    await vi.advanceTimersByTimeAsync(1000);
    expect(shownCount()).toBe('5');
    expect(toasts()).toHaveLength(0);
  });

  it('row 5: when everything is read (count 0) the badge is hidden', async () => {
    const { client } = fakeClient([answer([item(1, 'other'), item(2, 'other')], 1), answer([], 2)]);
    mount(client, () => 'XC-B01', 1000);
    await vi.advanceTimersByTimeAsync(0);
    expect(shownCount()).toBe('2');

    await vi.advanceTimersByTimeAsync(1000);
    expect(shownCount(), 'badge hidden at 0').toBeNull();
    expect(bell(), 'the link stays').not.toBeNull();
    expect(toasts()).toHaveLength(0);
  });

  it('row 6: a failed poll keeps the last badge and shows no toast; a later new cash_count still toasts once', async () => {
    const first = [item(1, 'other'), item(2, 'other'), item(3, 'other')];
    const { client, seen } = fakeClient([
      answer(first, 1),
      new Error('down'),
      answer([...first, item(4, 'cash_count', 'after the failure')], 2),
    ]);
    mount(client, () => 'XC-B01', 1000);
    await vi.advanceTimersByTimeAsync(0);
    expect(shownCount()).toBe('3');

    await vi.advanceTimersByTimeAsync(1000);
    expect(seen).toHaveLength(2);
    expect(shownCount(), 'badge keeps its last value').toBe('3');
    expect(toasts()).toHaveLength(0);

    // ViewPoller backs off to 2 s after an error (view-poller.test.ts); the next good answer still toasts.
    await vi.advanceTimersByTimeAsync(2000);
    expect(seen).toHaveLength(3);
    expect(shownCount()).toBe('4');
    expect(toasts()).toHaveLength(1);
    expect(toasts()[0]!.textContent).toContain('after the failure');
  });

  it('row 7: signed out (branch null) makes no request; signing out mid-run stops polling', async () => {
    const out = fakeClient([answer([item(1, 'cash_count')], 1)]);
    mount(out.client, () => null, 1000);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(out.seen).toHaveLength(0);
    expect(shownCount()).toBeNull();
    dispose?.();
    dispose = undefined;

    const [branch, setBranch] = createSignal<string | null>('XC-B01');
    const live = fakeClient([answer([item(1, 'other')], 1), unchanged(1)]);
    mount(live.client, branch, 1000);
    await vi.advanceTimersByTimeAsync(0);
    expect(live.seen).toHaveLength(1);
    expect(shownCount()).toBe('1');

    setBranch(null);
    await vi.advanceTimersByTimeAsync(0);
    const after = live.seen.length;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(live.seen, 'no request after sign-out').toHaveLength(after);
    expect(shownCount(), 'no badge after sign-out').toBeNull();
  });

  it('stops polling when unmounted', async () => {
    const { client, seen } = fakeClient([answer([item(1, 'other')], 1), unchanged(1)]);
    mount(client, () => 'XC-B01', 1000);
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toHaveLength(1);
    dispose?.();
    dispose = undefined;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(seen).toHaveLength(1);
  });
});
