// #5313: Close count waiting for certification notice and close count button visibility
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
const OLD_SHIFT_ID = '99999999-9999-4999-8999-999999999999';

function me(permissions = ['ShiftOperate']) {
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
    myShift: {
      id: MY_SHIFT_ID,
      openedAt: '2026-10-11T01:05:00Z',
    },
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

      if (method === 'POST') {
        if (path === '/api/cash-counts') {
          // Add the close count to dynamicCounts for subsequent read
          dynamicCounts.push(
            makeCashCountRow({
              id: 'c-saved-1',
              branchId: BRANCH,
              shiftId: MY_SHIFT_ID,
              kind: 'CLOSE',
              countedBy: ME,
              countedAt: '2026-10-11T03:30:00Z',
              verifiedBy: null,
              verifiedAt: null,
            }),
          );
          return json(200, { id: 'c-saved-1' });
        }
        return json(200, { id: 'ok' });
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

describe('#5313 ShiftCard close waiting notice and button visibility', () => {
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

  it('1. open shift, no CLOSE count of mine: shows นับปิดกะ button and no notice', async () => {
    stub([]);
    mount();
    await settle();

    const closeBtn = host.querySelector('[data-testid="shift-close-count"]');
    expect(closeBtn, 'close count button is offered').not.toBeNull();
    expect(closeBtn?.textContent).toContain(th.closeCount);

    const statusNotice = host.querySelector('[role="status"]');
    expect(statusNotice, 'no waiting notice').toBeNull();
  });

  it('2. my CLOSE count for this shift, unverified: shows notice with time and hides นับปิดกะ (TH and EN)', async () => {
    const waitingCount = makeCashCountRow({
      id: 'c-wait-1',
      branchId: BRANCH,
      shiftId: MY_SHIFT_ID,
      kind: 'CLOSE',
      countedBy: ME,
      countedAt: '2026-10-11T04:30:00Z', // 04:30 UTC = 11:30 Bangkok
      verifiedBy: null,
      verifiedAt: null,
    });
    stub([waitingCount]);
    mount();
    await settle();

    const closeBtn = host.querySelector('[data-testid="shift-close-count"]');
    expect(closeBtn, 'นับปิดกะ button is hidden').toBeNull();

    const statusNotice = host.querySelector('[role="status"]');
    expect(statusNotice, 'notice is role="status"').not.toBeNull();
    expect(statusNotice?.textContent).toContain('11/10 11:30');
    expect(statusNotice?.textContent).toContain(th.closeWaiting ?? 'ยอดนับปิดกะรอผู้มีสิทธิ์ตรวจรับรอง');

    // Test in English
    setLang('en');
    await settle();
    expect(statusNotice?.textContent).toContain('11/10 11:30');
    expect(statusNotice?.textContent).toContain(en.closeWaiting ?? 'Close count waiting for certification');
  });

  it('3. my CLOSE count verified: neither close button nor notice (shift closes as today)', async () => {
    const verifiedCount = makeCashCountRow({
      id: 'c-verified-1',
      branchId: BRANCH,
      shiftId: MY_SHIFT_ID,
      kind: 'CLOSE',
      countedBy: ME,
      countedAt: '2026-10-11T04:30:00Z',
      verifiedBy: OTHER,
      verifiedAt: '2026-10-11T04:35:00Z',
    });
    stub([verifiedCount]);
    // When shift is verified, floor.myShift is null
    mount(me(), floor({ myShift: null }));
    await settle();

    const closeBtn = host.querySelector('[data-testid="shift-close-count"]');
    expect(closeBtn, 'no close button when shift closed').toBeNull();

    const statusNotice = host.querySelector('[role="status"]');
    expect(statusNotice, 'no waiting notice when shift closed').toBeNull();
  });

  it('4. someone else\'s unverified CLOSE count: นับปิดกะ stays for me and no notice', async () => {
    const someoneElseCount = makeCashCountRow({
      id: 'c-other-1',
      branchId: BRANCH,
      shiftId: 'other-shift-id',
      kind: 'CLOSE',
      countedBy: OTHER,
      countedAt: '2026-10-11T04:30:00Z',
      verifiedBy: null,
      verifiedAt: null,
    });
    stub([someoneElseCount]);
    mount();
    await settle();

    const closeBtn = host.querySelector('[data-testid="shift-close-count"]');
    expect(closeBtn, 'นับปิดกะ stays for me').not.toBeNull();

    const statusNotice = host.querySelector('[role="status"]');
    expect(statusNotice, 'no waiting notice for me').toBeNull();
  });

  it('5. save a close count -> 200: notice appears without reload and นับปิดกะ is hidden', async () => {
    stub([]);
    mount();
    await settle();

    const closeBtn = host.querySelector('[data-testid="shift-close-count"]') as HTMLButtonElement | null;
    expect(closeBtn).not.toBeNull();
    closeBtn!.click();
    await settle();

    const form = host.querySelector('[data-testid="shift-count-form"]') as HTMLFormElement | null;
    expect(form, 'count form opened').not.toBeNull();

    const input = form!.querySelector('input[data-denomination-id="11"]') as HTMLInputElement | null;
    expect(input).not.toBeNull();
    input!.value = '5';
    input!.dispatchEvent(new Event('input', { bubbles: true }));

    const saveBtn = host.querySelector('[data-testid="shift-count-save"]') as HTMLButtonElement | null;
    expect(saveBtn).not.toBeNull();
    saveBtn!.click();
    await settle();

    // After save, notice appears without reload
    const statusNotice = host.querySelector('[role="status"]');
    expect(statusNotice, 'waiting notice appears without reload').not.toBeNull();
    expect(statusNotice?.textContent).toContain('11/10 10:30'); // 03:30 UTC = 10:30 Bangkok

    const closeBtnAfter = host.querySelector('[data-testid="shift-close-count"]');
    expect(closeBtnAfter, 'นับปิดกะ is hidden after save').toBeNull();
  });

  it('6. older shift CLOSE count of mine does not hide นับปิดกะ for current shift', async () => {
    const oldShiftCount = makeCashCountRow({
      id: 'c-old-1',
      branchId: BRANCH,
      shiftId: OLD_SHIFT_ID,
      kind: 'CLOSE',
      countedBy: ME,
      countedAt: '2026-10-10T12:00:00Z',
      verifiedBy: null,
      verifiedAt: null,
    });
    stub([oldShiftCount]);
    mount();
    await settle();

    const closeBtn = host.querySelector('[data-testid="shift-close-count"]');
    expect(closeBtn, 'นับปิดกะ offered for current shift').not.toBeNull();

    const statusNotice = host.querySelector('[role="status"]');
    expect(statusNotice, 'no waiting notice for older shift').toBeNull();
  });
});
