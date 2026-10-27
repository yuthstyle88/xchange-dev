// #5306: /staff waiting counts show worded kind, Bangkok time, short id, and totals (not counter's raw user id)
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th, en } from './i18n';
import { ShiftCard } from './ShiftCard';
import type { StaffCtx } from '../cards';
import {
  makeCashCountRow,
  makeCashCountTotalRow,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';

const BRANCH = 'B1';
const ME = '00000000-0000-4000-8000-0000000000a1';
const COUNTER = '37ba9f73-54a0-4b94-aac2-7b32f08ff33e';

function mockMe() {
  return makeStaffMeView({
    userId: ME,
    name: 'Manager Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B1',
    permissions: ['ShiftCloseCertify'],
  });
}

function mockFloor() {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  });
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('#5306 ShiftCard waiting counts row display (§8, §9)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    setLang('th');
    vi.unstubAllGlobals();
  });

  function renderCard(counts: ReturnType<typeof makeCashCountRow>[]) {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url =
          typeof input === 'string'
            ? input
            : input instanceof URL
              ? input.href
              : input.url;
        if (url.includes('/api/v3/views/cash_counts')) {
          return json(200, { version: 1, changed: true, data: counts });
        }
        return json(200, { version: 1, changed: true, data: [] });
      }),
    );
    const ctx: StaffCtx = { me: mockMe(), floor: mockFloor(), refresh: vi.fn(async () => {}) };
    dispose = render(() => <ShiftCard ctx={ctx} />, host);
  }

  it('Row 1: one OPEN count in Thai shows worded OPEN, Bangkok time, short id with full id in title, and totals', async () => {
    const countId = '37ba9f73-54a0-4b94-aac2-7b32f08ff33e';
    const count = makeCashCountRow({
      id: countId,
      branchId: BRANCH,
      kind: 'OPEN',
      countedBy: COUNTER,
      countedAt: '2026-10-27T20:16:00Z', // 28/10 03:16 Asia/Bangkok
      totals: [makeCashCountTotalRow({ currency: 'THB', counted: '5000' })],
    });

    renderCard([count]);
    await settle();

    const item = host.querySelector(`[data-count-id="${countId}"]`);
    expect(item, 'waiting count item rendered').not.toBeNull();

    const title = item!.querySelector('.xc-shift-item-title');
    expect(title?.textContent).toBe(th.OPEN ?? 'เปิดกะ');

    const sub = item!.querySelector('.xc-shift-item-sub');
    expect(sub?.textContent).toContain('28/10 03:16');
    expect(sub?.textContent).toContain('#37ba9f73');
    expect(sub?.textContent).toContain('THB 5000');
    expect(sub?.textContent).not.toContain(COUNTER);

    const shortIdEl = sub!.querySelector(`[title="${countId}"]`);
    expect(shortIdEl, 'element has full id in title').not.toBeNull();
    expect(shortIdEl?.textContent).toBe('#37ba9f73');
  });

  it('Row 2: two OPEN counts by the same user show different sub-lines', async () => {
    const idA = '606bdfcc-1111-4000-8000-000000000001';
    const idB = 'bb2d97ae-2222-4000-8000-000000000002';
    const countA = makeCashCountRow({
      id: idA,
      branchId: BRANCH,
      kind: 'OPEN',
      countedBy: COUNTER,
      countedAt: '2026-10-27T20:16:00Z', // 28/10 03:16
      totals: [makeCashCountTotalRow({ currency: 'THB', counted: '1000' })],
    });
    const countB = makeCashCountRow({
      id: idB,
      branchId: BRANCH,
      kind: 'OPEN',
      countedBy: COUNTER,
      countedAt: '2026-10-25T05:03:00Z', // 25/10 12:03
      totals: [makeCashCountTotalRow({ currency: 'THB', counted: '2000' })],
    });

    renderCard([countA, countB]);
    await settle();

    const itemA = host.querySelector(`[data-count-id="${idA}"]`);
    const itemB = host.querySelector(`[data-count-id="${idB}"]`);
    const subA = itemA?.querySelector('.xc-shift-item-sub')?.textContent;
    const subB = itemB?.querySelector('.xc-shift-item-sub')?.textContent;

    expect(subA).toContain('28/10 03:16');
    expect(subA).toContain('#606bdfcc');
    expect(subB).toContain('25/10 12:03');
    expect(subB).toContain('#bb2d97ae');
    expect(subA).not.toEqual(subB);
  });

  it('Row 3: kind CLOSE in English shows the English word for CLOSE', async () => {
    setLang('en');
    const countId = 'c-close-001';
    const count = makeCashCountRow({
      id: countId,
      branchId: BRANCH,
      kind: 'CLOSE',
      countedBy: COUNTER,
      countedAt: '2026-10-27T20:16:00Z',
      totals: [],
    });

    renderCard([count]);
    await settle();

    const item = host.querySelector(`[data-count-id="${countId}"]`);
    const title = item?.querySelector('.xc-shift-item-title');
    expect(title?.textContent).toBe(en.CLOSE ?? 'Close');
  });

  it('Row 4: kind WEIRD shows WEIRD as it is', async () => {
    const countId = 'c-weird-001';
    const count = makeCashCountRow({
      id: countId,
      branchId: BRANCH,
      kind: 'WEIRD',
      countedBy: COUNTER,
      countedAt: '2026-10-27T20:16:00Z',
      totals: [],
    });

    renderCard([count]);
    await settle();

    const item = host.querySelector(`[data-count-id="${countId}"]`);
    const title = item?.querySelector('.xc-shift-item-title');
    expect(title?.textContent).toBe('WEIRD');
  });

  it('Edge case: empty totals has no stray separator', async () => {
    const countId = 'c-empty-tot-001';
    const count = makeCashCountRow({
      id: countId,
      branchId: BRANCH,
      kind: 'OPEN',
      countedBy: COUNTER,
      countedAt: '2026-10-27T20:16:00Z',
      totals: [],
    });

    renderCard([count]);
    await settle();

    const item = host.querySelector(`[data-count-id="${countId}"]`);
    const sub = item?.querySelector('.xc-shift-item-sub');
    expect(sub?.textContent).toBe('28/10 03:16 · #c-empty-');
    expect(sub?.textContent?.endsWith('·')).toBe(false);
    expect(sub?.textContent?.endsWith('· ')).toBe(false);
  });

  it('Edge case: midnight countedAt 2026-10-27T17:00:00Z shows 28/10 00:00 (Bangkok)', async () => {
    const countId = 'c-midnight-001';
    const count = makeCashCountRow({
      id: countId,
      branchId: BRANCH,
      kind: 'OPEN',
      countedBy: COUNTER,
      countedAt: '2026-10-27T17:00:00Z',
      totals: [],
    });

    renderCard([count]);
    await settle();

    const item = host.querySelector(`[data-count-id="${countId}"]`);
    const sub = item?.querySelector('.xc-shift-item-sub');
    expect(sub?.textContent).toContain('28/10 00:00');
  });
});
