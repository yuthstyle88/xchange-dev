// #5641 (DESK-SHIFT T4, docs/design/desk-shift-open.md §3 T4 rows G and D1–D4): the /desk shift gate's panel.
// Written before the implementation by the ticket author claude/studio-claude-15. Do not edit, delete or weaken;
// add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, Show } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeCashCountRow,
  makeDenominationRow,
  makeMe,
  makeStaffFloorView,
  makeStaffMeView,
  type Me,
  makeOpenShiftChoice,
  makeOpenShiftChoices,
} from '../../../shared/testing/fixtures';
import { publishFreshNotices } from '../../../shared/ui/notice-bell/notice-feed';
import { th } from '../i18n';
import { createDeskShiftGate } from './DeskShiftGate';

const TELLER = makeMe({ id: 'u-1', name: 'Ploy', branch: 'MAIN', permissions: ['ShiftOperate', 'AttendanceClock'] });
const OWNER = makeMe({ id: 'u-o', name: 'Owner', branch: 'MAIN', permissions: ['ReversalRequest', 'OverviewRead'] });

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}
const view = (data: unknown) => json(200, { version: 1, changed: true, data });

interface Server {
  clockedIn: boolean;
  shiftOpen: boolean;
  counts: ReturnType<typeof makeCashCountRow>[];
  staffMeFails: boolean;
}
let server: Server;
let calls: string[] = [];

// CS-15m: the OPEN count form now reads open_shift_choices (ShiftPicker); one covering shift keeps Save usable as
// before. Stub restated per the lane controller's ruling on PR 6274.
const ONE_SHIFT = makeOpenShiftChoices({ choices: [makeOpenShiftChoice({ templateId: 'tpl-1' })], preselect: 'tpl-1' });

