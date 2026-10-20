import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { en as sharedEn } from '../../../shared/i18n/en';
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

describe('Slips rework: branch list read failure (#1715)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let branchesResponse: (() => Response) | 'network';
  let slipsResponse: (() => Response) | 'network';

  beforeEach(() => {
    setLang('th');
    branchesResponse = () => json({ version: 1, changed: true, data: { branches: SAMPLE_BRANCHES } });
    slipsResponse = () => json({ version: 1, changed: true, data: SAMPLE_SLIPS });

    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (PAGE_WIDE.some((p) => url.includes(p))) {
        return json({ version: 1, changed: false });
      }
      if (url.includes('/api/v3/views/admin_branches')) {
        if (branchesResponse === 'network') {
          return new Response('Network error', { status: 500 });
        }
        return branchesResponse();
      }
      if (url.includes('/api/v3/views/admin_slips')) {
        if (slipsResponse === 'network') {
          return new Response('Network error', { status: 500 });
        }
        return slipsResponse();
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
    root = document.createElement('div');
    document.body.appendChild(root);
    dispose = render(
      () => <Slips branch={branch} period={period} shown={shown} />,
      root,
    );
    return { setBranch, setShown };
  }

  function errorAlertText(): string {
    const el = root.querySelector('[role="alert"]');
    return (el?.textContent ?? '').trim();
  }

  // Row 1: web/src/screens/admin/slips/Slips.tsx:47 — branches fail: empty picker, no message -> worded error
  describe('Row 1: branch list read failure shows worded error', () => {
    it('shows worded error when branches read fails with 500 (network failure)', async () => {
      branchesResponse = 'network';
      mount();

      await vi.waitFor(() => {
        expect(errorAlertText()).toContain(sharedTh['error.serverError']);
      });
    });

    it('shows worded error when branches read fails with known refusal code', async () => {
      branchesResponse = () =>
        json({ error: { code: 'roleNotAllowedHere', message: 'raw refusal' } }, 403);
      mount();

      await vi.waitFor(() => {
        expect(errorAlertText()).toContain(sharedTh['error.roleNotAllowedHere']);
      });
      expect(errorAlertText()).not.toContain('raw refusal');
      expect(errorAlertText()).not.toContain('roleNotAllowedHere');
    });

    it('shows the server sentence for an unknown code (W10 (4)), never the raw code', async () => {
      branchesResponse = () =>
        json({ error: { code: 'unknownBranchCode123', message: 'RAW_UNWORDED_MSG' } }, 400);
      mount();

      await vi.waitFor(() => {
        expect(errorAlertText()).toContain('RAW_UNWORDED_MSG');
      });
      expect(errorAlertText()).not.toContain('unknownBranchCode123');
    });

    it('shows English worded error when language is en and branches read fails', async () => {
      setLang('en');
      branchesResponse = () =>
        json({ error: { code: 'roleNotAllowedHere', message: 'raw refusal' } }, 403);
      mount();

      await vi.waitFor(() => {
        expect(errorAlertText()).toContain(sharedEn['error.roleNotAllowedHere']);
      });
    });
  });
});
