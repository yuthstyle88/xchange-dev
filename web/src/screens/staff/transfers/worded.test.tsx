import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { TransfersCard } from './TransfersCard';
import type { StaffCtx } from '../cards';
import {
  makeCashTransferRow,
  makeDenominationRow,
  makeStaffFloorView,
  makeStaffMeView,
  makeTransferBranchRow,
  type StaffFloorView,
  type StaffMeView,
} from '../../../shared/testing/fixtures';

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000b1';

function mockMe(permissions: string[] = ['CashManage', 'FinanceReconcile']) {
  return makeStaffMeView({
    userId: ME,
    name: 'Ploy',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions,
  });
}

function mockFloor() {
  return makeStaffFloorView({
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
  });
}

const SAMPLE_TRANSFERS = [
  makeCashTransferRow({
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
  }),
  makeCashTransferRow({
    id: 'tx-002',
    kind: 'BRANCH',
    fromBranchId: 'B99',
    toBranchId: BRANCH,
    fromShiftId: 'shift-remote',
    toShiftId: null,
    status: 'DISPATCHED',
    createdBy: 'other-user',
    note: 'dispatched transfer',
    createdAt: '2026-10-11T00:00:00Z',
    lines: [],
  }),
  makeCashTransferRow({
    id: 'tx-003',
    kind: 'BRANCH',
    fromBranchId: 'B99',
    toBranchId: BRANCH,
    fromShiftId: 'shift-remote',
    toShiftId: null,
    status: 'RECEIVED',
    createdBy: 'other-user',
    note: 'received transfer',
    createdAt: '2026-10-11T00:00:00Z',
    lines: [],
  }),
];

const SAMPLE_DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
];

const SAMPLE_BRANCHES = [
  makeTransferBranchRow({ branchId: 'B99', name: 'Branch 99' }),
];

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let customHandler: ((url: string, method: string, init?: RequestInit) => Promise<Response> | Response) | null = null;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function defaultFetch(url: string, method: string): Response {
  const path = url.split('?')[0]!;
  if (method === 'GET' && path.startsWith('/api/v3/views/')) {
    const viewName = path.slice('/api/v3/views/'.length);
    if (viewName === 'cash_transfers') {
      return json(200, { version: 1, changed: true, data: SAMPLE_TRANSFERS });
    }
    if (viewName === 'denominations') {
      return json(200, { version: 1, changed: true, data: { rows: SAMPLE_DENOMS } });
    }
    if (viewName === 'transfer_branches') {
      return json(200, { version: 1, changed: true, data: { rows: SAMPLE_BRANCHES } });
    }
    return json(404, { error: { code: 'notFound', message: 'not found' } });
  }
  return json(200, { id: 'tx-new-id' });
}

function installFetch() {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (customHandler) {
        const customRes = await customHandler(url, method, init);
        if (customRes) return customRes;
      }
      return defaultFetch(url, method);
    }),
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

function mount(m: StaffMeView = mockMe(), f: StaffFloorView = mockFloor()) {
  const ctx: StaffCtx = { me: m, floor: f, refresh: vi.fn(async () => {}) };
  dispose = render(() => <TransfersCard ctx={ctx} />, host);
  return { ctx };
}

function q<T extends Element = HTMLElement>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

function must<T extends Element = HTMLElement>(sel: string): T {
  const el = q<T>(sel);
  expect(el, sel).not.toBeNull();
  return el!;
}

const UNKEYED_MESSAGE = 'ข้อผิดพลาดเฉพาะที่ไม่มีคีย์ร่วมจากเซิร์ฟเวอร์';

