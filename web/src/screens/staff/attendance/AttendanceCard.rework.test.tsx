import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type { StaffFloorView } from '../../../shared/api/generated/staff_floor';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import type { StaffCtx } from '../cards';
import { AttendanceCard } from './AttendanceCard';

const BRANCH = 'MAIN';

function me(permissions = ['AttendanceClock']): StaffMeView {
  return {
    userId: '00000000-0000-4000-8000-0000000000d1',
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  };
}

function floor(): StaffFloorView {
  return {
    phase: 'OPEN',
    hours: null,
    myClockIn: null,
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  };
}

function makeCtx(): StaffCtx {
  return {
    me: me(),
    floor: floor(),
    refresh: vi.fn(async () => {}),
  } as unknown as StaffCtx;
}

describe('AttendanceCard rework tests (#1720: worded errors on attendance read failure)', () => {
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
  });

  it('row 1 (AttendanceCard.tsx:23): failed read shows worded error for 500 network failure', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_attendance')) {
          return new Response('Internal Server Error', {
            status: 500,
            statusText: 'Internal Server Error',
            headers: { 'content-type': 'text/plain' },
          });
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="attendance-error"]');
      expect(errEl, 'attendance-error must be rendered when attendance read fails').not.toBeNull();
      expect(errEl?.textContent).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    });
  });

  it('row 1 (AttendanceCard.tsx:23): failed read shows worded error for known refusal code', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_attendance')) {
          return new Response(
            JSON.stringify({
              error: {
                code: 'roleNotAllowed',
                message: 'role not permitted',
              },
            }),
            {
              status: 403,
              headers: { 'content-type': 'application/json' },
            },
          );
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="attendance-error"]');
      expect(errEl, 'attendance-error must be rendered on refusal').not.toBeNull();
      expect(errEl?.textContent).toBe(errorText('roleNotAllowed', ''));
    });
  });

  it('row 1 (AttendanceCard.tsx:23): failed read shows generic worded message for unknown refusal code (never raw code or message)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_attendance')) {
          return new Response(
            JSON.stringify({
              error: {
                code: 'unknownAttendanceCode',
                message: 'database connection dropped while fetching attendance',
              },
            }),
            {
              status: 400,
              headers: { 'content-type': 'application/json' },
            },
          );
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="attendance-error"]');
      expect(errEl, 'attendance-error must be rendered on unknown refusal code').not.toBeNull();
      expect(errEl?.textContent).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
      expect(errEl?.textContent).not.toContain('unknownAttendanceCode');
      expect(errEl?.textContent).not.toContain('database connection dropped');
    });
  });

  it('row 1 (AttendanceCard.tsx:23): worded error renders in active language (en)', async () => {
    setLang('en');
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_attendance')) {
          return new Response('Internal Server Error', {
            status: 500,
            statusText: 'Internal Server Error',
            headers: { 'content-type': 'text/plain' },
          });
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    const ctx = makeCtx();
    dispose = render(() => <AttendanceCard ctx={ctx} />, host);

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="attendance-error"]');
      expect(errEl, 'attendance-error must be rendered').not.toBeNull();
      expect(errEl?.textContent).toBe('Server error');
    });
  });
});
