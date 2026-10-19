import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../shared/i18n';
import { ApiClient } from '../../shared/api/client';
import {
  makeStaffFloorHours,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../shared/testing/fixtures';
import type { StaffCard, StaffCtx } from './cards';
import { StaffPage } from './StaffPage';

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (err: unknown) => void;
}

function deferred<T = Response>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (err: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function okBody(data: unknown) {
  return { version: 1, changed: true, data };
}

function staffMe(name = 'Ploy', permissions: string[] = []) {
  return makeStaffMeView({
    userId: '00000000-0000-4000-8000-0000000000a1',
    name,
    roles: [],
    branchId: 'MAIN',
    branchName: 'XC-B',
    permissions,
  });
}

// 10:00–20:00 Asia/Bangkok
const FLOOR_INITIAL = makeStaffFloorView({
  phase: 'OPEN',
  hours: makeStaffFloorHours({ open: '2026-10-11T03:00:00Z', close: '2026-10-11T13:00:00Z' }),
  myClockIn: null,
  myShift: null,
  cutoff: null,
  target: null,
  safe: null,
});

// 08:00–17:00 Asia/Bangkok (Refresh A - stale)
const FLOOR_A = makeStaffFloorView({
  ...FLOOR_INITIAL,
  hours: makeStaffFloorHours({ open: '2026-10-11T01:00:00Z', close: '2026-10-11T10:00:00Z' }),
});

// 11:00–21:00 Asia/Bangkok (Refresh B - latest)
const FLOOR_B = makeStaffFloorView({
  ...FLOOR_INITIAL,
  hours: makeStaffFloorHours({ open: '2026-10-11T04:00:00Z', close: '2026-10-11T14:00:00Z' }),
});

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
  }
}