describe('WE-44: screens/staff/transfers/ worded errors (web-worded-error.md §4.2)', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
    customHandler = null;
    installFetch();
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
    customHandler = null;
  });

  describe('catch 1: loadTransfers', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/cash_transfers')) throw new TypeError('Failed to fetch');
        return defaultFetch(url, 'GET');
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.network']);

      // EN
      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/cash_transfers')) {
          return json(500, { error: { code: 'roleNotAllowedHere', message: 'fail' } });
        }
        return defaultFetch(url, 'GET');
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/cash_transfers')) {
          return json(400, { error: { code: 'unkeyedTransfersCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, 'GET');
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 2: loadDenominations', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/denominations')) throw new TypeError('Failed to fetch');
        return defaultFetch(url, 'GET');
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/denominations')) {
          return json(500, { error: { code: 'roleNotAllowedHere', message: 'fail' } });
        }
        return defaultFetch(url, 'GET');
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/denominations')) {
          return json(400, { error: { code: 'unkeyedDenomsCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, 'GET');
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 3: loadTransferBranches', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/transfer_branches')) throw new TypeError('Failed to fetch');
        return defaultFetch(url, 'GET');
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/transfer_branches')) {
          return json(500, { error: { code: 'roleNotAllowedHere', message: 'fail' } });
        }
        return defaultFetch(url, 'GET');
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url) => {
        if (url.includes('/api/v3/views/transfer_branches')) {
          return json(400, { error: { code: 'unkeyedBranchesCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, 'GET');
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 4: handleCreateSubmit', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-transfers') && method === 'POST') throw new TypeError('Failed to fetch');
        return defaultFetch(url, method);
      };
      mount();
      await settle();
      must('[data-testid="transfer-new"]').click();
      await settle();
      const sel = must<HTMLSelectElement>('[data-testid="transfer-to-branch"]');
      sel.value = 'B99';
      sel.dispatchEvent(new Event('change'));
      await settle();
      must('[data-testid="transfer-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount();
      await settle();
      must('[data-testid="transfer-new"]').click();
      await settle();
      const selEn = must<HTMLSelectElement>('[data-testid="transfer-to-branch"]');
      selEn.value = 'B99';
      selEn.dispatchEvent(new Event('change'));
      await settle();
      must('[data-testid="transfer-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-transfers') && method === 'POST') {
          return json(500, { error: { code: 'roleNotAllowedHere', message: 'fail' } });
        }
        return defaultFetch(url, method);
      };
      mount();
      await settle();
      must('[data-testid="transfer-new"]').click();
      await settle();
      const sel = must<HTMLSelectElement>('[data-testid="transfer-to-branch"]');
      sel.value = 'B99';
      sel.dispatchEvent(new Event('change'));
      await settle();
      must('[data-testid="transfer-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-transfers') && method === 'POST') {
          return json(400, { error: { code: 'unkeyedCreateCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, method);
      };
      mount();
      await settle();
      must('[data-testid="transfer-new"]').click();
      await settle();
      const sel = must<HTMLSelectElement>('[data-testid="transfer-to-branch"]');
      sel.value = 'B99';
      sel.dispatchEvent(new Event('change'));
      await settle();
      must('[data-testid="transfer-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 5: handleDispatch', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-transfers/tx-001/dispatch') && method === 'POST') {
          throw new TypeError('Failed to fetch');
        }
        return defaultFetch(url, method);
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-dispatch"]')).not.toBeNull());
      must('[data-testid="transfer-dispatch"]').click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-dispatch"]')).not.toBeNull());
      must('[data-testid="transfer-dispatch"]').click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-transfers/tx-001/dispatch') && method === 'POST') {
          return json(500, { error: { code: 'roleNotAllowedHere', message: 'fail' } });
        }
        return defaultFetch(url, method);
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-dispatch"]')).not.toBeNull());
      must('[data-testid="transfer-dispatch"]').click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-transfers/tx-001/dispatch') && method === 'POST') {
          return json(400, { error: { code: 'unkeyedDispatchCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, method);
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-dispatch"]')).not.toBeNull());
      must('[data-testid="transfer-dispatch"]').click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 6: handleReceiveSubmit', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-transfers/tx-002/receive') && method === 'POST') {
          throw new TypeError('Failed to fetch');
        }
        return defaultFetch(url, method);
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-receive"]')).not.toBeNull());
      must('[data-testid="transfer-receive"]').click();
      await settle();
      must('[data-testid="transfer-receive-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-receive"]')).not.toBeNull());
      must('[data-testid="transfer-receive"]').click();
      await settle();
      must('[data-testid="transfer-receive-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-transfers/tx-002/receive') && method === 'POST') {
          return json(500, { error: { code: 'roleNotAllowedHere', message: 'fail' } });
        }
        return defaultFetch(url, method);
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-receive"]')).not.toBeNull());
      must('[data-testid="transfer-receive"]').click();
      await settle();
      must('[data-testid="transfer-receive-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-transfers/tx-002/receive') && method === 'POST') {
          return json(400, { error: { code: 'unkeyedReceiveCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, method);
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-receive"]')).not.toBeNull());
      must('[data-testid="transfer-receive"]').click();
      await settle();
      must('[data-testid="transfer-receive-save"]').click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });

  describe('catch 7: handleReconcileConfirm', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-transfers/tx-003/reconcile') && method === 'POST') {
          throw new TypeError('Failed to fetch');
        }
        return defaultFetch(url, method);
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-reconcile"]')).not.toBeNull());
      must('[data-testid="transfer-reconcile"]').click();
      await settle();
      const confirmBtn = must<HTMLButtonElement>('.xc-dialog-actions button:last-child');
      confirmBtn.click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.network']);

      dispose?.();
      host.innerHTML = '';
      setLang('en');
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-reconcile"]')).not.toBeNull());
      must('[data-testid="transfer-reconcile"]').click();
      await settle();
      const confirmBtnEn = must<HTMLButtonElement>('.xc-dialog-actions button:last-child');
      confirmBtnEn.click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(en['error.network']);
    });

    it('(2) 500 refusal shows error.serverError', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-transfers/tx-003/reconcile') && method === 'POST') {
          return json(500, { error: { code: 'roleNotAllowedHere', message: 'fail' } });
        }
        return defaultFetch(url, method);
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-reconcile"]')).not.toBeNull());
      must('[data-testid="transfer-reconcile"]').click();
      await settle();
      const confirmBtn = must<HTMLButtonElement>('.xc-dialog-actions button:last-child');
      confirmBtn.click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(th['error.serverError']);
    });

    it('(3) unkeyed code with server message shows message', async () => {
      customHandler = (url, method) => {
        if (url.includes('/api/cash-transfers/tx-003/reconcile') && method === 'POST') {
          return json(400, { error: { code: 'unkeyedReconcileCode', message: UNKEYED_MESSAGE } });
        }
        return defaultFetch(url, method);
      };
      mount();
      await vi.waitFor(() => expect(q('[data-testid="transfer-reconcile"]')).not.toBeNull());
      must('[data-testid="transfer-reconcile"]').click();
      await settle();
      const confirmBtn = must<HTMLButtonElement>('.xc-dialog-actions button:last-child');
      confirmBtn.click();
      await vi.waitFor(() => expect(q('[data-testid="transfer-error"]')).not.toBeNull());
      expect(must('[data-testid="transfer-error"]').textContent?.trim()).toBe(UNKEYED_MESSAGE);
    });
  });
});
