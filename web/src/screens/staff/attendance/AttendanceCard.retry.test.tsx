import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import {
  makeStaffAttendanceRow,
  makeStaffFloorView,
  makeStaffMeView,
  type StaffAttendanceRow,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import * as staffI18n from '../i18n';
import { AttendanceCard } from './AttendanceCard';
import * as attI18n from './i18n';

const BRANCH = 'MAIN';

const SAMPLE_ROWS = [
  makeStaffAttendanceRow({ id: 'att-3', name: 'Ploy', in: '2026-10-11T03:00:00Z', out: null }),
  makeStaffAttendanceRow({ id: 'att-2', name: 'Somchai', in: '2026-10-11T01:00:00Z', out: '2026-10-11T02:30:00Z' }),
];

function me(permissions = ['AttendanceClock']) {
  return makeStaffMeView({
    userId: '00000000-0000-4000-8000-0000000000d1',
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  });
}

function floor() {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: null,
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
  };
}

function okAttendanceResponse(rows: StaffAttendanceRow[] = SAMPLE_ROWS): Response {
  return new Response(JSON.stringify({ version: 1, changed: true, data: rows }), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
}

function serverErrorResponse(): Response {
  return new Response('Internal Server Error', {
    status: 500,
    statusText: 'Internal Server Error',
    headers: { 'content-type': 'text/plain' },
  });
}

describe('AttendanceCard retry on failed read (#2355)', () => {
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
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    setLang('th');
  });

  it('read 500s: shows worded error plus a retry button', async () => {
    let reads = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_attendance')) {
          reads += 1;
          return serverErrorResponse();
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    dispose = render(() => <AttendanceCard ctx={makeCtx()} />, host);

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="attendance-error"]');
      expect(errEl).not.toBeNull();
      expect(errEl?.textContent).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    });

    const retryBtn = host.querySelector<HTMLButtonElement>('[data-card="attendance"] button');
    expect(retryBtn, 'retry button must be rendered when attendance read fails').not.toBeNull();
    expect(retryBtn?.textContent).toBe(staffI18n.th.refresh);
    expect(reads).toBe(1);
  });

  it('retry, then 200 with rows: error and button are gone and rows are shown', async () => {
    let reads = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_attendance')) {
          reads += 1;
          if (reads === 1) return serverErrorResponse();
          return okAttendanceResponse(SAMPLE_ROWS);
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    dispose = render(() => <AttendanceCard ctx={makeCtx()} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="attendance-error"]')).not.toBeNull();
    });

    const retryBtn = host.querySelector<HTMLButtonElement>('[data-card="attendance"] button');
    expect(retryBtn).not.toBeNull();
    retryBtn!.click();

    await vi.waitFor(() => {
      expect(host.querySelectorAll('[data-attendance-id]')).toHaveLength(2);
    });

    expect(reads).toBe(2);
    expect(host.querySelector('[data-testid="attendance-error"]')).toBeNull();
    expect(host.querySelector('[data-card="attendance"] button')).toBeNull();
    expect(host.querySelector('[data-attendance-id="att-3"]')?.textContent).toContain('Ploy');
    expect(host.querySelector('[data-attendance-id="att-3"]')?.textContent).toContain(attI18n.th.stillIn);
    expect(host.querySelector('[data-attendance-id="att-2"]')?.textContent).toContain('Somchai');
  });

  it('retry, then fails again: error still shown, button still there, and exactly 2 reads', async () => {
    let reads = 0;
    let resolveSecond!: (res: Response) => void;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_attendance')) {
          reads += 1;
          if (reads === 1) return serverErrorResponse();
          return new Promise<Response>((resolve) => {
            resolveSecond = resolve;
          });
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    dispose = render(() => <AttendanceCard ctx={makeCtx()} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="attendance-error"]')).not.toBeNull();
    });

    const retryBtn = host.querySelector<HTMLButtonElement>('[data-card="attendance"] button');
    expect(retryBtn).not.toBeNull();
    retryBtn!.click();

    await vi.waitFor(() => {
      expect(reads).toBe(2);
      expect(resolveSecond).toBeTypeOf('function');
    });
    resolveSecond(serverErrorResponse());
    for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));

    const errEl = host.querySelector('[data-testid="attendance-error"]');
    expect(errEl).not.toBeNull();
    expect(errEl?.textContent).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    const retryBtnAfter = host.querySelector<HTMLButtonElement>('[data-card="attendance"] button');
    expect(retryBtnAfter).not.toBeNull();
    expect(retryBtnAfter?.textContent).toBe(staffI18n.th.refresh);
  });

  it('edge case: a double click on retry sends only one read while in flight', async () => {
    let reads = 0;
    let resolveRetry!: (res: Response) => void;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_attendance')) {
          reads += 1;
          if (reads === 1) return serverErrorResponse();
          return new Promise<Response>((resolve) => {
            resolveRetry = resolve;
          });
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    dispose = render(() => <AttendanceCard ctx={makeCtx()} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="attendance-error"]')).not.toBeNull();
    });

    const retryBtn = host.querySelector<HTMLButtonElement>('[data-card="attendance"] button');
    expect(retryBtn).not.toBeNull();

    // Double click retry while the retry read is in flight
    retryBtn!.click();
    retryBtn!.click();

    expect(reads).toBe(2);

    resolveRetry(okAttendanceResponse(SAMPLE_ROWS));

    await vi.waitFor(() => {
      expect(host.querySelectorAll('[data-attendance-id]')).toHaveLength(2);
    });
    expect(reads).toBe(2);
    expect(host.querySelector('[data-testid="attendance-error"]')).toBeNull();
  });

  it('edge case: the language switch applies to both the error and the retry button', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_attendance')) {
          return serverErrorResponse();
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    dispose = render(() => <AttendanceCard ctx={makeCtx()} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="attendance-error"]')?.textContent).toBe('เซิร์ฟเวอร์ขัดข้อง');
    });

    const retryBtn = host.querySelector<HTMLButtonElement>('[data-card="attendance"] button');
    expect(retryBtn).not.toBeNull();
    expect(retryBtn?.textContent).toBe(staffI18n.th.refresh);

    setLang('en');

    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="attendance-error"]')?.textContent).toBe('Server error');
      expect(host.querySelector<HTMLButtonElement>('[data-card="attendance"] button')?.textContent).toBe(
        staffI18n.en.refresh,
      );
    });
  });
});
