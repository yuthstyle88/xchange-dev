import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { en as sharedEn } from '../../../shared/i18n/en';
import { Payments } from './Payments';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';
import type { AdminPaymentsData } from '../../../shared/api/generated/admin_payments';

const SAMPLE_PAYMENTS_DATA: AdminPaymentsData = {
  branchId: 'MAIN',
  settingsVersion: 1,
  methods: [
    { id: 'Cash', enabled: true, buy: true, sell: true, account: 'Main Cash', instruction: 'Pay at counter' },
    { id: 'Credit Card', enabled: false, buy: false, sell: true, account: '', instruction: '' },
    { id: 'Alipay', enabled: false, buy: false, sell: true, account: '', instruction: '' },
    { id: 'PromptPay', enabled: false, buy: false, sell: true, account: '', instruction: '' },
    { id: 'Bank Transfer', enabled: false, buy: false, sell: true, account: '', instruction: '' },
  ],
  updatedAt: null,
  updatedBy: null,
  canEdit: true,
  history: [],
};

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

describe('Payments rework: branch list read failure (#1713)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let branchesResponse: (() => Response) | 'network';
  let paymentsResponse: (() => Response) | 'network';

  beforeEach(() => {
    setLang('th');
    branchesResponse = () => json({ version: 1, changed: true, data: { branches: SAMPLE_BRANCHES } });
    paymentsResponse = () => json({ version: 1, changed: true, data: SAMPLE_PAYMENTS_DATA });

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
      if (url.includes('/api/v3/views/admin_payments')) {
        if (paymentsResponse === 'network') {
          return new Response('Network error', { status: 500 });
        }
        return paymentsResponse();
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
      () => <Payments branch={branch} period={period} shown={shown} />,
      root,
    );
    return { setBranch, setShown };
  }

  function errorBannerText(): string {
    const el = root.querySelector('.error-banner, [role="alert"]');
    return (el?.textContent ?? '').trim();
  }

  // Row 1: web/src/screens/admin/payments/Payments.tsx:122 — branches fail: empty picker... -> worded error via existing setError
  describe('Row 1: branch list read failure shows worded error', () => {
    it('shows worded error when branches read fails with 500 (network failure)', async () => {
      branchesResponse = 'network';
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('shows worded error when branches read fails with known refusal code', async () => {
      branchesResponse = () =>
        json({ error: { code: 'roleNotAllowedHere', message: 'raw refusal' } }, 403);
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.roleNotAllowedHere']);
      });
      expect(errorBannerText()).not.toContain('raw refusal');
      expect(errorBannerText()).not.toContain('roleNotAllowedHere');
    });

    it('shows generic worded error when branches read fails with unknown code, never raw code or err.message', async () => {
      branchesResponse = () =>
        json({ error: { code: 'unknownBranchCode123', message: 'RAW_UNWORDED_MSG' } }, 400);
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.serverError']);
      });
      expect(errorBannerText()).not.toContain('unknownBranchCode123');
      expect(errorBannerText()).not.toContain('RAW_UNWORDED_MSG');
    });

    it('shows English worded error when language is en and branches read fails', async () => {
      setLang('en');
      branchesResponse = () =>
        json({ error: { code: 'roleNotAllowedHere', message: 'raw refusal' } }, 403);
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedEn['error.roleNotAllowedHere']);
      });
    });

    it('keeps stale payments data visible when branch list read fails on switch', async () => {
      const { setBranch } = mount('MAIN');
      // Wait for initial load with successful branches and payments
      await vi.waitFor(() => {
        expect(root.textContent).toContain('ช่องทางของ');
      });
      expect(errorBannerText()).toBe('');

      // Now switch branch and simulate branches read failure
      branchesResponse = 'network';
      setBranch('B2');

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.serverError']);
      });
      // Stale payments data remains visible
      expect(root.textContent).toContain('ช่องทางของ');
    });
  });
});
