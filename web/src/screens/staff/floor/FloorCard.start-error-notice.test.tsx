import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { StaffFloorView } from '../../../shared/api/generated/staff_floor';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import type { StaffCtx } from '../cards';
import { FloorCard } from './FloorCard';
import { th } from './i18n';

const BRANCH = 'MAIN';
const PUBLISHED_AT = '2026-10-11T02:30:00Z';
const ALL_PERMS = ['AttendanceClock', 'BranchDayOperate', 'CashTargetSet'];

function me(permissions: string[] = ALL_PERMS): StaffMeView {
  return {
    userId: '00000000-0000-4000-8000-0000000000f1',
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  };
}

function floor(over: Partial<StaffFloorView> = {}): StaffFloorView {
  return {
    phase: 'READY',
    hours: null,
    myClockIn: null,
    myShift: null,
    cutoff: null,
    target: { amounts: { USD: '1000.00' }, at: '2026-10-11T01:00:00Z' },
    safe: null,
    ...over,
  };
}

const RATES: PublishedRatesData = {
  publication: {
    id: '11111111-2222-4333-8444-555555555555',
    version: 7,
    publishedAt: PUBLISHED_AT,
  },
  sourceDate: '2026-10-10',
  stale: false,
  lines: [
    {
      currency: 'USD',
      group: 'ALL',
      unitSize: '1',
      buyRate: '32.4500',
      sellRate: '32.9000',
      lineId: 'l-usd',
    },
  ],
};

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('FloorCard: Start service error clears earlier notice (#2721)', () => {
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
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  it('F1: An action succeeds ("Saved" shown), then Start service and staff_published_rates fails (500) -> floor-notice is gone, floor-error shows', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'POST' && path === '/api/attendance') {
          return jsonResponse(200, {});
        }
        if (method === 'GET' && path === '/api/v3/views/staff_published_rates') {
          return jsonResponse(500, { error: { code: 'serverError', message: 'rates failed' } });
        }
        return jsonResponse(404, {});
      }),
    );

    const ctx: StaffCtx = {
      me: me(ALL_PERMS),
      floor: floor({ phase: 'READY', myClockIn: null }),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <FloorCard ctx={ctx} />, host);
    await flush();

    // 1. Clock in succeeds -> "Saved" notice is displayed
    const clockBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]')!;
    clockBtn.click();
    await flush();

    expect(host.querySelector('[data-testid="floor-notice"]')?.textContent).toBe(th.saved);
    expect(host.querySelector('[data-testid="floor-error"]')).toBeNull();

    // 2. Start service confirmed -> rates fetch returns 500
    const startBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
    startBtn.click();
    await flush();

    const confirmBtn = host.querySelectorAll<HTMLButtonElement>('dialog button')[1]!;
    confirmBtn.click();
    await flush();

    // floor-notice must be gone, floor-error must show worded error
    expect(host.querySelector('[data-testid="floor-notice"]')).toBeNull();
    expect(host.querySelector('[data-testid="floor-error"]')?.textContent).toBe(
      errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'),
    );
  });

  it('F2: An action succeeds ("Saved" shown), then Start service and rates answer has no publication.publishedAt -> floor-notice is gone, floor-error shows', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'POST' && path === '/api/attendance') {
          return jsonResponse(200, {});
        }
        if (method === 'GET' && path === '/api/v3/views/staff_published_rates') {
          return jsonResponse(200, {
            version: 1,
            changed: true,
            data: {
              ...RATES,
              publication: null,
            },
          });
        }
        return jsonResponse(404, {});
      }),
    );

    const ctx: StaffCtx = {
      me: me(ALL_PERMS),
      floor: floor({ phase: 'READY', myClockIn: null }),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <FloorCard ctx={ctx} />, host);
    await flush();

    // 1. Clock in succeeds -> "Saved" notice is displayed
    const clockBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]')!;
    clockBtn.click();
    await flush();

    expect(host.querySelector('[data-testid="floor-notice"]')?.textContent).toBe(th.saved);
    expect(host.querySelector('[data-testid="floor-error"]')).toBeNull();

    // 2. Start service confirmed -> rates answer has no publishedAt
    const startBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
    startBtn.click();
    await flush();

    const confirmBtn = host.querySelectorAll<HTMLButtonElement>('dialog button')[1]!;
    confirmBtn.click();
    await flush();

    // floor-notice must be gone, floor-error must show
    expect(host.querySelector('[data-testid="floor-notice"]')).toBeNull();
    expect(host.querySelector('[data-testid="floor-error"]')?.textContent).toBe(
      errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'),
    );
  });

  it('F3: Start service succeeds -> floor-notice shows "Saved"; no floor-error (kept)', async () => {
    const posts: string[] = [];

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'GET' && path === '/api/v3/views/staff_published_rates') {
          return jsonResponse(200, { version: 1, changed: true, data: RATES });
        }
        if (method === 'POST' && path === '/api/branch-day/start') {
          posts.push(path);
          return jsonResponse(200, {});
        }
        return jsonResponse(404, {});
      }),
    );

    const ctx: StaffCtx = {
      me: me(ALL_PERMS),
      floor: floor({ phase: 'READY' }),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <FloorCard ctx={ctx} />, host);
    await flush();

    const startBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
    startBtn.click();
    await flush();

    const confirmBtn = host.querySelectorAll<HTMLButtonElement>('dialog button')[1]!;
    confirmBtn.click();
    await flush();

    expect(posts).toEqual(['/api/branch-day/start']);
    expect(host.querySelector('[data-testid="floor-notice"]')?.textContent).toBe(th.saved);
    expect(host.querySelector('[data-testid="floor-error"]')).toBeNull();
  });

  it('F4: Start service, and the start command is refused -> floor-notice is gone; floor-error shows (kept)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'POST' && path === '/api/attendance') {
          return jsonResponse(200, {});
        }
        if (method === 'GET' && path === '/api/v3/views/staff_published_rates') {
          return jsonResponse(200, { version: 1, changed: true, data: RATES });
        }
        if (method === 'POST' && path === '/api/branch-day/start') {
          return jsonResponse(400, { error: { code: 'badRequest', message: 'refused' } });
        }
        return jsonResponse(404, {});
      }),
    );

    const ctx: StaffCtx = {
      me: me(ALL_PERMS),
      floor: floor({ phase: 'READY', myClockIn: null }),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <FloorCard ctx={ctx} />, host);
    await flush();

    // 1. Clock in succeeds -> "Saved" notice is displayed
    const clockBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]')!;
    clockBtn.click();
    await flush();

    expect(host.querySelector('[data-testid="floor-notice"]')?.textContent).toBe(th.saved);
    expect(host.querySelector('[data-testid="floor-error"]')).toBeNull();

    // 2. Start service confirmed -> command refused (400)
    const startBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
    startBtn.click();
    await flush();

    const confirmBtn = host.querySelectorAll<HTMLButtonElement>('dialog button')[1]!;
    confirmBtn.click();
    await flush();

    expect(host.querySelector('[data-testid="floor-notice"]')).toBeNull();
    expect(host.querySelector('[data-testid="floor-error"]')?.textContent).toBe('refused');
  });

  it('F5: Start service while another action is in flight -> Nothing sent (kept)', async () => {
    const postCalls: string[] = [];
    let releaseClock: (() => void) | undefined;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = rawUrl.split('?')[0]!;
        const method = (init?.method ?? 'GET').toUpperCase();

        if (method === 'POST' && path === '/api/attendance') {
          postCalls.push(path);
          await new Promise<void>((r) => {
            releaseClock = r;
          });
          return jsonResponse(200, {});
        }
        if (method === 'GET' && path === '/api/v3/views/staff_published_rates') {
          return jsonResponse(200, { version: 1, changed: true, data: RATES });
        }
        if (method === 'POST' && path === '/api/branch-day/start') {
          postCalls.push(path);
          return jsonResponse(200, {});
        }
        return jsonResponse(404, {});
      }),
    );

    const ctx: StaffCtx = {
      me: me(ALL_PERMS),
      floor: floor({ phase: 'READY', myClockIn: null }),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <FloorCard ctx={ctx} />, host);
    await flush();

    // 1. Clock in starts and stays in-flight
    const clockBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-clock"]')!;
    clockBtn.click();
    await flush();

    expect(postCalls).toEqual(['/api/attendance']);

    // 2. Start service button is disabled while busy
    const startBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
    expect(startBtn.disabled).toBe(true);

    startBtn.click();
    await flush();

    // Confirm dialog is not opened
    expect(host.querySelector('dialog')).toBeNull();
    // No start post dispatched
    expect(postCalls).toEqual(['/api/attendance']);

    // Release clock in
    releaseClock!();
    await flush();

    expect(startBtn.disabled).toBe(false);
  });
});
