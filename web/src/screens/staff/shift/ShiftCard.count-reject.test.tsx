// #5312 (SC-FE2): /staff verifier rejects a waiting count with a reason; list filters on status
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from './i18n';
import { ShiftCard } from './ShiftCard';
import type { StaffCtx } from '../cards';
import {
  makeCashCountRow,
  makeCountRejected,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';

const BRANCH = 'B1';
const ME = '00000000-0000-4000-8000-0000000000a1';
const COUNTER = '37ba9f73-54a0-4b94-aac2-7b32f08ff33e';

function mockMe(permissions = ['ShiftCloseCertify']) {
  return makeStaffMeView({
    userId: ME,
    name: 'Manager Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B1',
    permissions,
  });
}

function mockFloor() {
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

function typeInto(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('#5312 ShiftCard count reject and status filter (§8, §9)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let currentCounts: ReturnType<typeof makeCashCountRow>[];
  let posts: { url: string; body: unknown }[];
  let cashCountReads: number;
  let postHandler: ((url: string, body: unknown) => Response) | null;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    currentCounts = [];
    posts = [];
    cashCountReads = 0;
    postHandler = null;

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
          if (path === '/api/v3/views/cash_counts') {
            cashCountReads += 1;
            return json(200, { version: 1, changed: true, data: currentCounts });
          }
          return json(200, { version: 1, changed: true, data: [] });
        }

        const parsedBody = init?.body ? JSON.parse(String(init.body)) : {};
        posts.push({ url: path, body: parsedBody });
        if (postHandler) {
          return postHandler(path, parsedBody);
        }
        return json(200, makeCountRejected({ id: path.split('/')[3] ?? 'c-1', status: 'REJECTED' }));
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    setLang('th');
    vi.unstubAllGlobals();
  });

  function mountCard(counts: ReturnType<typeof makeCashCountRow>[]) {
    currentCounts = [...counts];
    const ctx: StaffCtx = { me: mockMe(), floor: mockFloor(), refresh: vi.fn(async () => {}) };
    dispose = render(() => <ShiftCard ctx={ctx} />, host);
    return ctx;
  }

  it('Row 1: a count with status: REJECTED is not in the waiting list', async () => {
    const rejected = makeCashCountRow({
      id: 'c-rejected-1',
      branchId: BRANCH,
      kind: 'OPEN',
      status: 'REJECTED',
      countedBy: COUNTER,
      verifiedBy: null,
      rejectedBy: ME,
      rejectedAt: '2026-10-27T20:20:00Z',
      rejectReason: 'ยอดไม่ตรง',
    });

    mountCard([rejected]);
    await settle();

    expect(host.querySelector('[data-count-id="c-rejected-1"]')).toBeNull();
  });

  it('Row 2: a WAITING count, me with ShiftCloseCertify shows verify and reject buttons', async () => {
    const waiting = makeCashCountRow({
      id: 'c-wait-1',
      branchId: BRANCH,
      kind: 'OPEN',
      status: 'WAITING',
      countedBy: COUNTER,
      verifiedBy: null,
    });

    mountCard([waiting]);
    await settle();

    const row = host.querySelector('[data-count-id="c-wait-1"]');
    expect(row).not.toBeNull();
    const verifyBtn = row!.querySelector<HTMLButtonElement>('[data-testid="shift-verify"]');
    const rejectBtn = row!.querySelector<HTMLButtonElement>('[data-testid="shift-reject"]');
    expect(verifyBtn).not.toBeNull();
    expect(rejectBtn).not.toBeNull();
    expect(rejectBtn!.textContent).toContain(th.reject);
  });

  it('Row 3: reject with a reason -> 200 sends one POST /api/cash-counts/{id}/reject with {reason}; the row is gone after reload', async () => {
    const waiting = makeCashCountRow({
      id: 'c-wait-2',
      branchId: BRANCH,
      kind: 'OPEN',
      status: 'WAITING',
      countedBy: COUNTER,
      verifiedBy: null,
    });

    postHandler = (path, body) => {
      if (path === '/api/cash-counts/c-wait-2/reject') {
        currentCounts = [];
        return json(
          200,
          makeCountRejected({
            id: 'c-wait-2',
            status: 'REJECTED',
            rejectedBy: ME,
            reason: (body as { reason: string }).reason,
          }),
        );
      }
      return json(404, {});
    };

    mountCard([waiting]);
    await settle();

    const row = host.querySelector('[data-count-id="c-wait-2"]')!;
    row.querySelector<HTMLButtonElement>('[data-testid="shift-reject"]')!.click();
    await settle();

    const input = row.querySelector<HTMLInputElement>('[data-testid="shift-reject-reason"]');
    expect(input).not.toBeNull();
    typeInto(input!, 'เงินในเซฟไม่พอ');

    const confirmBtn = row.querySelector<HTMLButtonElement>('[data-testid="shift-reject-confirm"]');
    expect(confirmBtn).not.toBeNull();
    confirmBtn!.click();
    await settle();

    expect(posts).toEqual([
      { url: '/api/cash-counts/c-wait-2/reject', body: { reason: 'เงินในเซฟไม่พอ' } },
    ]);
    expect(host.querySelector('[data-count-id="c-wait-2"]')).toBeNull();
  });

  it('Row 4: reject with a blank reason shows reasonRequired under the input and sends no request', async () => {
    const waiting = makeCashCountRow({
      id: 'c-wait-3',
      branchId: BRANCH,
      kind: 'CLOSE',
      status: 'WAITING',
      countedBy: COUNTER,
      verifiedBy: null,
    });

    mountCard([waiting]);
    await settle();

    const row = host.querySelector('[data-count-id="c-wait-3"]')!;
    row.querySelector<HTMLButtonElement>('[data-testid="shift-reject"]')!.click();
    await settle();

    const input = row.querySelector<HTMLInputElement>('[data-testid="shift-reject-reason"]')!;
    typeInto(input, '   ');

    row.querySelector<HTMLButtonElement>('[data-testid="shift-reject-confirm"]')!.click();
    await settle();

    expect(posts).toEqual([]);
    const fieldErr = row.querySelector('[data-field-error="reason"]');
    expect(fieldErr).not.toBeNull();
    expect(fieldErr!.textContent).toBe(th.reasonRequired);
  });

  it('Row 5: server 409 notWaiting shows its text on the row line', async () => {
    const waiting = makeCashCountRow({
      id: 'c-wait-4',
      branchId: BRANCH,
      kind: 'OPEN',
      status: 'WAITING',
      countedBy: COUNTER,
      verifiedBy: null,
    });

    postHandler = () =>
      json(409, {
        error: { code: 'notWaiting', message: 'not waiting' },
      });

    mountCard([waiting]);
    await settle();

    const row = host.querySelector('[data-count-id="c-wait-4"]')!;
    row.querySelector<HTMLButtonElement>('[data-testid="shift-reject"]')!.click();
    await settle();

    const input = row.querySelector<HTMLInputElement>('[data-testid="shift-reject-reason"]')!;
    typeInto(input, 'นับผิด');
    row.querySelector<HTMLButtonElement>('[data-testid="shift-reject-confirm"]')!.click();
    await settle();

    const rowAfter = host.querySelector('[data-count-id="c-wait-4"]')!;
    expect(rowAfter.textContent).toContain(th.notWaiting);
  });

  it('Row 6: cancel the reason input sends no request and brings buttons back', async () => {
    const waiting = makeCashCountRow({
      id: 'c-wait-5',
      branchId: BRANCH,
      kind: 'OPEN',
      status: 'WAITING',
      countedBy: COUNTER,
      verifiedBy: null,
    });

    mountCard([waiting]);
    await settle();

    const row = host.querySelector('[data-count-id="c-wait-5"]')!;
    row.querySelector<HTMLButtonElement>('[data-testid="shift-reject"]')!.click();
    await settle();

    expect(row.querySelector('[data-testid="shift-verify"]')).toBeNull();
    expect(row.querySelector('[data-testid="shift-reject"]')).toBeNull();

    const input = row.querySelector<HTMLInputElement>('[data-testid="shift-reject-reason"]')!;
    typeInto(input, 'เปลี่ยนใจ');

    row.querySelector<HTMLButtonElement>('[data-testid="shift-reject-cancel"]')!.click();
    await settle();

    expect(posts).toEqual([]);
    expect(row.querySelector('[data-testid="shift-reject-reason"]')).toBeNull();
    expect(row.querySelector('[data-testid="shift-verify"]')).not.toBeNull();
    expect(row.querySelector('[data-testid="shift-reject"]')).not.toBeNull();
  });

  it('Edge case (§9): two managers — one rejects while the other list is stale -> 409 notWaiting -> reload', async () => {
    const waiting = makeCashCountRow({
      id: 'c-wait-6',
      branchId: BRANCH,
      kind: 'OPEN',
      status: 'WAITING',
      countedBy: COUNTER,
      verifiedBy: null,
    });

    postHandler = () => {
      currentCounts = [];
      return json(409, {
        error: { code: 'notWaiting', message: 'already rejected' },
      });
    };

    mountCard([waiting]);
    await settle();
    const readsBefore = cashCountReads;

    const row = host.querySelector('[data-count-id="c-wait-6"]')!;
    row.querySelector<HTMLButtonElement>('[data-testid="shift-reject"]')!.click();
    await settle();

    const input = row.querySelector<HTMLInputElement>('[data-testid="shift-reject-reason"]')!;
    typeInto(input, 'ซ้ำ');
    row.querySelector<HTMLButtonElement>('[data-testid="shift-reject-confirm"]')!.click();
    await settle();

    expect(cashCountReads).toBeGreaterThan(readsBefore);
    expect(host.querySelector('[data-count-id="c-wait-6"]')).toBeNull();
  });

  it('Edge case (§9): verify still works on a WAITING count exactly as before', async () => {
    const waiting = makeCashCountRow({
      id: 'c-wait-7',
      branchId: BRANCH,
      kind: 'OPEN',
      status: 'WAITING',
      countedBy: COUNTER,
      verifiedBy: null,
    });

    postHandler = (path) => {
      if (path === '/api/cash-counts/c-wait-7/verify') {
        currentCounts = [];
        return json(200, { id: 'c-wait-7', status: 'VERIFIED', approvalRequestId: null });
      }
      return json(404, {});
    };

    const ctx = mountCard([waiting]);
    await settle();

    const row = host.querySelector('[data-count-id="c-wait-7"]')!;
    row.querySelector<HTMLButtonElement>('[data-testid="shift-verify"]')!.click();
    await settle();

    expect(posts).toEqual([
      { url: '/api/cash-counts/c-wait-7/verify', body: { id: 'c-wait-7', branch: BRANCH } },
    ]);
    expect(ctx.refresh).toHaveBeenCalledTimes(1);
    expect(host.querySelector('[data-testid="shift-notice"]')?.textContent).toBe(th.saved);
  });
});
