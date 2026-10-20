import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import {
  makeStaffFloorView,
  makeStaffMeView,
  type StaffFloorView,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { FloorCard } from './FloorCard';

const BRANCH = 'MAIN';

function me(permissions: string[]) {
  return makeStaffMeView({
    userId: '00000000-0000-4000-8000-0000000000f1',
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
    myClockIn: null,
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
    ...over,
  });
}

function makeCtx(over: Partial<StaffFloorView> = {}, perms = ['BranchDayOperate']): StaffCtx {
  return {
    me: me(perms),
    floor: floor(over),
    refresh: vi.fn(async () => {}),
  } as unknown as StaffCtx;
}

describe('FloorCard rework tests (#1690: worded errors on rate read failures)', () => {
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

  it('row 1 (FloorCard.tsx:122): failed loadRates shows worded error for 500 network failure', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
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
    dispose = render(() => <FloorCard ctx={ctx} />, host);

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="floor-error"]');
      expect(errEl, 'floor-error must be rendered when loadRates fails').not.toBeNull();
      expect(errEl?.textContent).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    });
  });

  it('row 1 (FloorCard.tsx:122): failed loadRates shows worded error for known refusal code', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
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
    dispose = render(() => <FloorCard ctx={ctx} />, host);

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="floor-error"]');
      expect(errEl, 'floor-error must be rendered when loadRates is refused').not.toBeNull();
      expect(errEl?.textContent).toBe(errorText('roleNotAllowed', ''));
    });
  });

  it('row 1 (FloorCard.tsx:122): failed loadRates shows generic worded message for unknown refusal code (never raw code or message)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (url.includes('/api/v3/views/staff_published_rates')) {
          return new Response(
            JSON.stringify({
              error: {
                code: 'unknownRateCode',
                message: 'raw technical backend failure',
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
    dispose = render(() => <FloorCard ctx={ctx} />, host);

    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="floor-error"]');
      expect(errEl, 'floor-error must be rendered on unknown refusal code').not.toBeNull();
      expect(errEl?.textContent).toBe('raw technical backend failure');
      expect(errEl?.textContent).not.toContain('unknownRateCode');
    });
  });

  it('row 2 (FloorCard.tsx:169): refresh failure in handleStartService aborts start and shows worded error', async () => {
    const sentCalls: { url: string; method: string }[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const method = (init?.method ?? 'GET').toUpperCase();
        sentCalls.push({ url, method });

        if (url.includes('/api/v3/views/staff_published_rates')) {
          return new Response('Internal Server Error', {
            status: 500,
            statusText: 'Internal Server Error',
            headers: { 'content-type': 'text/plain' },
          });
        }
        if (url.includes('/api/branch-day/start')) {
          return new Response(JSON.stringify({ ok: true }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          });
        }
        return new Response('Not found', { status: 404 });
      }),
    );

    const ctx = makeCtx({ phase: 'READY' });
    dispose = render(() => <FloorCard ctx={ctx} />, host);

    // Initial loadRates fails on 500
    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="floor-error"]');
      expect(errEl).not.toBeNull();
    });

    // Click Start Service
    const startBtn = host.querySelector<HTMLButtonElement>('[data-testid="floor-start"]')!;
    expect(startBtn).not.toBeNull();
    startBtn.click();

    // Confirm dialog
    const dialog = host.querySelector('dialog');
    expect(dialog).not.toBeNull();
    const buttons = dialog!.querySelectorAll('button');
    const confirmBtn = buttons[1]!;
    expect(confirmBtn).toBeDefined();
    confirmBtn.click();

    // Must show worded error and abort start (never call /api/branch-day/start)
    await vi.waitFor(() => {
      const errEl = host.querySelector('[data-testid="floor-error"]');
      expect(errEl?.textContent).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    });

    const startCmdCalls = sentCalls.filter((c) => c.url.includes('/api/branch-day/start'));
    expect(startCmdCalls).toHaveLength(0);
  });
});
