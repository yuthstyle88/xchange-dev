import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { en as sharedEn } from '../../../shared/i18n/en';
import { Payments } from './Payments';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';
import {
  makeAdminPaymentsData,
  makePaymentMethodItem,
  type AdminPaymentsData,
} from '../../../shared/testing/fixtures';

const SAMPLE_BRANCHES = [
  { id: 'MAIN', name: 'Main Desk' },
  { id: 'B2', name: 'Branch 2' },
];

const SAMPLE_PAYMENTS_DATA: AdminPaymentsData = makeAdminPaymentsData({
  branchId: 'MAIN',
  settingsVersion: 1,
  methods: [
    makePaymentMethodItem({ id: 'Cash', enabled: true, buy: true, sell: true, account: 'Main Cash', instruction: 'Pay at counter' }),
    makePaymentMethodItem({ id: 'Credit Card', enabled: false, buy: false, sell: true, account: '', instruction: '' }),
    makePaymentMethodItem({ id: 'Alipay', enabled: false, buy: false, sell: true, account: '', instruction: '' }),
    makePaymentMethodItem({ id: 'PromptPay', enabled: false, buy: false, sell: true, account: '', instruction: '' }),
    makePaymentMethodItem({ id: 'Bank Transfer', enabled: false, buy: false, sell: true, account: '', instruction: '' }),
  ],
  updatedAt: null,
  updatedBy: null,
  canEdit: true,
  history: [],
});

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('Payments wordedError (#1768)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let branchesHandler: () => Promise<Response>;
  let paymentsHandler: (branch: string) => Promise<Response>;
  let saveHandler: () => Promise<Response>;

  beforeEach(() => {
    setLang('th');
    branchesHandler = async () =>
      json({ version: 1, changed: true, data: { branches: SAMPLE_BRANCHES } });
    paymentsHandler = async (branch: string) =>
      json({
        version: 1,
        changed: true,
        data: { ...SAMPLE_PAYMENTS_DATA, branchId: branch },
      });
    saveHandler = async () => json({ ok: true });

    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const method = (init?.method ?? 'GET').toUpperCase();

      if (PAGE_WIDE.some((p) => url.includes(p))) {
        return json({ version: 1, changed: false });
      }

      if (url.includes('/api/v3/views/admin_branches')) {
        return branchesHandler();
      }

      if (url.includes('/api/v3/views/admin_payments')) {
        const u = new URL(url, 'http://localhost');
        const branch = u.searchParams.get('branch') ?? 'MAIN';
        return paymentsHandler(branch);
      }

      if (method === 'POST' && url.includes('/api/payment-methods')) {
        return saveHandler();
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

  function cardIds(): string[] {
    return Array.from(root.querySelectorAll<HTMLElement>('[data-payment-id]')).map(
      (e) => e.dataset.paymentId ?? '',
    );
  }

  function errorBannerText(): string {
    const el = root.querySelector('.error-banner, [role="alert"]');
    return (el?.textContent ?? '').trim();
  }

  function feedbackText(): string {
    const el = root.querySelector('#payment-feedback, .xc-payment-feedback');
    return (el?.textContent ?? '').trim();
  }

  // ── Catch 1: loadBranches ──────────────────────────────────────────────────
  describe('Catch 1: loadBranches', () => {
    it('shows error.network on network TypeError in TH and EN', async () => {
      branchesHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.network']);
      });

      dispose?.();
      setLang('en');
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedEn['error.network']);
      });
    });

    it('shows error.serverError on 500 response with keyed non-server code', async () => {
      branchesHandler = async () =>
        json({ error: { code: 'roleNotAllowedHere', message: 'role not allowed' } }, 500);
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('shows server message on refusal with unknown code and server message', async () => {
      branchesHandler = async () =>
        json({ error: { code: 'unknownBranchRefusal123', message: 'ข้อความปฏิเสธจากเซิร์ฟเวอร์สาขา' } }, 400);
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe('ข้อความปฏิเสธจากเซิร์ฟเวอร์สาขา');
      });
    });
  });

  // ── Catch 2: loadData ──────────────────────────────────────────────────────
  describe('Catch 2: loadData', () => {
    it('shows error.network on network TypeError in TH and EN', async () => {
      paymentsHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.network']);
      });

      dispose?.();
      setLang('en');
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedEn['error.network']);
      });
    });

    it('shows error.serverError on 500 response with keyed non-server code', async () => {
      paymentsHandler = async () =>
        json({ error: { code: 'roleNotAllowedHere', message: 'role not allowed' } }, 500);
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('shows server message on refusal with unknown code and server message', async () => {
      paymentsHandler = async () =>
        json({ error: { code: 'unknownPaymentRefusal456', message: 'ข้อความปฏิเสธการโหลดวิธีชำระเงิน' } }, 400);
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe('ข้อความปฏิเสธการโหลดวิธีชำระเงิน');
      });
    });

    it('clears previous branch data on branch switch failure, but retains data on same-branch reload failure', async () => {
      const { setBranch } = mount('MAIN');

      // Wait for MAIN data to load successfully
      await vi.waitFor(() => {
        expect(cardIds().length).toBe(5);
      });
      expect(errorBannerText()).toBe('');

      // 1. Switch to B2, but B2 read fails with 500
      paymentsHandler = async (b: string) => {
        if (b === 'B2') {
          return new Response('Server Error', { status: 500 });
        }
        return json({
          version: 1,
          changed: true,
          data: { ...SAMPLE_PAYMENTS_DATA, branchId: b },
        });
      };

      setBranch('B2');

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.serverError']);
      });
      // Previous branch MAIN data must NOT be shown
      expect(cardIds()).toEqual([]);

      // 2. Switch back to MAIN which succeeds
      setBranch('MAIN');
      await vi.waitFor(() => {
        expect(cardIds().length).toBe(5);
      });
      expect(errorBannerText()).toBe('');

      // 3. Same branch failure (e.g. reload fails) retains stale data
      paymentsHandler = async () => new Response('Server Error', { status: 500 });
      const reloadBtn = Array.from(root.querySelectorAll('button')).find((b) =>
        b.textContent?.includes('โหลดล่าสุด'),
      );
      expect(reloadBtn).toBeDefined();
      reloadBtn?.click();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.serverError']);
      });
      // Stale data remains for the same branch
      expect(cardIds().length).toBe(5);
    });
  });

  // ── Catch 3: handleSave ────────────────────────────────────────────────────
  describe('Catch 3: handleSave', () => {
    async function setupForSave() {
      mount('MAIN');
      await vi.waitFor(() => {
        expect(cardIds().length).toBe(5);
      });

      // Find an input to dirty the form
      const accountInput = root.querySelector<HTMLInputElement>(
        '[data-payment-id="Cash"] input[type="text"]',
      );
      expect(accountInput).toBeDefined();
      accountInput!.value = 'Edited Cash Account';
      accountInput!.dispatchEvent(new Event('input', { bubbles: true }));
      accountInput!.dispatchEvent(new Event('change', { bubbles: true }));

      // Find save button
      const saveBtn = Array.from(root.querySelectorAll('button')).find(
        (b) =>
          b.textContent?.includes('บันทึกการตั้งค่า') ||
          b.textContent?.includes('Save settings') ||
          b.textContent?.includes('บันทึก'),
      );
      expect(saveBtn).toBeDefined();
      expect(saveBtn?.disabled).toBe(false);
      return saveBtn!;
    }

    it('shows error.network on network TypeError in TH and EN', async () => {
      saveHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      const saveBtn = await setupForSave();
      saveBtn.click();

      await vi.waitFor(() => {
        expect(feedbackText()).toBe(sharedTh['error.network']);
      });

      dispose?.();
      setLang('en');
      const saveBtn2 = await setupForSave();
      saveBtn2.click();

      await vi.waitFor(() => {
        expect(feedbackText()).toBe(sharedEn['error.network']);
      });
    });

    it('shows error.serverError on 500 response with keyed non-server code', async () => {
      saveHandler = async () =>
        json({ error: { code: 'roleNotAllowedHere', message: 'role not allowed' } }, 500);
      const saveBtn = await setupForSave();
      saveBtn.click();

      await vi.waitFor(() => {
        expect(feedbackText()).toBe(sharedTh['error.serverError']);
      });
    });

    it('shows server message on refusal with unknown code and server message', async () => {
      saveHandler = async () =>
        json({ error: { code: 'paymentCustomRefusal', message: 'ไม่อนุญาตให้แก้ไขช่องทางนี้' } }, 400);
      const saveBtn = await setupForSave();
      saveBtn.click();

      await vi.waitFor(() => {
        expect(feedbackText()).toBe('ไม่อนุญาตให้แก้ไขช่องทางนี้');
      });
    });
  });
});
