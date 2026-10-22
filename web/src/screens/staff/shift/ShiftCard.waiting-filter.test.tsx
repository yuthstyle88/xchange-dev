// #4745 (R3-8): ShiftCard reads cash_counts with branch and status=WAITING filter
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ShiftCard } from './ShiftCard';
import type { StaffCtx } from '../cards';
import { makeStaffFloorView, makeStaffMeView } from '../../../shared/testing/fixtures';

const ME = '00000000-0000-4000-8000-0000000000a1';

function me(permissions: string[], branchId = 'B1') {
  return makeStaffMeView({
    userId: ME,
    name: 'Ploy',
    roles: [],
    branchId,
    branchName: 'XC-B1',
    permissions,
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

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('#4745 ShiftCard cash_counts waiting filter (R3-8)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let requestedUrls: string[];

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    requestedUrls = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url =
          typeof input === 'string'
            ? input
            : input instanceof URL
              ? input.href
              : input.url;
        requestedUrls.push(url);
        return json(200, { version: 1, changed: true, data: [] });
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
  });

  it('W1: a ShiftCloseCertify holder at branch B1 sends branch=B1 and status=WAITING on cash_counts', async () => {
    const ctx: StaffCtx = { me: me(['ShiftCloseCertify'], 'B1'), floor: floor(), refresh: vi.fn(async () => {}) };
    dispose = render(() => <ShiftCard ctx={ctx} />, host);
    await settle();

    const cashCountsUrl = requestedUrls.find((u) => u.startsWith('/api/v3/views/cash_counts'));
    expect(cashCountsUrl, 'cash_counts request sent').toBeDefined();
    const parsed = new URL(cashCountsUrl!, 'http://localhost');
    expect(parsed.pathname).toBe('/api/v3/views/cash_counts');
    expect(parsed.searchParams.get('branch')).toBe('B1');
    expect(parsed.searchParams.get('status')).toBe('WAITING');
  });

  it('W2: without ShiftCloseCertify, no cash_counts request is sent', async () => {
    const ctx: StaffCtx = { me: me(['ShiftOperate'], 'B1'), floor: floor(), refresh: vi.fn(async () => {}) };
    dispose = render(() => <ShiftCard ctx={ctx} />, host);
    await settle();

    const cashCountsUrls = requestedUrls.filter((u) => u.startsWith('/api/v3/views/cash_counts'));
    expect(cashCountsUrls).toEqual([]);
  });
});
