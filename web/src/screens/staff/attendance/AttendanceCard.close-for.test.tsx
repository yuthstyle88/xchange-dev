// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm (CE, Bangkok) from the shared formatter.
// SC-FE1 #5311: a person holding AttendanceCloseFor closes a forgotten earlier-day clock-in
// on /staff (form + "ค้างจากวันก่อน" label).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeAttendanceClosed,
  makeOpenAttendanceRow,
  makeStaffAttendanceRow,
  makeStaffFloorView,
  makeStaffMeView,
  makeV3ErrorBody,
  makeV3ErrorDetail,
  makeViewAnswer,
  type OpenAttendanceRow,
  type StaffAttendanceRow,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { AttendanceCard } from './AttendanceCard';
import { checkCloseFor, toRfc3339Bangkok } from './close-for';
import { th } from './i18n';

/** 2026-10-28 00:30 ICT = 2026-10-27 17:30 UTC (UTC is still 10-27 while Bangkok is 10-28). */
const NOW = new Date('2026-10-27T17:30:00Z');
const BRANCH = 'MAIN';
const CALLER_ID = '00000000-0000-4000-8000-0000000000c1';

function makeCtx(permissions: string[]): StaffCtx {
  return {
    me: makeStaffMeView({
      userId: CALLER_ID,
      name: 'Ploy',
      roles: [],
      branchId: BRANCH,
      branchName: 'XC-B',
      permissions,
    }),
    floor: makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: null,
      myShift: null,
      cutoff: null,
      target: null,
      safe: null,
    }),
    refresh: vi.fn(async () => {}),
  };
}

function row1() {
  // 2026-10-26 09:00 ICT = 2026-10-26 02:00 UTC
  return makeOpenAttendanceRow({
    id: 'open-1',
    employeeId: 'emp-1',
    name: 'Somchai',
    clockIn: '2026-10-26T02:00:00.000Z',
  });
}

function row2() {
  // 2026-10-25 10:30 ICT = 2026-10-25 03:30 UTC
  return makeOpenAttendanceRow({
    id: 'open-2',
    employeeId: 'emp-2',
    name: 'Malee',
    clockIn: '2026-10-25T03:30:00.000Z',
  });
}

interface SentPost {
  path: string;
  body: Record<string, unknown>;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function typeInput(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('SC-FE1 #5311 AttendanceCard close-for and earlier-day label', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let reads: string[];
  let posts: SentPost[];
  let openRowsData: OpenAttendanceRow[];
  let staffRowsData: StaffAttendanceRow[];
  let postAnswer: { status: number; body: unknown };
  let onPost: (() => void) | undefined;

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(NOW);
    setLang('th');
    reads = [];
    posts = [];
    openRowsData = [row1(), row2()];
    staffRowsData = [];
    postAnswer = {
      status: 200,
      body: makeAttendanceClosed({
        id: 'open-1',
        employeeId: 'emp-1',
        clockIn: '2026-10-26T02:00:00.000Z',
        clockOut: '2026-10-26T16:59:00.000Z',
      }),
    };
    onPost = undefined;
    host = document.createElement('div');
    document.body.appendChild(host);

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = raw.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();
        if (method === 'GET') {
          const viewName = path.startsWith('/api/v3/views/') ? path.slice('/api/v3/views/'.length) : path;
          reads.push(viewName);
          if (viewName === 'staff_attendance') {
            return json(200, makeViewAnswer(staffRowsData));
          }
          if (viewName === 'open_attendance') {
            return json(200, makeViewAnswer(openRowsData));
          }
          return json(404, makeV3ErrorBody({ error: makeV3ErrorDetail({ code: 'notFound', message: 'not found' }) }));
        }
        if (method === 'POST') {
          const parsed = typeof init?.body === 'string' && init.body ? (JSON.parse(init.body) as Record<string, unknown>) : {};
          posts.push({ path, body: parsed });
          onPost?.();
          return json(postAnswer.status, postAnswer.body);
        }
        return json(404, {});
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 8; i++) {
      await Promise.resolve();
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  it('1. me without AttendanceCloseFor: no list, no request to open_attendance', async () => {
    const ctx = makeCtx(['AttendanceClock']);
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);
    await settle();

    expect(reads).toContain('staff_attendance');
    expect(reads).not.toContain('open_attendance');
    expect(host.querySelectorAll('[data-open-attendance-id]')).toHaveLength(0);
    expect(host.textContent).not.toContain('ปิดเวลาออก');
  });

  it('2. me with AttendanceCloseFor, view has 2 rows: shows 2 rows, each with "ปิดเวลาออก" and dd/mm/yyyy HH:mm', async () => {
    const ctx = makeCtx(['AttendanceClock', 'AttendanceCloseFor']);
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);
    await settle();

    expect(reads).toContain('open_attendance');
    const items = host.querySelectorAll('[data-open-attendance-id]');
    expect(items).toHaveLength(2);
    expect(items[0]?.textContent).toContain('Somchai');
    expect(items[0]?.textContent).toContain('26/10/2026 09:00');
    expect(items[0]?.querySelector('button')?.textContent).toBe('ปิดเวลาออก');
    expect(items[1]?.textContent).toContain('Malee');
    expect(items[1]?.textContent).toContain('25/10/2026 10:30');
    expect(items[1]?.querySelector('button')?.textContent).toBe('ปิดเวลาออก');
  });

