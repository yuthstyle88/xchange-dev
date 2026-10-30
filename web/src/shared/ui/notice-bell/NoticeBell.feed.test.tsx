// #5566 (DESK-R4 T1, docs/design/fe-desk-round4.md §3 T1): NoticeBell publishes every fresh notice to the notice feed.
// Written before the implementation by the ticket author claude/studio-claude-15. Do not edit, delete or weaken;
// add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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
import { onFreshNotice, publishFreshNotices, type FreshNotice } from './notice-feed';

// Wrap the real publisher so B2b can read the published list (the listeners still hear it).
vi.mock('./notice-feed', async (importOriginal) => {
  const real = await importOriginal<typeof import('./notice-feed')>();
  return { ...real, publishFreshNotices: vi.fn(real.publishFreshNotices) };
});

function fakeClient(script: Array<ViewAnswer<MyNotificationsData | undefined>>) {
  let i = 0;
  return {
    view: async <U,>(_name: string, _params: ViewParams = {}): Promise<ViewAnswer<U>> => {
      const next = script[Math.min(i, script.length - 1)]!;
      i += 1;
      return makeViewAnswer(next.data as unknown as U, { version: next.version, changed: next.changed });
    },
  };
}

function item(seq: number, objectType: string | null): MyNotificationItem {
  return makeMyNotificationItem({
    id: `n-${seq}`,
    seq,
    title: `title ${seq}`,
    body: `body ${seq}`,
    ...(objectType === null ? {} : { object: makeMyNotificationObject({ objectType, objectId: `obj-${seq}` }) }),
  });
}

const answer = (items: MyNotificationItem[], version: number) =>
  makeViewAnswer<MyNotificationsData | undefined>(makeMyNotificationsData({ items }), { version, changed: true });

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let stops: Array<() => void> = [];

/** Every notice the feed carried, of the given types. */
function listen(...types: string[]): FreshNotice[] {
  const heard: FreshNotice[] = [];
  for (const t of types) stops.push(onFreshNotice(t, (n) => heard.push(n)));
  return heard;
}

beforeEach(() => {
  vi.useFakeTimers({ now: 0 });
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  for (const s of stops) s();
  stops = [];
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.useRealTimers();
});

describe('NoticeBell publishes fresh notices (#5566)', () => {
  it('B1 the first answer publishes nothing', async () => {
    const heard = listen('cash_count', 'booking');
    const client = fakeClient([answer([item(1, 'cash_count'), item(2, 'booking')], 1)]);
    dispose = render(() => <NoticeBell client={client} branch="XC-B01" intervalMs={1000} />, host);
    await vi.advanceTimersByTimeAsync(0);
    expect(heard).toEqual([]);
  });

  it('B2 a later answer publishes every item above the highest seq, of any type, in seq order', async () => {
    const heard = listen('cash_count', 'booking');
    const client = fakeClient([
      answer([item(1, 'cash_count')], 1),
      answer([item(1, 'cash_count'), item(5, 'booking'), item(3, 'cash_count'), item(4, null)], 2),
    ]);
    dispose = render(() => <NoticeBell client={client} branch="XC-B01" intervalMs={1000} />, host);
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(1000);
    expect(heard).toEqual([
      { seq: 3, objectType: 'cash_count', objectId: 'obj-3' },
      { seq: 5, objectType: 'booking', objectId: 'obj-5' },
    ]);
  });

  it('B2b an item with no object is published with objectType null', async () => {
    const seen: FreshNotice[] = [];
    const spy = vi.mocked(publishFreshNotices);
    spy.mockClear();
    const client = fakeClient([answer([item(1, 'cash_count')], 1), answer([item(1, 'cash_count'), item(2, null)], 2)]);
    dispose = render(() => <NoticeBell client={client} branch="XC-B01" intervalMs={1000} />, host);
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(1000);
    for (const call of spy.mock.calls) seen.push(...call[0]);
    expect(seen).toEqual([{ seq: 2, objectType: null, objectId: null }]);
  });

  it('B3 the same items again publish nothing more', async () => {
    const heard = listen('cash_count');
    const later = answer([item(1, 'cash_count'), item(2, 'cash_count')], 2);
    const client = fakeClient([answer([item(1, 'cash_count')], 1), later, later, later]);
    dispose = render(() => <NoticeBell client={client} branch="XC-B01" intervalMs={1000} />, host);
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(1000);
    await vi.advanceTimersByTimeAsync(1000);
    await vi.advanceTimersByTimeAsync(1000);
    expect(heard.map((n) => n.seq)).toEqual([2]);
  });
});