describe('StaffPage refresh request ordering (#2336)', () => {
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
    setLang('th');
  });

  it("row 1: Refresh A then B; B answers first, then A -> Screen shows B's floor; A's answer ignored", async () => {
    let floorCall = 0;
    const floorReqA = deferred<Response>();
    const floorReqB = deferred<Response>();

    const fetchFn = vi.fn((input: RequestInfo | URL) => {
      const path = String(input).split('?')[0];
      if (path === '/api/v3/views/staff_me') {
        return Promise.resolve(jsonResponse(okBody(staffMe())));
      }
      if (path === '/api/v3/views/staff_floor') {
        floorCall += 1;
        if (floorCall === 1) {
          return Promise.resolve(jsonResponse(okBody(FLOOR_INITIAL)));
        }
        if (floorCall === 2) {
          return floorReqA.promise;
        }
        return floorReqB.promise;
      }
      throw new Error(`unexpected request ${String(input)}`);
    });

    const navigate = vi.fn();
    const api = new ApiClient(fetchFn as unknown as typeof fetch);
    dispose = render(() => <StaffPage api={api} cards={[]} navigate={navigate} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('10:00');
    });

    const refreshBtn = host.querySelector('[data-testid="staff-refresh"]') as HTMLButtonElement;
    // Trigger Refresh A, then Refresh B
    refreshBtn.click();
    await flush();
    refreshBtn.click();
    await flush();
    expect(floorCall).toBe(3);

    // B answers first (11:00–21:00)
    floorReqB.resolve(jsonResponse(okBody(FLOOR_B)));
    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('11:00');
    });

    // A answers last (08:00–17:00) -> must be ignored
    floorReqA.resolve(jsonResponse(okBody(FLOOR_A)));
    await flush();

    expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('11:00');
    expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).not.toContain('08:00');
  });

  it("row 2: A fails late after B succeeded -> No error shown; B's data stays", async () => {
    let floorCall = 0;
    const floorReqA = deferred<Response>();
    const floorReqB = deferred<Response>();

    const fetchFn = vi.fn((input: RequestInfo | URL) => {
      const path = String(input).split('?')[0];
      if (path === '/api/v3/views/staff_me') {
        return Promise.resolve(jsonResponse(okBody(staffMe())));
      }
      if (path === '/api/v3/views/staff_floor') {
        floorCall += 1;
        if (floorCall === 1) {
          return Promise.resolve(jsonResponse(okBody(FLOOR_INITIAL)));
        }
        if (floorCall === 2) {
          return floorReqA.promise;
        }
        return floorReqB.promise;
      }
      throw new Error(`unexpected request ${String(input)}`);
    });

    const navigate = vi.fn();
    const api = new ApiClient(fetchFn as unknown as typeof fetch);
    dispose = render(() => <StaffPage api={api} cards={[]} navigate={navigate} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('10:00');
    });

    const refreshBtn = host.querySelector('[data-testid="staff-refresh"]') as HTMLButtonElement;
    refreshBtn.click();
    await flush();
    refreshBtn.click();
    await flush();

    // B succeeds first
    floorReqB.resolve(jsonResponse(okBody(FLOOR_B)));
    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('11:00');
    });
    expect(host.querySelector('[role="alert"]')).toBeNull();

    // A fails late with 500 -> must not overwrite B's data or show an error
    floorReqA.resolve(
      jsonResponse({ error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }, 500),
    );
    await flush();

    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('11:00');
  });

  it("row 3: B fails after A answered OK -> B's error shown, A's data still visible", async () => {
    let floorCall = 0;
    const staleReq = deferred<Response>();
    const failReqB = deferred<Response>();

    const fetchFn = vi.fn((input: RequestInfo | URL) => {
      const path = String(input).split('?')[0];
      if (path === '/api/v3/views/staff_me') {
        return Promise.resolve(jsonResponse(okBody(staffMe())));
      }
      if (path === '/api/v3/views/staff_floor') {
        floorCall += 1;
        if (floorCall === 1) {
          // A answers OK
          return Promise.resolve(jsonResponse(okBody(FLOOR_A)));
        }
        if (floorCall === 2) {
          return staleReq.promise;
        }
        return failReqB.promise;
      }
      throw new Error(`unexpected request ${String(input)}`);
    });

    const navigate = vi.fn();
    const api = new ApiClient(fetchFn as unknown as typeof fetch);
    dispose = render(() => <StaffPage api={api} cards={[]} navigate={navigate} />, host);

    // A answered OK -> shows A's hours (08:00)
    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('08:00');
    });

    const refreshBtn = host.querySelector('[data-testid="staff-refresh"]') as HTMLButtonElement;
    // Start an intermediate refresh, then B which fails
    refreshBtn.click();
    await flush();
    refreshBtn.click();
    await flush();

    failReqB.resolve(
      jsonResponse({ error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }, 500),
    );
    await vi.waitFor(() => {
      const alert = host.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    });
    expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('08:00');

    // Earlier stale request resolves late -> must not clear B's error or overwrite A's floor
    staleReq.resolve(jsonResponse(okBody(FLOOR_B)));
    await flush();

    const alert = host.querySelector('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('08:00');
  });

  it("row 4: A card's ctx.refresh -> Reads both views again", async () => {
    let meCall = 0;
    let floorCall = 0;
    const floorReqA = deferred<Response>();
    const floorReqB = deferred<Response>();
    let capturedCtx: StaffCtx | undefined;

    const testCard: StaffCard = {
      id: 'shift',
      order: 10,
      visible: () => true,
      Component: (p): JSX.Element => {
        capturedCtx = p.ctx;
        return (
          <div data-testid="card-phase">
            {p.ctx.floor?.hours?.open ?? 'none'}
          </div>
        );
      },
    };

    const fetchFn = vi.fn((input: RequestInfo | URL) => {
      const path = String(input).split('?')[0];
      if (path === '/api/v3/views/staff_me') {
        meCall += 1;
        return Promise.resolve(jsonResponse(okBody(staffMe())));
      }
      if (path === '/api/v3/views/staff_floor') {
        floorCall += 1;
        if (floorCall === 1) {
          return Promise.resolve(jsonResponse(okBody(FLOOR_INITIAL)));
        }
        if (floorCall === 2) {
          return floorReqA.promise;
        }
        return floorReqB.promise;
      }
      throw new Error(`unexpected request ${String(input)}`);
    });

    const navigate = vi.fn();
    const api = new ApiClient(fetchFn as unknown as typeof fetch);
    dispose = render(() => <StaffPage api={api} cards={[testCard]} navigate={navigate} />, host);

    await vi.waitFor(() => {
      expect(capturedCtx).toBeDefined();
      expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('10:00');
    });
    expect(meCall).toBe(1);
    expect(floorCall).toBe(1);

    // Card invokes ctx.refresh() twice overlapping
    const pA = capturedCtx!.refresh();
    await flush();
    const pB = capturedCtx!.refresh();
    await flush();

    // Both views were read again for each ctx.refresh() call
    expect(meCall).toBe(3);
    expect(floorCall).toBe(3);

    floorReqB.resolve(jsonResponse(okBody(FLOOR_B)));
    await pB;
    await flush();
    expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('11:00');

    floorReqA.resolve(jsonResponse(okBody(FLOOR_A)));
    await pA;
    await flush();
    expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('11:00');
  });

  it('row 5: A failed refresh after a success -> Stale data stays + error shown', async () => {
    let meCall = 0;
    let floorCall = 0;
    const staleMeReq = deferred<Response>();

    const fetchFn = vi.fn((input: RequestInfo | URL) => {
      const path = String(input).split('?')[0];
      if (path === '/api/v3/views/staff_me') {
        meCall += 1;
        if (meCall === 2) {
          return staleMeReq.promise;
        }
        return Promise.resolve(jsonResponse(okBody(staffMe())));
      }
      if (path === '/api/v3/views/staff_floor') {
        floorCall += 1;
        if (floorCall === 1) {
          return Promise.resolve(jsonResponse(okBody(FLOOR_INITIAL)));
        }
        return Promise.resolve(
          jsonResponse({ error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }, 500),
        );
      }
      throw new Error(`unexpected request ${String(input)}`);
    });

    const navigate = vi.fn();
    const api = new ApiClient(fetchFn as unknown as typeof fetch);
    dispose = render(() => <StaffPage api={api} cards={[]} navigate={navigate} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('10:00');
    });

    const refreshBtn = host.querySelector('[data-testid="staff-refresh"]') as HTMLButtonElement;
    // Start refresh #2 (held at staff_me), then refresh #3 (fails at staff_floor)
    refreshBtn.click();
    await flush();
    refreshBtn.click();
    await flush();

    await vi.waitFor(() => {
      const alert = host.querySelector('[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    });
    expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('10:00');

    // Now stale refresh #2's staff_me resolves -> must not proceed to staff_floor or overwrite state
    staleMeReq.resolve(jsonResponse(okBody(staffMe())));
    await flush();

    expect(floorCall).toBe(2);
    expect(host.querySelector('[role="alert"]')).not.toBeNull();
    expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('10:00');
  });

  it('edge case (§9): A stale call gets 401 on staff_me while a newer call succeeded: must not navigate to /login', async () => {
    let meCall = 0;
    const staleMeReq = deferred<Response>();

    const fetchFn = vi.fn((input: RequestInfo | URL) => {
      const path = String(input).split('?')[0];
      if (path === '/api/v3/views/staff_me') {
        meCall += 1;
        if (meCall === 2) {
          return staleMeReq.promise;
        }
        return Promise.resolve(jsonResponse(okBody(staffMe())));
      }
      if (path === '/api/v3/views/staff_floor') {
        return Promise.resolve(jsonResponse(okBody(FLOOR_B)));
      }
      throw new Error(`unexpected request ${String(input)}`);
    });

    const navigate = vi.fn();
    const api = new ApiClient(fetchFn as unknown as typeof fetch);
    dispose = render(() => <StaffPage api={api} cards={[]} navigate={navigate} />, host);

    // Mount read (call 1) succeeds
    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('11:00');
    });

    const refreshBtn = host.querySelector('[data-testid="staff-refresh"]') as HTMLButtonElement;
    // Start Refresh A (call 2, held on staff_me), then Refresh B (call 3, succeeds)
    refreshBtn.click();
    await flush();
    refreshBtn.click();
    await flush();

    // Stale Refresh A answers 401 on staff_me -> must not navigate to /login
    staleMeReq.resolve(
      jsonResponse({ error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } }, 401),
    );
    await flush();

    expect(navigate).not.toHaveBeenCalled();
    expect(host.querySelector('[data-testid="staff-hours"]')?.textContent).toContain('11:00');
  });
});
