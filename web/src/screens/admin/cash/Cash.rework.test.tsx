import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { Cash } from './Cash';

const CASH = '/api/v3/views/admin_cash';
const MOVES = '/api/v3/views/admin_cash_movements';
const SHIFTS = '/api/v3/views/admin_shifts';

const MOCK_CASH = {
  version: 1,
  shift: {
    id: 'SH-01',
    openedAt: '2026-10-14T08:00:00.000Z',
    staff: 'teller1',
    status: 'OPEN',
  },
  stats: {
    opening: '100000.00',
    incoming: '5000.00',
    outgoing: '2000.00',
    remaining: '103000.00',
  },
  reference: {
    spread: '500.00',
    percent: '1.20',
    withReference: 1,
    withoutReference: 0,
  },
  currencies: [
    { code: 'THB', opening: '100000.00', net: '3000.00', expected: '103000.00' },
  ],
};

const MOCK_MOVES = [
  {
    id: 'M-1',
    shiftId: 'SH-01',
    at: '2026-10-14T09:00:00.000Z',
    type: 'IN',
    amount: '1000.00',
    currency: 'THB',
    reason: 'เติมลิ้นชัก',
  },
];

const MOCK_SHIFTS = [
  {
    id: 'SH-PAST-99',
    openedAt: '2026-10-14T08:00:00.000Z',
    closedAt: null,
    openedBy: 'teller1',
    status: 'OPEN',
    difference: [],
  },
];

interface Reply {
  status: number;
  body: unknown;
}

class TestServer {
  version = 1;
  movesRefusal: Reply | 'network' | null = null;
  shiftsRefusal: Reply | 'network' | null = null;
  moves = MOCK_MOVES;
  shifts = MOCK_SHIFTS;

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(rawUrl, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();

    const json = (reply: Reply): Response =>
      new Response(JSON.stringify(reply.body), {
        status: reply.status,
        headers: { 'content-type': 'application/json' },
      });

    if (url.pathname === '/api/v3/views/me') {
      return json({
        status: 200,
        body: { version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN' } },
      });
    }

    if (method === 'GET' && url.pathname === CASH) {
      return json({ status: 200, body: { version: this.version, changed: true, data: MOCK_CASH } });
    }

    if (method === 'GET' && url.pathname === MOVES) {
      if (this.movesRefusal === 'network') throw new TypeError('Failed to fetch');
      if (this.movesRefusal) return json(this.movesRefusal);
      return json({ status: 200, body: { version: this.version, changed: true, data: this.moves } });
    }

    if (method === 'GET' && url.pathname === SHIFTS) {
      if (this.shiftsRefusal === 'network') throw new TypeError('Failed to fetch');
      if (this.shiftsRefusal) return json(this.shiftsRefusal);
      return json({ status: 200, body: { version: this.version, changed: true, data: this.shifts } });
    }

    return json({ status: 200, body: { version: 1, changed: false } });
  };
}

describe('Admin Cash screen worded errors for failed reads (#1694)', () => {
  let server: TestServer;
  let root: HTMLElement;
  let dispose: (() => void) | undefined;

  function mount(branch = 'MAIN'): { el: HTMLElement; setBranch: (b: string) => void } {
    const [b, setBranch] = createSignal(branch);
    const [period] = createSignal({ from: '2026-10-14', to: '2026-10-14' });
    const [shown] = createSignal(true);

    root = document.createElement('div');
    document.body.appendChild(root);
    dispose = render(
      () => (
        <>
          <Cash branch={b} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ),
      root,
    );
    return { el: root, setBranch };
  }

  function errorBannerText(): string {
    const el = root.querySelector('.error-banner, [role="alert"]');
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

  // Row 1: web/src/screens/admin/cash/Cash.tsx:83 — movements fail: empty table, no message -> worded error
  describe('Row 1: movements read failure shows worded error', () => {
    it('shows worded error when movements read fails with 500 (network failure)', async () => {
      server.movesRefusal = 'network';
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.network']);
      });
    });

    it('shows worded error when movements read fails with known refusal code', async () => {
      server.movesRefusal = {
        status: 403,
        body: { error: { code: 'roleNotAllowedHere', message: 'raw refusal' } },
      };
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.roleNotAllowedHere']);
      });
      expect(errorBannerText()).not.toContain('raw refusal');
      expect(errorBannerText()).not.toContain('roleNotAllowedHere');
    });

    it('shows generic worded error when movements read fails with unknown code, never raw code or err.message', async () => {
      server.movesRefusal = {
        status: 400,
        body: { error: { code: 'unknownMoveCode123', message: 'RAW_UNWORDED_MSG' } },
      };
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe('RAW_UNWORDED_MSG');
      });
      expect(errorBannerText()).not.toContain('unknownMoveCode123');
    });

    it('shows English worded error when language is en and movements read fails', async () => {
      setLang('en');
      server.movesRefusal = {
        status: 403,
        body: { error: { code: 'roleNotAllowedHere', message: 'raw' } },
      };
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedEn['error.roleNotAllowedHere']);
      });
    });

    it('clears movements and shows worded error when subsequent movements read fails on switch', async () => {
      const { setBranch } = mount('MAIN');
      // Wait for initial load
      await vi.waitFor(() => {
        expect(root.textContent).toContain('เติมลิ้นชัก');
      });
      expect(errorBannerText()).toBe('');

      // Now switch branch and simulate movements read failure
      server.movesRefusal = {
        status: 500,
        body: { error: { code: 'serverError', message: '500 error' } },
      };
      setBranch('B2');

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.serverError']);
      });
      // Movements table is cleared
      expect(root.textContent).not.toContain('เติมลิ้นชัก');
    });
  });

  // Row 2: web/src/screens/admin/cash/Cash.tsx:95 — shifts fail: empty table, no message -> worded error
  describe('Row 2: shifts read failure shows worded error', () => {
    it('shows worded error when shifts read fails with 500 (network failure)', async () => {
      server.shiftsRefusal = 'network';
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.network']);
      });
    });

    it('shows worded error when shifts read fails with known refusal code', async () => {
      server.shiftsRefusal = {
        status: 403,
        body: { error: { code: 'roleNotAllowedHere', message: 'raw' } },
      };
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.roleNotAllowedHere']);
      });
      expect(errorBannerText()).not.toContain('roleNotAllowedHere');
    });

    it('shows generic worded error when shifts read fails with unknown code, never raw code or err.message', async () => {
      server.shiftsRefusal = {
        status: 400,
        body: { error: { code: 'unknownShiftCode999', message: 'RAW_SHIFT_MSG' } },
      };
      mount();

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe('RAW_SHIFT_MSG');
      });
      expect(errorBannerText()).not.toContain('unknownShiftCode999');
    });

    it('clears shifts and shows worded error when subsequent shifts read fails on switch', async () => {
      const { setBranch } = mount('MAIN');
      // Wait for initial load
      await vi.waitFor(() => {
        expect(root.textContent).toContain('SH-PAST-99');
      });
      expect(errorBannerText()).toBe('');

      // Now switch branch and simulate shifts read failure
      server.shiftsRefusal = {
        status: 500,
        body: { error: { code: 'serverError', message: '500 error' } },
      };
      setBranch('B2');

      await vi.waitFor(() => {
        expect(errorBannerText()).toBe(sharedTh['error.serverError']);
      });
      // Shifts table is cleared
      expect(root.textContent).not.toContain('SH-PAST-99');
      expect(root.querySelector('.shifts-table')).toBeNull();
    });
  });
});
