// #5423: Teller with ShiftOperate only loads cash counts and sees open waiting notice
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
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
const CLOCKED_IN = '2026-10-11T01:00:00Z';

function tellerMe(permissions = ['ShiftOperate']) {
  return makeStaffMeView({
    userId: ME,
    name: 'Somchai',
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
let requestedViews: string[] = [];
let dynamicCounts: ReturnType<typeof makeCashCountRow>[] = [];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stub(counts: ReturnType<typeof makeCashCountRow>[] = []) {
  dynamicCounts = [...counts];
  requestedViews = [];
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
        if (viewName) requestedViews.push(viewName);

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

function mount(m: StaffMeView = tellerMe(), f: StaffFloorView = floor()) {
  const refresh = vi.fn(async () => {});
  const ctx: StaffCtx = { me: m, floor: f, refresh };
  dispose = render(() => <ShiftCard ctx={ctx} />, host);
  return { refresh };
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('#5423 ShiftCard teller (ShiftOperate only) open waiting', () => {
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

  it('teller with ShiftOperate only and waiting OPEN count: view is requested, shows open-waiting, hides open-count', async () => {
    const waitingCount = makeCashCountRow({
      id: 'c-wait-open-1',
      branchId: BRANCH,
      shiftId: null,
      kind: 'OPEN',
      countedBy: ME,
      countedAt: '2026-10-11T04:30:00Z',
      status: 'WAITING',
      verifiedBy: null,
      verifiedAt: null,
    });
    stub([waitingCount]);
    mount(tellerMe(['ShiftOperate']), floor());
    await settle();

    expect(requestedViews).toContain('cash_counts');
    const waitingNotice = host.querySelector('[data-testid="shift-open-waiting"]');
    expect(waitingNotice).not.toBeNull();
    const openBtn = host.querySelector('[data-testid="shift-open-count"]');
    expect(openBtn).toBeNull();
  });

  it('teller with ShiftOperate only and no waiting count: shows open-count, hides open-waiting', async () => {
    stub([]);
    mount(tellerMe(['ShiftOperate']), floor());
    await settle();

    expect(requestedViews).toContain('cash_counts');
    const openBtn = host.querySelector('[data-testid="shift-open-count"]');
    expect(openBtn).not.toBeNull();
    const waitingNotice = host.querySelector('[data-testid="shift-open-waiting"]');
    expect(waitingNotice).toBeNull();
  });

  it('user with neither permission: cash_counts is not requested', async () => {
    stub([]);
    mount(tellerMe([]), floor());
    await settle();

    expect(requestedViews).not.toContain('cash_counts');
  });
});
