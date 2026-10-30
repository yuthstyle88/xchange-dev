// docs/design/desk-shift-open.md §2: the WAITING-counts reader /staff and /desk share.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import { makeCashCountRow } from '../../../shared/testing/fixtures';
import { publishFreshNotices } from '../../../shared/ui/notice-bell/notice-feed';
import { createWaitingCounts, type WaitingCounts } from './waiting-counts';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let reads: string[] = [];
let answers: Array<() => Response> = [];

function stub() {
  reads = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      reads.push(url);
      const next = answers.shift();
      return next ? next() : json(200, { version: 1, changed: true, data: [] });
    }),
  );
}

function mount(enabled = true): { w: () => WaitingCounts; setEnabled: (v: boolean) => void } {
  let w!: WaitingCounts;
  const [on, setOn] = createSignal(enabled);
  dispose = render(() => {
    w = createWaitingCounts({ api: new ApiClient(), branch: () => 'MAIN', enabled: on });
    return <span />;
  }, host);
  return { w: () => w, setEnabled: setOn };
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

const cashReads = () => reads.filter((u) => u.includes('/api/v3/views/cash_counts'));

describe('createWaitingCounts', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    answers = [];
    stub();
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
  });

  it('W1 reads the branch WAITING counts once on mount; loaded only after the answer', async () => {
    answers.push(() => json(200, { version: 1, changed: true, data: [makeCashCountRow({ id: 'c1' })] }));
    const { w } = mount();
    expect(w().loaded()).toBe(false);
    await settle();
    expect(cashReads()).toHaveLength(1);
    const u = new URL(cashReads()[0]!, 'http://x');
    expect(u.searchParams.get('branch')).toBe('MAIN');
    expect(u.search).toContain('WAITING');
    expect(w().loaded()).toBe(true);
    expect(w().rows().map((r) => r.id)).toEqual(['c1']);
  });

  it('W2 a {rows} answer is read too', async () => {
    answers.push(() => json(200, { version: 1, changed: true, data: { rows: [makeCashCountRow({ id: 'r1' })] } }));
    const { w } = mount();
    await settle();
    expect(w().rows().map((r) => r.id)).toEqual(['r1']);
  });

  it('W3 a fresh cash_count notice reads again; another type does not', async () => {
    mount();
    await settle();
    publishFreshNotices([{ seq: 1, objectType: 'booking', objectId: null }]);
    await settle();
    expect(cashReads()).toHaveLength(1);
    publishFreshNotices([{ seq: 2, objectType: 'cash_count', objectId: 'c1' }]);
    await settle();
    expect(cashReads()).toHaveLength(2);
  });

  it('W4 a failed read: loaded, worded error, the old rows kept; the next good read clears the error', async () => {
    answers.push(() => json(200, { version: 1, changed: true, data: [makeCashCountRow({ id: 'keep' })] }));
    answers.push(() => json(500, { error: { code: 'internal', message: 'boom' } }));
    const { w } = mount();
    await settle();
    await w().reload();
    await settle();
    expect(w().loaded()).toBe(true);
    expect(w().error()).not.toBeNull();
    expect(w().rows().map((r) => r.id)).toEqual(['keep']);
    await w().reload();
    await settle();
    expect(w().error()).toBeNull();
  });

  it('W5 disabled: no read, not loaded', async () => {
    const { w } = mount(false);
    await settle();
    await w().reload();
    expect(cashReads()).toHaveLength(0);
    expect(w().loaded()).toBe(false);
  });

  it('W6 after unmount a notice reads nothing', async () => {
    mount();
    await settle();
    dispose?.();
    dispose = undefined;
    publishFreshNotices([{ seq: 3, objectType: 'cash_count', objectId: null }]);
    await settle();
    expect(cashReads()).toHaveLength(1);
  });
});
