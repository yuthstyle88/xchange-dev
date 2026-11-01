// FE-T4b (fe-triage-1101 T4/T10): the manager's waiting-count row names the counter (`countedByName`), never the user id.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
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

describe('FE-T4b ShiftCard waiting-count row names the counter', () => {
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

  it('N1: a count with countedByName shows the name in [data-counted-by], with the counter id only in its title', async () => {
    const countId = '606bdfcc-faef-463a-b343-787b5389f142';
    renderCard([
      makeCashCountRow({
        id: countId,
        branchId: BRANCH,
        kind: 'OPEN',
        countedBy: COUNTER,
        countedByName: 'Somchai Teller',
        countedAt: '2026-10-27T20:16:00Z',
        totals: [makeCashCountTotalRow({ currency: 'THB', counted: '5000' })],
      }),
    ]);
    await settle();

    const item = host.querySelector(`[data-count-id="${countId}"]`);
    expect(item, 'waiting count item rendered').not.toBeNull();
    const name = item!.querySelector('[data-counted-by]');
    expect(name, 'counter name element').not.toBeNull();
    expect(name!.textContent).toBe('Somchai Teller');
    expect(name!.getAttribute('title')).toBe(COUNTER);
    const sub = item!.querySelector('.xc-shift-item-sub')?.textContent ?? '';
    expect(sub).toContain('#606bdfcc');
    expect(item!.textContent).not.toContain(COUNTER);
  });

  it('N2: a count with no countedByName renders no name element and never prints the counter id', async () => {
    const countId = 'bb2d97ae-2222-4000-8000-000000000002';
    renderCard([
      makeCashCountRow({
        id: countId,
        branchId: BRANCH,
        kind: 'CLOSE',
        countedBy: COUNTER,
        countedByName: null,
        countedAt: '2026-10-25T05:03:00Z',
        totals: [makeCashCountTotalRow({ currency: 'THB', counted: '2000' })],
      }),
    ]);
    await settle();

    const item = host.querySelector(`[data-count-id="${countId}"]`);
    expect(item, 'waiting count item rendered').not.toBeNull();
    expect(item!.querySelector('[data-counted-by]')).toBeNull();
    expect(item!.textContent).not.toContain(COUNTER);
    expect(item!.textContent).not.toContain(COUNTER.slice(0, 8));
    expect(item!.textContent).toContain('#bb2d97ae');
  });
});
