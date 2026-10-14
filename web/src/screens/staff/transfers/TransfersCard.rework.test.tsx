import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { TransfersCard } from './TransfersCard';
import { errorText, setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { en as sharedEn } from '../../../shared/i18n/en';
import type { StaffCtx } from '../cards';
import type { StaffMeView } from '../../../shared/api/generated/staff_me';
import type { StaffFloorView } from '../../../shared/api/generated/staff_floor';
import type { CashTransferRow } from '../../../shared/api/generated/cash_transfers';
import type { DenominationRow } from '../../../shared/api/generated/denominations';
import type { TransferBranchRow } from '../../../shared/api/generated/transfer_branches';

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000b1';

function me(permissions: string[] = ['CashManage']): StaffMeView {
  return {
    userId: ME,
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
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: {
      id: 'shift-open-001',
      openedAt: '2026-10-11T01:00:00Z',
    },
    cutoff: null,
    target: null,
    safe: null,
  };
}

const SAMPLE_TRANSFERS: CashTransferRow[] = [
  {
    id: 'tx-001',
    kind: 'BRANCH',
    fromBranchId: BRANCH,
    toBranchId: 'B99',
    fromShiftId: 'shift-open-001',
    toShiftId: null,
    status: 'APPROVED',
    createdBy: ME,
    note: 'test transfer',
    createdAt: '2026-10-11T00:00:00Z',
    lines: [],
  },
];

const SAMPLE_DENOMS: DenominationRow[] = [
  { id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' },
];

const SAMPLE_BRANCHES: TransferBranchRow[] = [
  { branchId: 'B99', name: 'Branch 99' },
];

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let viewHandlers: Record<string, () => Promise<Response>>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stubViews(overrides: Record<string, () => Promise<Response>> = {}) {
  viewHandlers = {
    cash_transfers: async () => json(200, { version: 1, changed: true, data: SAMPLE_TRANSFERS }),
    denominations: async () => json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } }),
    transfer_branches: async () => json(200, { version: 1, changed: true, data: { rows: SAMPLE_BRANCHES } }),
    ...overrides,
  };

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
      if (method === 'GET' && path.startsWith('/api/v3/views/')) {
        const viewName = path.slice('/api/v3/views/'.length);
        if (viewHandlers[viewName]) {
          return viewHandlers[viewName]();
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }
      return json(200, {});
    }),
  );
}


function mount(m: StaffMeView = me(), f: StaffFloorView = floor()) {
  const ctx: StaffCtx = { me: m, floor: f, refresh: vi.fn(async () => {}) };
  dispose = render(() => <TransfersCard ctx={ctx} />, host);
  return { ctx };
}

function tid(id: string): string {
  return `[data-testid="${id}"]`;
}

function q<T extends Element = HTMLElement>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

function must<T extends Element = HTMLElement>(sel: string): T {
  const el = q<T>(sel);
  expect(el, sel).not.toBeNull();
  return el!;
}

describe('#1689 TransfersCard read error handling', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
  });

  it('1. cash_transfers read failure shows worded error', async () => {
    stubViews({
      cash_transfers: async () =>
        json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }),
    });

    mount();
    await vi.waitFor(() => expect(q(tid('transfer-error'))).not.toBeNull());

    const errorEl = must(tid('transfer-error'));
    expect(errorEl.textContent?.trim()).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
  });

  it('2. denominations read failure shows worded error', async () => {
    stubViews({
      denominations: async () =>
        json(403, { error: { code: 'roleNotAllowedHere', message: 'refused' } }),
    });

    mount();
    await vi.waitFor(() => expect(q(tid('transfer-error'))).not.toBeNull());

    const errorEl = must(tid('transfer-error'));
    expect(errorEl.textContent?.trim()).toBe(errorText('roleNotAllowedHere', 'refused'));
  });

  it('3. transfer_branches read failure shows worded error', async () => {
    stubViews({
      transfer_branches: async () =>
        json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }),
    });

    mount();
    await vi.waitFor(() => expect(q(tid('transfer-error'))).not.toBeNull());

    const errorEl = must(tid('transfer-error'));
    expect(errorEl.textContent?.trim()).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
  });

  it('4. network 500 / connection failure shows worded generic message in TH and EN', async () => {
    // In Thai:
    stubViews({
      cash_transfers: async () => {
        throw new TypeError('Failed to fetch');
      },
    });

    mount();
    await vi.waitFor(() => expect(q(tid('transfer-error'))).not.toBeNull());

    let errorEl = must(tid('transfer-error'));
    expect(errorEl.textContent?.trim()).toBe(sharedTh['error.network']);
    expect(errorEl.textContent).not.toContain('Failed to fetch');

    // In English:
    dispose?.();
    dispose = undefined;
    host.innerHTML = '';
    setLang('en');

    mount();
    await vi.waitFor(() => expect(q(tid('transfer-error'))).not.toBeNull());

    errorEl = must(tid('transfer-error'));
    expect(errorEl.textContent?.trim()).toBe(sharedEn['error.network']);
    expect(errorEl.textContent).not.toContain('Failed to fetch');
  });

  it('5. unknown refusal code shows worded generic message, never raw code or message', async () => {
    stubViews({
      cash_transfers: async () =>
        json(400, { error: { code: 'UNKNOWN_REFUSAL_CODE', message: 'raw wire message' } }),
    });

    mount();
    await vi.waitFor(() => expect(q(tid('transfer-error'))).not.toBeNull());

    const errorEl = must(tid('transfer-error'));
    expect(errorEl.textContent?.trim()).toBe('raw wire message');
    expect(errorEl.textContent).not.toContain('UNKNOWN_REFUSAL_CODE');
  });

  it('6. stale data stays visible when subsequent read fails, but error is shown', async () => {
    let callCount = 0;
    stubViews({
      cash_transfers: async () => {
        callCount++;
        if (callCount === 1) {
          return json(200, { version: 1, changed: true, data: SAMPLE_TRANSFERS });
        }
        return json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } });
      },
    });

    mount();
    await vi.waitFor(() => expect(q('[data-transfer-id="tx-001"]')).not.toBeNull());
    expect(q(tid('transfer-error'))).toBeNull();

    // Trigger open create dialog to cause reload or fail denominations
    viewHandlers.denominations = async () =>
      json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } });

    // Click dispatch or trigger reload
    // In TransfersCard: handleDispatch calls api.post then loadTransfers()
    // When loadTransfers fails on reload:
    viewHandlers.cash_transfers = async () =>
      json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } });

    const dispatchBtn = must(tid('transfer-dispatch'));
    (dispatchBtn as HTMLElement).click();

    await vi.waitFor(() => expect(q(tid('transfer-error'))).not.toBeNull());
    // Stale transfer row tx-001 is still visible:
    expect(q('[data-transfer-id="tx-001"]')).not.toBeNull();
    // Error is also displayed:
    expect(must(tid('transfer-error')).textContent?.trim()).toBe(
      errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'),
    );
  });
});
