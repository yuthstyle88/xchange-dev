import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { requestRefresh } from '../parts/refresh';
import { th as slipsTh } from './i18n';
import { Slips } from './Slips';

const SAMPLE_SLIPS: TransactionRow[] = [
  {
    id: 'XC-20261008-000001',
    date: '2026-10-08',
    created: '2026-10-08T03:04:05.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '32.1000',
    unitSize: '1',
    principal: '3210.00',
    fee: '0.00',
    bankFee: '0.00',
    settlement: '3210.00',
    method: 'Bank Transfer',
    customer: 'Customer 1',
    customerId: 'CX-00000001',
    staff: 'front1',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: null,
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: null,
    slipUrl: null,
  },
];

const SAMPLE_BRANCHES = [
  { id: 'MAIN', name: 'Main Desk' },
  { id: 'B2', name: 'Branch 2' },
];

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('Admin Slips: branch-list and slips read recovery on refresh (#2180)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let branchesHandler: () => Promise<Response>;
  let slipsHandler: () => Promise<Response>;
  let branchCalls: number;

  beforeEach(() => {
    setLang('th');
    branchCalls = 0;
    root = document.createElement('div');
    document.body.appendChild(root);

    branchesHandler = async () =>
      json({ version: 1, changed: true, data: { branches: SAMPLE_BRANCHES } });
    slipsHandler = async () =>
      json({ version: 1, changed: true, data: SAMPLE_SLIPS });

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url =
          typeof input === 'string'
            ? input
            : input instanceof URL
              ? input.href
              : input.url;
        if (PAGE_WIDE.some((p) => url.includes(p))) {
          return json({ version: 1, changed: false });
        }
        if (url.includes('/api/v3/views/admin_branches')) {
          branchCalls += 1;
          return branchesHandler();
        }
        if (url.includes('/api/v3/views/admin_slips')) {
          return slipsHandler();
        }
        return json({ error: { code: 'notFound', message: 'not found' } }, 404);
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(initialBranch = 'MAIN') {
    const [branch, setBranch] = createSignal(initialBranch);
    const [shown, setShown] = createSignal(true);
    const period = () => ({ from: '', to: '' });
    dispose = render(
      () => <Slips branch={branch} period={period} shown={shown} />,
      root,
    );
    return { branch, setBranch, setShown };
  }

  function alertEl(): HTMLElement | null {
    return root.querySelector('[role="alert"]');
  }

  function alertText(): string {
    return (alertEl()?.textContent ?? '').trim();
  }

  function statusText(): string {
    return (root.querySelector('[role="status"]')?.textContent ?? '').trim();
  }

  it('row 1: Branches fail, then Refresh with branches OK -> Alert gone, summary names the branch, pagination shown', async () => {
    branchesHandler = async () =>
      json({ error: { code: 'roleNotAllowedHere', message: 'raw branch error' } }, 403);
    mount('MAIN');

    await vi.waitFor(() => {
      expect(alertText()).toContain(slipsTh.notLoaded);
      expect(alertText()).toContain(sharedTh['error.roleNotAllowedHere']);
    });
    expect(statusText()).toBe(slipsTh.waiting);
    expect(root.querySelectorAll('.pagination button').length).toBe(0);

    branchesHandler = async () =>
      json({ version: 1, changed: true, data: { branches: SAMPLE_BRANCHES } });
    requestRefresh();

    await vi.waitFor(() => {
      expect(alertEl()).toBeNull();
    });
    expect(statusText()).toBe('สาขา Main Desk · พบ 1 สลิป ตามช่วงวันที่และตัวกรอง');
    expect(root.querySelectorAll('.pagination button').length).toBe(2);
    expect(root.querySelector('.page-indicator')?.textContent?.trim()).toBe('หน้า 1');
  });

  it('row 2: Branches fail -> Alert with the wordedError text (kept)', async () => {
    branchesHandler = async () =>
      json({ error: { code: 'roleNotAllowedHere', message: 'raw refusal' } }, 403);
    mount('MAIN');

    await vi.waitFor(() => {
      expect(alertText()).toContain(slipsTh.notLoaded);
      expect(alertText()).toContain(sharedTh['error.roleNotAllowedHere']);
    });
    expect(alertText()).not.toContain('raw refusal');
    expect(alertText()).not.toContain('roleNotAllowedHere');

    // If slips also fails once and then recovers while branches still fail, the branch wordedError is kept
    slipsHandler = async () =>
      json({ error: { code: 'serverError', message: 'slips error' } }, 500);
    requestRefresh();
    await vi.waitFor(() => {
      expect(alertText()).toContain(sharedTh['error.serverError']);
    });

    slipsHandler = async () =>
      json({ version: 1, changed: true, data: SAMPLE_SLIPS });
    requestRefresh();
    await vi.waitFor(() => {
      expect(alertText()).toContain(sharedTh['error.roleNotAllowedHere']);
    });
    expect(alertText()).not.toContain(sharedTh['error.serverError']);
  });

  it('row 3: Slips read fails -> notLoaded + error and the waiting summary (kept)', async () => {
    slipsHandler = async () =>
      json({ error: { code: 'roleNotAllowedHere', message: 'raw slips refusal' } }, 403);
    mount('MAIN');

    await vi.waitFor(() => {
      expect(alertText()).toContain(slipsTh.notLoaded);
      expect(alertText()).toContain(sharedTh['error.roleNotAllowedHere']);
    });
    expect(statusText()).toBe(slipsTh.waiting);
    expect(root.querySelectorAll('.pagination button').length).toBe(0);
  });

  it('row 4: Slips fail, then Refresh OK (branches fine) -> Alert gone (kept)', async () => {
    slipsHandler = async () =>
      json({ error: { code: 'serverError', message: 'server error' } }, 500);
    mount('MAIN');

    await vi.waitFor(() => {
      expect(alertText()).toContain(slipsTh.notLoaded);
      expect(alertText()).toContain(sharedTh['error.serverError']);
    });
    expect(statusText()).toBe(slipsTh.waiting);

    slipsHandler = async () =>
      json({ version: 1, changed: true, data: SAMPLE_SLIPS });
    requestRefresh();

    await vi.waitFor(() => {
      expect(alertEl()).toBeNull();
    });
    expect(statusText()).toBe('สาขา Main Desk · พบ 1 สลิป ตามช่วงวันที่และตัวกรอง');
    expect(root.querySelectorAll('.pagination button').length).toBe(2);
  });

  it('edge case: Branches recover but slips fail on the same refresh -> the alert shows the slips error', async () => {
    branchesHandler = async () =>
      json({ error: { code: 'roleNotAllowedHere', message: 'branch failed' } }, 403);
    mount('MAIN');

    await vi.waitFor(() => {
      expect(alertText()).toContain(sharedTh['error.roleNotAllowedHere']);
    });
    expect(branchCalls).toBe(1);

    slipsHandler = async () =>
      json({ error: { code: 'serverError', message: 'slips failed' } }, 500);
    branchesHandler = async () => {
      await new Promise((resolve) => setTimeout(resolve, 10));
      return json({ version: 1, changed: true, data: { branches: SAMPLE_BRANCHES } });
    };

    requestRefresh();

    await vi.waitFor(() => {
      expect(branchCalls).toBe(2);
      expect(alertText()).toContain(sharedTh['error.serverError']);
    });
    expect(alertText()).toContain(slipsTh.notLoaded);
    expect(alertText()).not.toContain(sharedTh['error.roleNotAllowedHere']);
    expect(statusText()).toBe(slipsTh.waiting);
    expect(root.querySelectorAll('.pagination button').length).toBe(0);
  });
});
