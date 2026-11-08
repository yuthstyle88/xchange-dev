import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeStaffAttendanceRow,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { AttendanceCard } from './AttendanceCard';

function me() {
  return makeStaffMeView({
    userId: '00000000-0000-4000-8000-0000000000c1',
    name: 'Ploy',
    roles: [],
    branchId: 'MAIN',
    branchName: 'XC-B',
    permissions: [],
  });
}

function floor() {
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

function makeCtx(): StaffCtx {
  return {
    me: me(),
    floor: floor(),
    refresh: vi.fn(async () => {}),
  } as unknown as StaffCtx;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function view(data: unknown): Response {
  return json(200, { version: 1, changed: true, data });
}

describe('AttendanceCard look tests (#5135)', () => {
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
    host.remove();
    setLang('th');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function flush(): Promise<void> {
    for (let i = 0; i < 8; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  it('renders root xc-attendance-card with data-testid="attendance-card", xc-attendance-header, h2.xc-screen-title, and list rows as xc-attendance-row', async () => {
    const rows = [
      makeStaffAttendanceRow({
        id: 'att-1',
        name: 'Somchai',
        out: null,
      }),
      makeStaffAttendanceRow({
        id: 'att-2',
        name: 'Somsak',
        out: '2026-10-11T09:00:00Z',
      }),
    ];

    vi.stubGlobal(
      'fetch',
      vi.fn(async () => view(rows)),
    );

    dispose = render(() => <AttendanceCard ctx={makeCtx()} />, host);
    await flush();

    const root = host.querySelector('[data-testid="attendance-card"].xc-attendance-card');
    expect(root).not.toBeNull();

    const header = host.querySelector('.xc-attendance-header');
    expect(header).not.toBeNull();

    const title = header?.querySelector('h2.xc-screen-title');
    expect(title).not.toBeNull();
    expect(title?.textContent).toBe('ลงเวลาวันนี้');

    const row1 = host.querySelector('[data-attendance-id="att-1"]');
    expect(row1).not.toBeNull();
    expect(row1?.classList.contains('xc-attendance-row')).toBe(true);
    expect(row1?.textContent).toContain('Somchai');
    expect(row1?.textContent).toContain('ยังไม่ออก');

    const row2 = host.querySelector('[data-attendance-id="att-2"]');
    expect(row2).not.toBeNull();
    expect(row2?.classList.contains('xc-attendance-row')).toBe(true);
    expect(row2?.textContent).toContain('Somsak');
    expect(row2?.textContent).not.toContain('ยังไม่ออก');
  });

  it('renders English title when lang is en', async () => {
    setLang('en');
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => view([])),
    );

    dispose = render(() => <AttendanceCard ctx={makeCtx()} />, host);
    await flush();

    const title = host.querySelector('.xc-screen-title');
    expect(title?.textContent).toBe("Today's attendance");
  });
});