  it('3. submit valid time + reason -> 200: row gone; one POST /api/attendance/{id}/close with clockOut ending +07:00', async () => {
    const ctx = makeCtx(['AttendanceClock', 'AttendanceCloseFor']);
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);
    await settle();

    const firstRow = host.querySelector('[data-open-attendance-id="open-1"]')!;
    firstRow.querySelector<HTMLButtonElement>('button')!.click();
    await settle();

    const clockOutInput = host.querySelector<HTMLInputElement>('input[name="clockOut"]')!;
    expect(clockOutInput.value).toBe('2026-10-26T23:59');

    const reasonInput = host.querySelector<HTMLInputElement>('input[name="reason"]')!;
    typeInput(reasonInput, 'ลืมลงเวลาออกหลังปิดร้าน');

    const form = host.querySelector<HTMLFormElement>('[data-testid="attendance-close-form"]')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(posts).toHaveLength(1);
    expect(posts[0]!.path).toBe('/api/attendance/open-1/close');
    expect(String(posts[0]!.body.clockOut)).toMatch(/\+07:00$/);
    expect(posts[0]!.body.clockOut).toBe('2026-10-26T23:59:00+07:00');
    expect(posts[0]!.body.reason).toBe('ลืมลงเวลาออกหลังปิดร้าน');

    expect(host.querySelector('[data-open-attendance-id="open-1"]')).toBeNull();
    expect(host.querySelector('[data-open-attendance-id="open-2"]')).not.toBeNull();
  });

  it('4. clock-out before clock-in: beforeClockIn text under clock-out, no request sent', async () => {
    const ctx = makeCtx(['AttendanceClock', 'AttendanceCloseFor']);
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);
    await settle();

    host.querySelector<HTMLButtonElement>('[data-open-attendance-id="open-1"] button')!.click();
    await settle();

    const clockOutInput = host.querySelector<HTMLInputElement>('input[name="clockOut"]')!;
    typeInput(clockOutInput, '2026-10-26T08:30');
    const reasonInput = host.querySelector<HTMLInputElement>('input[name="reason"]')!;
    typeInput(reasonInput, 'ลืมลงเวลา');

    const form = host.querySelector<HTMLFormElement>('[data-testid="attendance-close-form"]')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(posts).toHaveLength(0);
    const errEl = host.querySelector('[data-field-error="clockOut"]');
    expect(errEl).not.toBeNull();
    expect(errEl?.textContent).toBe((th as Record<string, string>).beforeClockIn);
  });

  it('5. clock-out today (Bangkok): notEarlierDay under clock-out, no request', async () => {
    const ctx = makeCtx(['AttendanceClock', 'AttendanceCloseFor']);
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);
    await settle();

    host.querySelector<HTMLButtonElement>('[data-open-attendance-id="open-1"] button')!.click();
    await settle();

    // NOW is 2026-10-28 00:30 ICT (2026-10-27 17:30 UTC), so 2026-10-28T00:10 ICT is today in Bangkok
    const clockOutInput = host.querySelector<HTMLInputElement>('input[name="clockOut"]')!;
    typeInput(clockOutInput, '2026-10-28T00:10');
    const reasonInput = host.querySelector<HTMLInputElement>('input[name="reason"]')!;
    typeInput(reasonInput, 'ลืมลงเวลา');

    const form = host.querySelector<HTMLFormElement>('[data-testid="attendance-close-form"]')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(posts).toHaveLength(0);
    const errEl = host.querySelector('[data-field-error="clockOut"]');
    expect(errEl).not.toBeNull();
    expect(errEl?.textContent).toBe((th as Record<string, string>).notEarlierDay);
  });

  it('6. blank reason: reasonRequired under reason, no request', async () => {
    const ctx = makeCtx(['AttendanceClock', 'AttendanceCloseFor']);
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);
    await settle();

    host.querySelector<HTMLButtonElement>('[data-open-attendance-id="open-1"] button')!.click();
    await settle();

    const reasonInput = host.querySelector<HTMLInputElement>('input[name="reason"]')!;
    typeInput(reasonInput, '   ');

    const form = host.querySelector<HTMLFormElement>('[data-testid="attendance-close-form"]')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(posts).toHaveLength(0);
    const errEl = host.querySelector('[data-field-error="reason"]');
    expect(errEl).not.toBeNull();
    expect(errEl?.textContent).toBe((th as Record<string, string>).reasonRequired);
  });

  it('7. server 409 notOpen: form line shows its text; row stays (and refreshes list)', async () => {
    postAnswer = {
      status: 409,
      body: makeV3ErrorBody({
        error: makeV3ErrorDetail({ code: 'notOpen', message: 'Attendance row is not open' }),
      }),
    };
    const ctx = makeCtx(['AttendanceClock', 'AttendanceCloseFor']);
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);
    await settle();

    const openReadsBefore = reads.filter((r) => r === 'open_attendance').length;
    host.querySelector<HTMLButtonElement>('[data-open-attendance-id="open-1"] button')!.click();
    await settle();

    const reasonInput = host.querySelector<HTMLInputElement>('input[name="reason"]')!;
    typeInput(reasonInput, 'ปิดย้อนหลัง');

    const form = host.querySelector<HTMLFormElement>('[data-testid="attendance-close-form"]')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(posts).toHaveLength(1);
    const formErr = host.querySelector('[data-testid="attendance-close-error"]');
    expect(formErr).not.toBeNull();
    expect(formErr?.textContent).toBe((th as Record<string, string>).notOpen);
    expect(host.querySelector('[data-open-attendance-id="open-1"]')).not.toBeNull();
    expect(reads.filter((r) => r === 'open_attendance').length).toBeGreaterThan(openReadsBefore);
  });

  it('8. server 400 inFuture field clockOut: shows under clock-out', async () => {
    postAnswer = {
      status: 400,
      body: makeV3ErrorBody({
        error: makeV3ErrorDetail({
          code: 'inFuture',
          field: 'clockOut',
          message: 'Clock-out time cannot be in the future',
        }),
      }),
    };
    const ctx = makeCtx(['AttendanceClock', 'AttendanceCloseFor']);
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);
    await settle();

    host.querySelector<HTMLButtonElement>('[data-open-attendance-id="open-1"] button')!.click();
    await settle();

    const reasonInput = host.querySelector<HTMLInputElement>('input[name="reason"]')!;
    typeInput(reasonInput, 'ปิดย้อนหลัง');

    const form = host.querySelector<HTMLFormElement>('[data-testid="attendance-close-form"]')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(posts).toHaveLength(1);
    const errEl = host.querySelector('[data-field-error="clockOut"]');
    expect(errEl).not.toBeNull();
    expect(errEl?.textContent).toBe((th as Record<string, string>).inFuture);
  });

  it('9. earlier-day row on the main card: has the "ค้างจากวันก่อน" label', async () => {
    staffRowsData = [
      makeStaffAttendanceRow({
        id: 'att-today',
        name: 'Somchai',
        in: '2026-10-27T17:10:00Z', // 2026-10-28 00:10 ICT (today)
        out: null,
      }),
      makeStaffAttendanceRow({
        id: 'att-earlier',
        name: 'Ploy',
        in: '2026-10-26T01:00:00Z', // 2026-10-26 08:00 ICT (earlier day)
        out: null,
      }),
    ];
    const ctx = makeCtx(['AttendanceClock']);
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);
    await settle();

    const todayRow = host.querySelector('[data-attendance-id="att-today"]')!;
    const earlierRow = host.querySelector('[data-attendance-id="att-earlier"]')!;
    expect(todayRow.textContent).not.toContain('ค้างจากวันก่อน');
    expect(earlierRow.classList.contains('xc-attendance-row-earlier')).toBe(true);
    expect(earlierRow.textContent).toContain('ค้างจากวันก่อน');
  });

  it('10. edge case (§9): timezone edge in Asia/Bangkok — 23:30 ICT yesterday passes; 00:10 ICT today is notEarlierDay while UTC is still yesterday', () => {
    // Clock-in at 23:30 ICT yesterday (2026-10-27T16:30:00Z), closed at 23:45 ICT yesterday
    expect(checkCloseFor('2026-10-27T16:30:00Z', '2026-10-27T23:45', NOW)).toBeUndefined();
    // Clock-in earlier, closed at 23:30 ICT yesterday passes
    expect(checkCloseFor('2026-10-26T02:00:00Z', '2026-10-27T23:30', NOW)).toBeUndefined();
    // 00:10 ICT today (2026-10-27T17:10:00Z, still 10-27 in UTC) is notEarlierDay
    expect(checkCloseFor('2026-10-26T02:00:00Z', '2026-10-28T00:10', NOW)).toEqual({
      field: 'clockOut',
      code: 'notEarlierDay',
    });
    expect(toRfc3339Bangkok('2026-10-27T23:30')).toBe('2026-10-27T23:30:00+07:00');
  });
});
