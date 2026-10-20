import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import { Slips } from './Slips';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';
import { makeTransactionRow } from '../../../shared/testing/fixtures';

const SAMPLE_SLIPS = [
  makeTransactionRow({
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
  }),
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

describe('WE-24 screens/admin/slips/ worded error (#1773)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let branchesHandler: () => Promise<Response>;
  let slipsHandler: () => Promise<Response>;

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);

    branchesHandler = async () => json({ version: 1, changed: true, data: { branches: SAMPLE_BRANCHES } });
    slipsHandler = async () => json({ version: 1, changed: true, data: SAMPLE_SLIPS });

    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (PAGE_WIDE.some((p) => url.includes(p))) {
        return json({ version: 1, changed: false });
      }
      if (url.includes('/api/v3/views/admin_branches')) {
        return branchesHandler();
      }
      if (url.includes('/api/v3/views/admin_slips')) {
        return slipsHandler();
      }
      return json({ error: { code: 'notFound', message: 'not found' } }, 404);
    }));
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

  function errorAlertText(): string {
    const el = root.querySelector('[role="alert"] p') || root.querySelector('[role="alert"]');
    return (el?.textContent ?? '').trim();
  }

  describe('Catch 1: loadBranches (admin_branches)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      setLang('th');
      branchesHandler = async () => {
        throw new TypeError('Failed to fetch');
      };
      mount();

      await vi.waitFor(() => {
        expect(errorAlertText()).toBe(sharedTh['error.network']);
      });

      // Switch language to en and remount
      setLang('en');
      dispose?.();
      dispose = undefined;
      root.innerHTML = '';
      mount();

      await vi.waitFor(() => {
        expect(errorAlertText()).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows th["error.serverError"]', async () => {
      setLang('th');
      branchesHandler = async () =>
        json({ error: { code: 'roleNotAllowedHere', message: 'x' } }, 500);
      mount();

      await vi.waitFor(() => {
        expect(errorAlertText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      setLang('th');
      const serverMessage = 'สาขาถูกปิดการใช้งานชั่วคราวโดยผู้ดูแลระบบ';
      branchesHandler = async () =>
        json({ error: { code: 'unmappedBranchCode999', message: serverMessage } }, 400);
      mount();

      await vi.waitFor(() => {
        expect(errorAlertText()).toBe(serverMessage);
      });
    });
  });

  describe('Catch 2: fetchSlips (admin_slips)', () => {
    it('(1) network TypeError shows th["error.network"] and after setLang("en") shows en["error.network"]', async () => {
      setLang('th');
      slipsHandler = async () => {
        throw new TypeError('Failed to fetch');
      };
      const { setBranch } = mount();

      await vi.waitFor(() => {
        expect(errorAlertText()).toBe(sharedTh['error.network']);
      });

      // Switch language to en and trigger fetch
      setLang('en');
      setBranch('B2');

      await vi.waitFor(() => {
        expect(errorAlertText()).toBe(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal shows th["error.serverError"]', async () => {
      setLang('th');
      slipsHandler = async () =>
        json({ error: { code: 'roleNotAllowedHere', message: 'x' } }, 500);
      mount();

      await vi.waitFor(() => {
        expect(errorAlertText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unmapped code and server message shows that message', async () => {
      setLang('th');
      const serverMessage = 'ไม่สามารถค้นหารายการสลิปเนื่องจากข้อผิดพลาดเฉพาะ';
      slipsHandler = async () =>
        json({ error: { code: 'unmappedSlipCode456', message: serverMessage } }, 400);
      mount();

      await vi.waitFor(() => {
        expect(errorAlertText()).toBe(serverMessage);
      });
    });
  });

  describe('Criterion 3: stale data on branch switch', () => {
    it('clears stale data from previous branch when subsequent branch read fails', async () => {
      // 1. Initial branch load succeeds
      const { setBranch } = mount('MAIN');

      await vi.waitFor(() => {
        expect(root.textContent).toContain('XC-20261008-000001');
      });

      // 2. Next branch read fails
      slipsHandler = async () =>
        json({ error: { code: 'branchNotAccessible', message: 'เข้าถึงไม่ได้' } }, 400);
      setBranch('B2');

      await vi.waitFor(() => {
        expect(errorAlertText()).toBe('เข้าถึงไม่ได้');
      });

      // Verify previous branch's rows are no longer displayed
      expect(root.textContent).not.toContain('XC-20261008-000001');
      expect(root.querySelector('table tbody tr')).toBeNull();
    });
  });
});
