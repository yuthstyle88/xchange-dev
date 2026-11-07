// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm (CE, Bangkok) from the shared formatter.
// #5399: Open count waiting for certification notice and open count button visibility (§8)
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th, en } from './i18n';
import { ShiftCard } from './ShiftCard';
import type { StaffCtx } from '../cards';
import {
  makeCashCountRow,
  makeDenominationRow,
  makeStaffFloorView,
  makeStaffMeView,
  type StaffFloorView,
  type StaffMeView,
} from '../../../shared/testing/fixtures';

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000a1';
const OTHER = '00000000-0000-4000-8000-0000000000a2';
const CLOCKED_IN = '2026-10-11T01:00:00Z';
const MY_SHIFT_ID = '11111111-1111-4111-8111-111111111111';

function me(permissions = ['ShiftOperate', 'ShiftCloseCertify']) {
  return makeStaffMeView({
    userId: ME,
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  });
}

function floor(over: Partial<StaffFloorView> = {}) {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: CLOCKED_IN,
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
    ...over,
  });
}

const DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
];

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let dynamicCounts: ReturnType<typeof makeCashCountRow>[];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stub(counts: ReturnType<typeof makeCashCountRow>[] = []) {
  dynamicCounts = [...counts];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const path = url.split('?')[0]!;
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET') {
        const viewName = path.startsWith('/api/v3/views/')
          ? path.slice('/api/v3/views/'.length)
          : '';
        if (viewName === 'cash_counts') {
          return json(200, { version: 1, changed: true, data: dynamicCounts });
        }
        if (viewName === 'denominations') {
          return json(200, { version: 1, changed: true, data: { rows: DENOMS } });
        }
        if (viewName === 'handovers_pending') {
          return json(200, { version: 1, changed: true, data: [] });
        }
        if (viewName === 'handover_receivers') {
          return json(200, { version: 1, changed: true, data: { rows: [] } });
        }
        if (viewName === 'my_shift') {
          return json(200, { version: 1, changed: true, data: null });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }

      return json(200, {});
    }),
  );
}

function mount(m: StaffMeView = me(), f: StaffFloorView = floor()) {
  const refresh = vi.fn(async () => {});
  const ctx: StaffCtx = { me: m, floor: f, refresh };
  dispose = render(() => <ShiftCard ctx={ctx} />, host);
  return { refresh };
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('#5399 ShiftCard open waiting notice and button visibility (§8)', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
  });

  it('1. clocked in, no shift, no counts: shows shift-open-count button; no shift-open-waiting', async () => {
    stub([]);
    mount();
    await settle();

    const openBtn = host.querySelector('[data-testid="shift-open-count"]');
    expect(openBtn, 'open count button is offered').not.toBeNull();
    expect(openBtn?.textContent).toContain(th.openCount);

    const waitingNotice = host.querySelector('[data-testid="shift-open-waiting"]');
    expect(waitingNotice, 'no waiting notice').toBeNull();
  });

  it('2. clocked in, no shift, my OPEN count WAITING: shows shift-open-waiting with role="status" and Thai text; no shift-open-count', async () => {
    const waitingCount = makeCashCountRow({
      id: 'c-wait-open-1',
      branchId: BRANCH,
      shiftId: null,
      kind: 'OPEN',
      countedBy: ME,
      countedAt: '2026-10-11T04:30:00Z', // 04:30 UTC = 11:30 Bangkok
      status: 'WAITING',
      verifiedBy: null,
      verifiedAt: null,
    });
    stub([waitingCount]);
    mount();
    await settle();

    const openBtn = host.querySelector('[data-testid="shift-open-count"]');
    expect(openBtn, 'open count button is hidden').toBeNull();

    const waitingNotice = host.querySelector('[data-testid="shift-open-waiting"]');
    expect(waitingNotice, 'waiting notice is shown').not.toBeNull();
    expect(waitingNotice?.getAttribute('role')).toBe('status');
    expect(waitingNotice?.className).toBe('xc-shift-close-waiting');
    expect(waitingNotice?.textContent).toContain('11/10/2026 11:30');
    expect(waitingNotice?.textContent).toContain(th.openWaiting ?? 'ยอดนับเปิดกะรอผู้จัดการตรวจรับรอง');

    // Language rule: the new text is Thai only, copied unchanged into en
    setLang('en');
    await settle();
    expect(waitingNotice?.textContent).toContain('11/10/2026 11:30');
    expect(waitingNotice?.textContent).toContain(en.openWaiting ?? 'ยอดนับเปิดกะรอผู้จัดการตรวจรับรอง');
  });

  it('3. the same count status REJECTED: shift-open-count again; no waiting notice', async () => {
    const rejectedCount = makeCashCountRow({
      id: 'c-wait-open-1',
      branchId: BRANCH,
      shiftId: null,
      kind: 'OPEN',
      countedBy: ME,
      countedAt: '2026-10-11T04:30:00Z',
      status: 'REJECTED',
      verifiedBy: null,
      verifiedAt: null,
    });
    stub([rejectedCount]);
    mount();
    await settle();

    const openBtn = host.querySelector('[data-testid="shift-open-count"]');
    expect(openBtn, 'open count button is offered again').not.toBeNull();

    const waitingNotice = host.querySelector('[data-testid="shift-open-waiting"]');
    expect(waitingNotice, 'no waiting notice').toBeNull();
  });

  it('4. a WAITING OPEN count by another teller: shift-open-count (it is not mine; do not block me)', async () => {
    const someoneElseCount = makeCashCountRow({
      id: 'c-other-open-1',
      branchId: BRANCH,
      shiftId: null,
      kind: 'OPEN',
      countedBy: OTHER,
      countedAt: '2026-10-11T04:30:00Z',
      status: 'WAITING',
      verifiedBy: null,
      verifiedAt: null,
    });
    stub([someoneElseCount]);
    mount();
    await settle();

    const openBtn = host.querySelector('[data-testid="shift-open-count"]');
    expect(openBtn, 'open count stays for me').not.toBeNull();

    const waitingNotice = host.querySelector('[data-testid="shift-open-waiting"]');
    expect(waitingNotice, 'no waiting notice for me').toBeNull();
  });

  it('5. shift open (my OPEN count VERIFIED): today\'s open-shift actions; neither open-count nor open-waiting', async () => {
    const verifiedCount = makeCashCountRow({
      id: 'c-verified-open-1',
      branchId: BRANCH,
      shiftId: MY_SHIFT_ID,
      kind: 'OPEN',
      countedBy: ME,
      countedAt: '2026-10-11T04:30:00Z',
      status: 'VERIFIED',
      verifiedBy: OTHER,
      verifiedAt: '2026-10-11T04:35:00Z',
    });
    stub([verifiedCount]);
    mount(me(), floor({ myShift: { id: MY_SHIFT_ID, openedAt: '2026-10-11T04:35:00Z' } }));
    await settle();

    const openBtn = host.querySelector('[data-testid="shift-open-count"]');
    expect(openBtn, 'neither open-count').toBeNull();

    const waitingNotice = host.querySelector('[data-testid="shift-open-waiting"]');
    expect(waitingNotice, 'nor open-waiting').toBeNull();

    // today's open-shift actions
    const closeBtn = host.querySelector('[data-testid="shift-close-count"]');
    expect(closeBtn, 'today open-shift close action is offered').not.toBeNull();
    const handoverBtn = host.querySelector('[data-testid="shift-handover"]');
    expect(handoverBtn, 'today open-shift handover action is offered').not.toBeNull();
  });
});
