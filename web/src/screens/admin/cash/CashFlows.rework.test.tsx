import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import type { CashCountRow } from '../../../shared/api/generated/cash_counts';
import type { CashTransferRow } from '../../../shared/api/generated/cash_transfers';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { CashFlows } from './CashFlows';

interface Reply {
  status: number;
  body: unknown;
}

const VERIFY = 'ShiftCloseCertify';
const MANAGE = 'CashManage';
const RECONCILE = 'FinanceReconcile';

const MOCK_COUNT: CashCountRow = {
  id: 'cc-1',
  branchId: 'MAIN',
  shiftId: 'S-01',
  kind: 'CLOSE',
  countedBy: 'EMP-1',
  countedAt: '2026-10-15T01:00:00Z',
  verifiedBy: null,
  verifiedAt: null,
  note: null,
  totals: [{ currency: 'THB', counted: '1000', expected: '1000', difference: '0' }],
};

const MOCK_TRANSFER: CashTransferRow = {
  id: 'tr-1',
  kind: 'EXTERNAL_OUT',
  status: 'DRAFT',
  fromBranchId: 'MAIN',
  toBranchId: null,
  fromShiftId: null,
  toShiftId: null,
  createdBy: 'EMP-1',
  createdAt: '2026-10-15T01:00:00Z',
  note: 'test note',
  lines: [{ currency: 'THB', denominationId: 1, qtySent: '10', qtyReceived: null }],
};

class TestServer {
  meRefusal: Reply | 'network' | null = null;
  countsRefusal: Reply | 'network' | null = null;
  transfersRefusal: Reply | 'network' | null = null;
  commandRefusal: Reply | 'network' | null = null;

  me = {
    id: 'u-admin',
    name: 'Admin',
    roles: ['ADMIN'],
    permissions: [VERIFY, MANAGE, RECONCILE],
    branch: 'MAIN',
    branchName: 'Headquarters',
    branches: [{ id: 'MAIN', code: 'MAIN', name: 'Headquarters' }],
  };

  counts: CashCountRow[] = [MOCK_COUNT];
  transfers: CashTransferRow[] = [MOCK_TRANSFER];

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(rawUrl, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();

    const json = (reply: Reply): Response =>
      new Response(JSON.stringify(reply.body), {
        status: reply.status,
        headers: { 'content-type': 'application/json' },
      });

    if (method === 'POST') {
      if (this.commandRefusal === 'network') throw new TypeError('Failed to fetch');
      if (this.commandRefusal) return json(this.commandRefusal);
      return json({ status: 200, body: { id: 'cc-1', status: 'VERIFIED' } });
    }

    if (url.pathname === '/api/v3/views/me') {
      if (this.meRefusal === 'network') throw new TypeError('Failed to fetch');
      if (this.meRefusal) return json(this.meRefusal);
      return json({
        status: 200,
        body: { version: 1, changed: true, data: this.me },
      });
    }

    if (url.pathname === '/api/v3/views/cash_counts') {
      if (this.countsRefusal === 'network') throw new TypeError('Failed to fetch');
      if (this.countsRefusal) return json(this.countsRefusal);
      return json({ status: 200, body: { version: 1, changed: true, data: this.counts } });
    }

    if (url.pathname === '/api/v3/views/cash_transfers') {
      if (this.transfersRefusal === 'network') throw new TypeError('Failed to fetch');
      if (this.transfersRefusal) return json(this.transfersRefusal);
      return json({ status: 200, body: { version: 1, changed: true, data: this.transfers } });
    }

    if (url.pathname === '/api/v3/views/denominations') {
      return json({ status: 200, body: { version: 1, changed: true, data: { rows: [] } } });
    }

    if (url.pathname === '/api/v3/views/transfer_branches') {
      return json({ status: 200, body: { version: 1, changed: true, data: { rows: [] } } });
    }

    return json({ status: 200, body: { version: 1, changed: false } });
  };
}