function stub() {
  calls = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const path = url.split('?')[0]!;
      const method = (init?.method ?? 'GET').toUpperCase();
      calls.push(`${method} ${path}`);
      if (method === 'GET' && path === '/api/v3/views/staff_me') {
        if (server.staffMeFails) return json(500, { error: { code: 'internal', message: 'boom' } });
        return view(makeStaffMeView({ userId: 'u-1', name: 'Ploy', roles: [], branchId: 'MAIN', branchName: 'XC-B', permissions: ['ShiftOperate', 'AttendanceClock'] }));
      }
      if (method === 'GET' && path === '/api/v3/views/staff_floor') {
        return view(
          makeStaffFloorView({
            phase: 'OPEN',
            hours: null,
            myClockIn: server.clockedIn ? '2026-10-30T01:00:00Z' : null,
            myShift: server.shiftOpen ? { id: 'SH-1', openedAt: '2026-10-30T01:05:00Z' } : null,
            cutoff: null,
            target: null,
            safe: null,
          }),
        );
      }
      if (method === 'GET' && path === '/api/v3/views/cash_counts') return view(server.counts);
      if (method === 'GET' && path === '/api/v3/views/denominations') {
        return view({ rows: [makeDenominationRow({ id: 164, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' })] });
      }
      if (method === 'GET' && path === '/api/v3/views/open_shift_choices') return view(ONE_SHIFT);
      if (method === 'POST' && path === '/api/attendance') {
        server.clockedIn = true;
        return json(200, { success: true });
      }
      if (method === 'POST' && path === '/api/cash-counts') {
        server.counts = [makeCashCountRow({ id: 'c-new', kind: 'OPEN', countedBy: 'u-1', countedAt: '2026-10-30T03:53:00Z' })];
        return json(200, { id: 'c-new' });
      }
      return json(404, { error: { code: 'notFound', message: `${method} ${path}` } });
    }),
  );
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mount(initial: Me | undefined) {
  const [me, setMe] = createSignal<Me | undefined>(initial);
  dispose = render(() => {
    const gate = createDeskShiftGate({ me, branch: () => 'MAIN' });
    return (
      <Show when={gate.open()} fallback={gate.Panel()}>
        <p data-testid="steps">steps</p>
      </Show>
    );
  }, host);
  return { setMe };
}

async function settle() {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
}

const q = (sel: string) => host.querySelector(sel);
const text = () => host.textContent ?? '';
const staffReads = () => calls.filter((c) => /staff_me|staff_floor|cash_counts/.test(c));

describe('#5641 createDeskShiftGate', () => {
  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    server = { clockedIn: false, shiftOpen: false, counts: [], staffMeFails: false };
    stub();
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
  });

  it('G1 / D1 no session me yet: checking, and no staff read; once me arrives with ShiftOperate the reads start', async () => {
    const { setMe } = mount(undefined);
    await settle();
    expect(text()).toContain(th['shift.checking']);
    expect(staffReads()).toEqual([]);
    setMe(TELLER);
    await settle();
    expect(calls).toContain('GET /api/v3/views/staff_me');
    expect(calls).toContain('GET /api/v3/views/staff_floor');
    expect(calls).toContain('GET /api/v3/views/cash_counts');
  });

  it("G4 without ShiftOperate: v2's bar, and no staff_me, staff_floor or cash_counts read (owner 401, #3970)", async () => {
    mount(OWNER);
    await settle();
    expect(text()).toContain(th['shift.noShift']);
    expect(q('[data-testid="steps"]')).toBeNull();
    expect(staffReads()).toEqual([]);
  });

  it('G7 / D2 not clocked in: the line and the clock button; a click clocks in and moves on to the open count', async () => {
    mount(TELLER);
    await settle();
    expect(text()).toContain(th['shift.clockInFirst']);
    const clock = q('[data-testid="floor-clock"]') as HTMLButtonElement;
    expect(clock).not.toBeNull();
    clock.click();
    await settle();
    expect(calls).toContain('POST /api/attendance');
    expect(text()).toContain(th['shift.openCountFirst']);
    expect(q('[data-testid="shift-open-count"]')).not.toBeNull();
  });

  it('G10 / D2 the open count: the button opens the count form; a save shows the waiting line', async () => {
    server.clockedIn = true;
    mount(TELLER);
    await settle();
    expect(text()).toContain(th['shift.openCountFirst']);
    (q('[data-testid="shift-open-count"]') as HTMLButtonElement).click();
    await settle();
    const input = q('input[data-denomination-id="164"]') as HTMLInputElement;
    expect(input).not.toBeNull();
    input.value = '10';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    (q('[data-testid="shift-count-form"]') as HTMLFormElement).dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    expect(calls).toContain('POST /api/cash-counts');
    expect(q('[data-testid="shift-count-form"]')).toBeNull();
    expect(q('[data-testid="shift-open-waiting"]')).not.toBeNull();
  });

  it('G9 my OPEN count already waits: the waiting line, no count button', async () => {
    server.clockedIn = true;
    server.counts = [makeCashCountRow({ id: 'c-1', kind: 'OPEN', countedBy: 'u-1', countedAt: '2026-10-30T03:53:00Z' })];
    mount(TELLER);
    await settle();
    expect(q('[data-testid="shift-open-waiting"]')).not.toBeNull();
    expect(q('[data-testid="shift-open-count"]')).toBeNull();
  });

  it('G3 an open shift: the steps, no panel', async () => {
    server.clockedIn = true;
    server.shiftOpen = true;
    mount(TELLER);
    await settle();
    expect(q('[data-testid="steps"]')).not.toBeNull();
    expect(text()).not.toContain(th['shift.openCountFirst']);
  });

  it('D3 a fresh cash_count notice after the verify opens the steps without a reload', async () => {
    server.clockedIn = true;
    server.counts = [makeCashCountRow({ id: 'c-1', kind: 'OPEN', countedBy: 'u-1', countedAt: '2026-10-30T03:53:00Z' })];
    mount(TELLER);
    await settle();
    expect(q('[data-testid="shift-open-waiting"]')).not.toBeNull();
    server.shiftOpen = true;
    server.counts = [];
    publishFreshNotices([{ seq: 1, objectType: 'cash_count', objectId: 'c-1' }]);
    await settle();
    expect(q('[data-testid="steps"]')).not.toBeNull();
  });

  it('G2 staff_me fails: a worded error and a retry that reads again', async () => {
    server.staffMeFails = true;
    mount(TELLER);
    await settle();
    expect(q('[data-testid="steps"]')).toBeNull();
    const retry = q('button[data-action="shift-retry"]') as HTMLButtonElement;
    expect(retry).not.toBeNull();
    expect(retry.textContent?.trim()).toBe(th['shift.retry']);
    const before = calls.filter((c) => c === 'GET /api/v3/views/staff_me').length;
    server.staffMeFails = false;
    server.clockedIn = true;
    server.shiftOpen = true;
    retry.click();
    await settle();
    expect(calls.filter((c) => c === 'GET /api/v3/views/staff_me').length).toBe(before + 1);
    expect(q('[data-testid="steps"]')).not.toBeNull();
  });

  it('D6 the new texts: Thai, and the same Thai in en', async () => {
    const { en } = await import('../i18n');
    for (const k of ['shift.checking', 'shift.retry', 'shift.noShift', 'shift.clockInFirst', 'shift.openCountFirst', 'shift.openCount'] as const) {
      expect(th[k]).toMatch(/[฀-๿]/);
      expect(en[k]).toBe(th[k]);
    }
    expect(th['shift.noShift']).toBe('ยังไม่มีกะเปิด กรุณาเปิดกะจากหน้าแอดมินก่อน');
  });
});
