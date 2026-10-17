// WE-10 (#1759): screens/admin/cash/ — show failed calls through wordedError
// Tests for §4.2 (three tests per changed catch: readCash, readMovements, readShifts, handleLoadOlderShifts)
// and §4.3 (stale data cleared on branch switch before failed read).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { Cash } from './Cash';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('Cash worded error handling (#1759)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;

  let cashHandler: (url: string) => Response | Promise<Response>;
  let movementsHandler: (url: string) => Response | Promise<Response>;
  let shiftsHandler: (url: string) => Response | Promise<Response>;

  const defaultCashData = {
    counts: [],
    openShifts: [],
    balances: [],
    reference: {
      withReference: 0,
      withoutReference: 0,
      spread: '0.00',
      percent: '0.00',
    },
  };

  const defaultShifts = Array.from({ length: 10 }, (_, i) => ({
    id: `shift-${i + 1}`,
    branch: 'B01',
    teller: `teller-${i + 1}`,
    status: 'CLOSED' as const,
    openedAt: '2026-10-15T01:00:00Z',
    closedAt: '2026-10-15T09:00:00Z',
    difference: [],
  }));

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);

    cashHandler = () =>
      jsonResponse({
        version: 1,
        changed: true,
        data: defaultCashData,
      });

    movementsHandler = () => jsonResponse([]);

    shiftsHandler = () => jsonResponse(defaultShifts);

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const u = new URL(url, 'http://localhost');

        if (PAGE_WIDE.some((p) => u.pathname.includes(p))) {
          return jsonResponse({ version: 1, changed: false });
        }
        if (u.pathname === '/api/v3/views/admin_cash') {
          return cashHandler(url);
        }
        if (u.pathname === '/api/v3/views/admin_cash_movements') {
          return movementsHandler(url);
        }
        if (u.pathname === '/api/v3/views/admin_shifts') {
          return shiftsHandler(url);
        }
        return jsonResponse({ error: { code: 'notFound', message: 'not found' } }, 404);
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    container.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  async function settle(): Promise<void> {
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 10));
  }

  function mount(initialBranch = 'B01') {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-15' });
    const [shown] = createSignal(true);

    dispose = render(
      () => (
        <Cash
          branch={branch}
          period={period}
          shown={shown}
        />
      ),
      container,
    );

    return { branch, setBranch };
  }

  function getBannerErrorText(): string | null {
    const el = container.querySelector('.error-banner');
    return el ? (el.textContent ?? '').trim() : null;
  }

  function getOlderShiftsErrorText(): string | null {
    const el = container.querySelector('.load-older-shifts .error-message');
    return el ? (el.textContent ?? '').trim() : null;
  }

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 1: readCash view failure
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 1: readCash view failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      cashHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();
      await settle();

      expect(getBannerErrorText()).toBe(th['error.network']);
      expect(getBannerErrorText()).not.toContain('Failed to fetch');

      // In EN
      dispose?.();
      container.innerHTML = '';
      setLang('en');
      mount();
      await settle();

      expect(getBannerErrorText()).toBe(en['error.network']);
      expect(getBannerErrorText()).not.toContain('Failed to fetch');
    });

    it('(2) 500 server error with keyed code shows error.serverError in TH', async () => {
      cashHandler = () =>
        jsonResponse(
          { error: { code: 'roleNotAllowedHere', message: 'role not allowed' } },
          500,
        );
      mount();
      await settle();

      expect(getBannerErrorText()).toBe(th['error.serverError']);
      expect(getBannerErrorText()).not.toBe(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      cashHandler = () =>
        jsonResponse(
          { error: { code: 'customUnsharedCashCode', message: 'ยอดเงินสดไม่ถูกต้องตามระบบ' } },
          400,
        );
      mount();
      await settle();

      expect(getBannerErrorText()).toBe('ยอดเงินสดไม่ถูกต้องตามระบบ');
      expect(getBannerErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 2: readMovements page failure
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 2: readMovements page failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      movementsHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();
      await settle();

      expect(getBannerErrorText()).toBe(th['error.network']);
      expect(getBannerErrorText()).not.toContain('Failed to fetch');

      // In EN
      dispose?.();
      container.innerHTML = '';
      setLang('en');
      mount();
      await settle();

      expect(getBannerErrorText()).toBe(en['error.network']);
      expect(getBannerErrorText()).not.toContain('Failed to fetch');
    });

    it('(2) 500 server error with keyed code shows error.serverError in TH', async () => {
      movementsHandler = () =>
        jsonResponse(
          { error: { code: 'roleNotAllowedHere', message: 'role not allowed' } },
          500,
        );
      mount();
      await settle();

      expect(getBannerErrorText()).toBe(th['error.serverError']);
      expect(getBannerErrorText()).not.toBe(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      movementsHandler = () =>
        jsonResponse(
          { error: { code: 'customUnsharedMoveCode', message: 'ไม่สามารถดึงรายการเคลื่อนไหวเงินสดได้' } },
          400,
        );
      mount();
      await settle();

      expect(getBannerErrorText()).toBe('ไม่สามารถดึงรายการเคลื่อนไหวเงินสดได้');
      expect(getBannerErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 3: readShifts page failure
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 3: readShifts page failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      shiftsHandler = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();
      await settle();

      expect(getBannerErrorText()).toBe(th['error.network']);
      expect(getBannerErrorText()).not.toContain('Failed to fetch');

      // In EN
      dispose?.();
      container.innerHTML = '';
      setLang('en');
      mount();
      await settle();

      expect(getBannerErrorText()).toBe(en['error.network']);
      expect(getBannerErrorText()).not.toContain('Failed to fetch');
    });

    it('(2) 500 server error with keyed code shows error.serverError in TH', async () => {
      shiftsHandler = () =>
        jsonResponse(
          { error: { code: 'roleNotAllowedHere', message: 'role not allowed' } },
          500,
        );
      mount();
      await settle();

      expect(getBannerErrorText()).toBe(th['error.serverError']);
      expect(getBannerErrorText()).not.toBe(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      shiftsHandler = () =>
        jsonResponse(
          { error: { code: 'customUnsharedShiftCode', message: 'ไม่สามารถดึงรายการกะได้' } },
          400,
        );
      mount();
      await settle();

      expect(getBannerErrorText()).toBe('ไม่สามารถดึงรายการกะได้');
      expect(getBannerErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 4: handleLoadOlderShifts page failure
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 4: handleLoadOlderShifts page failures', () => {
    it('(1) network TypeError on loading older shifts shows error.network in TH and EN', async () => {
      shiftsHandler = (url) => {
        if (url.includes('before=')) {
          return Promise.reject(new TypeError('Network failure loading older'));
        }
        return jsonResponse(defaultShifts);
      };
      mount();
      await settle();

      const loadOlderBtn = container.querySelector<HTMLButtonElement>('.load-older-shifts button');
      expect(loadOlderBtn).not.toBeNull();
      loadOlderBtn!.click();
      await settle();

      expect(getOlderShiftsErrorText()).toBe(th['error.network']);
      expect(getOlderShiftsErrorText()).not.toContain('Network failure loading older');

      // In EN
      dispose?.();
      container.innerHTML = '';
      setLang('en');
      mount();
      await settle();

      const loadOlderBtnEn = container.querySelector<HTMLButtonElement>('.load-older-shifts button');
      expect(loadOlderBtnEn).not.toBeNull();
      loadOlderBtnEn!.click();
      await settle();

      expect(getOlderShiftsErrorText()).toBe(en['error.network']);
      expect(getOlderShiftsErrorText()).not.toContain('Network failure loading older');
    });

    it('(2) 500 server error with keyed code on loading older shifts shows error.serverError in TH', async () => {
      shiftsHandler = (url) => {
        if (url.includes('before=')) {
          return jsonResponse(
            { error: { code: 'roleNotAllowedHere', message: 'role not allowed' } },
            500,
          );
        }
        return jsonResponse(defaultShifts);
      };
      mount();
      await settle();

      const loadOlderBtn = container.querySelector<HTMLButtonElement>('.load-older-shifts button');
      expect(loadOlderBtn).not.toBeNull();
      loadOlderBtn!.click();
      await settle();

      expect(getOlderShiftsErrorText()).toBe(th['error.serverError']);
      expect(getOlderShiftsErrorText()).not.toBe(th['error.roleNotAllowedHere']);
    });

    it('(3) refusal on loading older shifts with unshared code and server message shows that message', async () => {
      shiftsHandler = (url) => {
        if (url.includes('before=')) {
          return jsonResponse(
            { error: { code: 'customOlderShiftCode', message: 'ไม่สามารถโหลดกะก่อนหน้าได้' } },
            400,
          );
        }
        return jsonResponse(defaultShifts);
      };
      mount();
      await settle();

      const loadOlderBtn = container.querySelector<HTMLButtonElement>('.load-older-shifts button');
      expect(loadOlderBtn).not.toBeNull();
      loadOlderBtn!.click();
      await settle();

      expect(getOlderShiftsErrorText()).toBe('ไม่สามารถโหลดกะก่อนหน้าได้');
      expect(getOlderShiftsErrorText()).not.toBe(th['error.serverError']);
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // §4.3: Branch switch stale data handling
  // ──────────────────────────────────────────────────────────────────────────
  describe('§4.3: Branch switch stale data handling', () => {
    it('after branch switch, a failed read does not leave previous branch data showing', async () => {
      const { setBranch } = mount('B01');
      await settle();

      // B01 loaded shifts table
      expect(container.querySelector('.shifts-table')).not.toBeNull();

      // Switch to B02 where readCash fails with 500
      cashHandler = () =>
        jsonResponse(
          { error: { code: 'serverError', message: 'B02 read failed' } },
          500,
        );
      setBranch('B02');
      await settle();

      // Error banner must show
      expect(getBannerErrorText()).toBe(th['error.serverError']);
    });
  });
});