describe('CashFlows rework: me read failure shows worded error (#1719)', () => {
  let server: TestServer;
  let root: HTMLElement;
  let dispose: (() => void) | undefined;

  function mount(branch = 'MAIN') {
    const [b, setBranch] = createSignal(branch);
    const [shown] = createSignal(true);
    const [shifts] = createSignal([]);

    root = document.createElement('div');
    document.body.appendChild(root);
    dispose = render(
      () => (
        <>
          <CashFlows branch={b} shown={shown} shifts={shifts} />
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );
    return { setBranch };
  }

  function alertText(): string {
    const el = root.querySelector('.cash-flows [role="alert"], .cash-flows .error-message');
    return (el?.textContent ?? '').trim();
  }

  beforeEach(() => {
    setLang('th');
    server = new TestServer();
    vi.stubGlobal('fetch', vi.fn(server.fetch));
  });

  afterEach(async () => {
    await pendingWrite.abandon();
    closeDialog();
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  // Row 1: web/src/screens/admin/cash/CashFlows.tsx:89 — me fails: permissions [], action buttons silently hidden -> worded error; actions shown disabled with the reason
  describe('Row 1: CashFlows.tsx:89 me read failure', () => {
    it('shows network worded error and actions disabled with reason when me read fails with network failure', async () => {
      server.meRefusal = 'network';
      mount();

      const expectedReason = sharedTh['error.network'];
      await vi.waitFor(() => {
        expect(alertText()).toBe(expectedReason);
        expect(root.querySelector('[data-action="verify"]')).not.toBeNull();
        expect(root.querySelector('[data-action="create"]')).not.toBeNull();
      });
      expect(alertText()).not.toContain('Failed to fetch');

      // Actions are rendered and disabled with the reason
      const verifyBtn = root.querySelector<HTMLButtonElement>('[data-action="verify"]')!;
      expect(verifyBtn.disabled).toBe(true);
      expect(verifyBtn.title).toBe(expectedReason);

      const createBtn = root.querySelector<HTMLButtonElement>('[data-action="create"]')!;
      expect(createBtn.disabled).toBe(true);
      expect(createBtn.title).toBe(expectedReason);
    });

    it('shows serverError worded error and actions disabled with reason when me read fails with 500', async () => {
      server.meRefusal = {
        status: 500,
        body: { error: { code: 'serverError', message: 'fatal crash' } },
      };
      mount();

      const expectedReason = sharedTh['error.serverError'];
      await vi.waitFor(() => {
        expect(alertText()).toBe(expectedReason);
        expect(root.querySelector('[data-action="verify"]')).not.toBeNull();
        expect(root.querySelector('[data-action="create"]')).not.toBeNull();
      });
      expect(alertText()).not.toContain('fatal crash');

      const verifyBtn = root.querySelector<HTMLButtonElement>('[data-action="verify"]')!;
      expect(verifyBtn.disabled).toBe(true);
      expect(verifyBtn.title).toBe(expectedReason);

      const createBtn = root.querySelector<HTMLButtonElement>('[data-action="create"]')!;
      expect(createBtn.disabled).toBe(true);
      expect(createBtn.title).toBe(expectedReason);
    });

    it('shows worded error and actions disabled with reason when me read fails with known refusal code', async () => {
      server.meRefusal = {
        status: 403,
        body: { error: { code: 'roleNotAllowedHere', message: 'raw forbidden message' } },
      };
      mount();

      const expectedReason = sharedTh['error.roleNotAllowedHere'];
      await vi.waitFor(() => {
        expect(alertText()).toBe(expectedReason);
        expect(root.querySelector('[data-action="verify"]')).not.toBeNull();
        expect(root.querySelector('[data-action="create"]')).not.toBeNull();
      });
      expect(alertText()).not.toContain('roleNotAllowedHere');
      expect(alertText()).not.toContain('raw forbidden message');

      const verifyBtn = root.querySelector<HTMLButtonElement>('[data-action="verify"]')!;
      expect(verifyBtn.disabled).toBe(true);
      expect(verifyBtn.title).toBe(expectedReason);

      const createBtn = root.querySelector<HTMLButtonElement>('[data-action="create"]')!;
      expect(createBtn.disabled).toBe(true);
      expect(createBtn.title).toBe(expectedReason);
    });

    it('shows server sentence and actions disabled with reason when me read fails with unknown code and server message', async () => {
      const serverSentence = 'ผู้ใช้นี้ถูกระงับสิทธิ์ชั่วคราว';
      server.meRefusal = {
        status: 400,
        body: { error: { code: 'unknownMeCode123', message: serverSentence } },
      };
      mount();

      const expectedReason = serverSentence;
      await vi.waitFor(() => {
        expect(alertText()).toBe(expectedReason);
        expect(root.querySelector('[data-action="verify"]')).not.toBeNull();
        expect(root.querySelector('[data-action="create"]')).not.toBeNull();
      });
      expect(alertText()).not.toContain('unknownMeCode123');
      expect(alertText()).not.toBe(sharedTh['error.serverError']);

      const verifyBtn = root.querySelector<HTMLButtonElement>('[data-action="verify"]')!;
      expect(verifyBtn.disabled).toBe(true);
      expect(verifyBtn.title).toBe(expectedReason);

      const createBtn = root.querySelector<HTMLButtonElement>('[data-action="create"]')!;
      expect(createBtn.disabled).toBe(true);
      expect(createBtn.title).toBe(expectedReason);
    });

    it('shows English network worded error and actions disabled with reason when language is en and me read fails with network error', async () => {
      setLang('en');
      server.meRefusal = 'network';
      mount();

      const expectedReason = sharedEn['error.network'];
      await vi.waitFor(() => {
        expect(alertText()).toBe(expectedReason);
        expect(root.querySelector('[data-action="verify"]')).not.toBeNull();
        expect(root.querySelector('[data-action="create"]')).not.toBeNull();
      });
      expect(alertText()).not.toContain('Failed to fetch');

      const verifyBtn = root.querySelector<HTMLButtonElement>('[data-action="verify"]')!;
      expect(verifyBtn.disabled).toBe(true);
      expect(verifyBtn.title).toBe(expectedReason);

      const createBtn = root.querySelector<HTMLButtonElement>('[data-action="create"]')!;
      expect(createBtn.disabled).toBe(true);
      expect(createBtn.title).toBe(expectedReason);
    });

    it('shows English worded error and actions disabled with reason when language is en and me read fails with known refusal code', async () => {
      setLang('en');
      server.meRefusal = {
        status: 403,
        body: { error: { code: 'roleNotAllowedHere', message: 'raw' } },
      };
      mount();

      const expectedReason = sharedEn['error.roleNotAllowedHere'];
      await vi.waitFor(() => {
        expect(alertText()).toBe(expectedReason);
        expect(root.querySelector('[data-action="verify"]')).not.toBeNull();
        expect(root.querySelector('[data-action="create"]')).not.toBeNull();
      });
      expect(alertText()).not.toContain('roleNotAllowedHere');

      const verifyBtn = root.querySelector<HTMLButtonElement>('[data-action="verify"]')!;
      expect(verifyBtn.disabled).toBe(true);
      expect(verifyBtn.title).toBe(expectedReason);

      const createBtn = root.querySelector<HTMLButtonElement>('[data-action="create"]')!;
      expect(createBtn.disabled).toBe(true);
      expect(createBtn.title).toBe(expectedReason);
    });

    it('shows sections and action buttons enabled without error alert when me read succeeds', async () => {
      mount();

      await vi.waitFor(() => {
        expect(root.querySelector('[data-section="counts-waiting"]')).not.toBeNull();
        expect(root.querySelector('[data-section="transfers"]')).not.toBeNull();
        expect(root.querySelector('[data-section="new-transfer"]')).not.toBeNull();
      });
      expect(alertText()).toBe('');

      const verifyBtn = root.querySelector<HTMLButtonElement>('[data-action="verify"]');
      expect(verifyBtn).not.toBeNull();
      expect(verifyBtn!.disabled).toBe(false);
      expect(verifyBtn!.title).toBe('');

      const createBtn = root.querySelector<HTMLButtonElement>('[data-action="create"]');
      expect(createBtn).not.toBeNull();
      expect(createBtn!.title).toBe('');
    });
  });

  describe('Commands: a command refusal through send shows the server sentence', () => {
    it('shows server sentence when command answered 400 with unknown code and server message', async () => {
      const serverSentence = 'ยอดเงินสดในเก๊ะไม่ตรงกับระบบ กรุณาตรวจสอบใหม่';
      server.commandRefusal = {
        status: 400,
        body: { error: { code: 'unknownVerifyCode', message: serverSentence } },
      };
      mount();

      await vi.waitFor(() => {
        expect(root.querySelector('[data-action="verify"]')).not.toBeNull();
      });

      const verifyBtn = root.querySelector<HTMLButtonElement>('[data-action="verify"]')!;
      expect(verifyBtn.disabled).toBe(false);
      verifyBtn.click();

      await vi.waitFor(() => {
        expect(alertText()).toBe(serverSentence);
      });
      expect(alertText()).not.toContain('unknownVerifyCode');
      expect(alertText()).not.toBe(sharedTh['error.serverError']);
    });
  });

  describe('Edge cases: counts and transfers read failures also show worded error', () => {
    it('shows worded error when counts read fails with network error', async () => {
      server.countsRefusal = 'network';
      mount();

      await vi.waitFor(() => {
        expect(alertText()).toBe(sharedTh['error.network']);
      });
      expect(alertText()).not.toContain('Failed to fetch');
    });

    it('shows worded error when counts read fails with known refusal code', async () => {
      server.countsRefusal = {
        status: 403,
        body: { error: { code: 'roleNotAllowedHere', message: 'counts forbidden' } },
      };
      mount();

      await vi.waitFor(() => {
        expect(alertText()).toBe(sharedTh['error.roleNotAllowedHere']);
      });
      expect(alertText()).not.toContain('roleNotAllowedHere');
    });

    it('shows server sentence when transfers read fails with unknown code and message', async () => {
      const serverSentence = 'ข้อมูลการโอนเงินไม่ถูกต้อง';
      server.transfersRefusal = {
        status: 400,
        body: { error: { code: 'unknownTransferCode', message: serverSentence } },
      };
      mount();

      await vi.waitFor(() => {
        expect(alertText()).toBe(serverSentence);
      });
      expect(alertText()).not.toContain('unknownTransferCode');
      expect(alertText()).not.toBe(sharedTh['error.serverError']);
    });
  });
});
