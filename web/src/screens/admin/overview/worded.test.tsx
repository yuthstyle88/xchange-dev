import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import { Overview } from './Overview';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { sharedSession } from '../../../shared/session/session';
import { meOf } from '../../../test-support/me';
import type { AdminMenuProps } from '../menus';

const EXPORTER_ME = {
  ...meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner' }),
  permissions: ['ReportsExport'],
};

function dummyProps(overrides?: Partial<AdminMenuProps>): AdminMenuProps {
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [branch] = createSignal('MAIN');
  const [shown] = createSignal(true);
  return {
    period,
    branch,
    shown,
    ...overrides,
  };
}

describe('WE-17: screens/admin/overview/ worded error tests (web-worded-error.md)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;

  beforeEach(() => {
    sharedSession().updateMe?.(EXPORTER_ME as any);
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
  });

  afterEach(async () => {
    closeDialog();
    if (dispose) {
      dispose();
      dispose = null;
    }
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  }

  describe('Catch 1: loadData', () => {
    // (1) a network TypeError shows th['error.network'], and after setLang('en') shows en['error.network']
    it('(1) network TypeError shows th error.network and en error.network after setLang', async () => {
      vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')));

      // Thai (default)
      setLang('th');
      dispose = render(() => <Overview {...dummyProps()} />, container);
      await settle();

      expect(container.textContent).toContain(th['error.network']);
      expect(container.textContent).not.toContain('Failed to fetch');
      expect(container.textContent).not.toContain('TypeError');

      dispose();
      dispose = null;
      container.innerHTML = '';

      // English
      setLang('en');
      dispose = render(() => <Overview {...dummyProps()} />, container);
      await settle();

      expect(container.textContent).toContain(en['error.network']);
      expect(container.textContent).not.toContain('Failed to fetch');
      expect(container.textContent).not.toContain('TypeError');
    });

    // (2) a 500 refusal shows th['error.serverError'] even with a keyed code
    it('(2) a 500 refusal shows th error.serverError', async () => {
      vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => {
        return new Response(
          JSON.stringify({
            error: {
              code: 'roleNotAllowedHere',
              message: 'Database connection failed',
            },
          }),
          {
            status: 500,
            headers: { 'content-type': 'application/json' },
          },
        );
      }));

      setLang('th');
      dispose = render(() => <Overview {...dummyProps()} />, container);
      await settle();

      expect(container.textContent).toContain(th['error.serverError']);
      expect(container.textContent).not.toContain('Database connection failed');
      expect(container.textContent).not.toContain(th['error.roleNotAllowedHere']);
    });

    // (3) a refusal with a code that has no shared key and a server message shows that message (pins W10 (4))
    it('(3) a refusal with an unkeyed code and a server message shows that message', async () => {
      const serverSentence = 'ข้อมูลสรุปของสาขานี้ยังไม่ได้รับการประมวลผล';
      vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => {
        return new Response(
          JSON.stringify({
            error: {
              code: 'overviewBranchNotYetProcessed',
              message: serverSentence,
            },
          }),
          {
            status: 400,
            headers: { 'content-type': 'application/json' },
          },
        );
      }));

      setLang('th');
      dispose = render(() => <Overview {...dummyProps()} />, container);
      await settle();

      expect(container.textContent).toContain(serverSentence);
      expect(container.textContent).not.toContain(th['error.serverError']);
    });
  });

  describe('Catch 2: exportLiveCsv', () => {
    const OVERVIEW_OK = {
      version: 1,
      changed: true,
      data: {
        period: { from: '2026-10-01', to: '2026-10-31' },
        totals: {
          volume: '100.00',
          buyPrincipal: '100.00',
          sellPrincipal: '0.00',
          buyCount: 1,
          sellCount: 0,
          buyShare: '100',
          sellShare: '0',
          received: '0.00',
          paid: '100.00',
          net: '-100.00',
          fees: '0.00',
          completedCount: 1,
          cancelledCount: 0,
          pendingCount: 0,
        },
        latest: [],
        status: { shiftOpen: true, pendingCount: 0, openShifts: [{ shiftId: 'SH-1', tellerId: 'U-1', tellerName: 'Staff', openedAt: '2026-10-06T01:00:00Z' }] },
      },
    };

    // (1) network TypeError on export shows th error.network and en error.network
    it('(1) network TypeError on export shows th error.network and en error.network', async () => {
      vi.stubGlobal('fetch', vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes('/api/v3/views/admin_overview')) {
          return new Response(JSON.stringify(OVERVIEW_OK), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          });
        }
        // Export call fails with TypeError
        throw new TypeError('Failed to fetch');
      }));

      setLang('th');
      dispose = render(
        () => (
          <>
            <Overview {...dummyProps()} />
            <Dialog />
            <Toast />
          </>
        ),
        container,
      );
      await settle();

      const exportBtn = container.querySelector<HTMLButtonElement>('.btn-export');
      expect(exportBtn).not.toBeNull();
      exportBtn!.click();
      await settle();

      const banner = container.querySelector('.error-banner');
      expect(banner?.textContent).toContain(th['error.network']);
      expect(banner?.textContent).not.toContain('Failed to fetch');

      dispose();
      dispose = null;
      container.innerHTML = '';

      // Test English
      setLang('en');
      dispose = render(
        () => (
          <>
            <Overview {...dummyProps()} />
            <Dialog />
            <Toast />
          </>
        ),
        container,
      );
      await settle();

      const exportBtnEn = container.querySelector<HTMLButtonElement>('.btn-export');
      exportBtnEn!.click();
      await settle();

      const bannerEn = container.querySelector('.error-banner');
      expect(bannerEn?.textContent).toContain(en['error.network']);
      expect(bannerEn?.textContent).not.toContain('Failed to fetch');
    });

    // (2) 500 on export shows th error.serverError
    it('(2) a 500 refusal on export shows th error.serverError', async () => {
      vi.stubGlobal('fetch', vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes('/api/v3/views/admin_overview')) {
          return new Response(JSON.stringify(OVERVIEW_OK), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          });
        }
        return new Response(
          JSON.stringify({
            error: {
              code: 'roleNotAllowedHere',
              message: 'Export service failure 500',
            },
          }),
          {
            status: 500,
            headers: { 'content-type': 'application/json' },
          },
        );
      }));

      setLang('th');
      dispose = render(
        () => (
          <>
            <Overview {...dummyProps()} />
            <Dialog />
            <Toast />
          </>
        ),
        container,
      );
      await settle();

      const exportBtn = container.querySelector<HTMLButtonElement>('.btn-export');
      exportBtn!.click();
      await settle();

      const banner = container.querySelector('.error-banner');
      expect(banner?.textContent).toContain(th['error.serverError']);
      expect(banner?.textContent).not.toContain('Export service failure 500');
      expect(banner?.textContent).not.toContain(th['error.roleNotAllowedHere']);
    });

    // (3) unkeyed code with server message on export shows server message
    it('(3) an unkeyed refusal on export shows server message', async () => {
      const serverSentence = 'ไม่อนุญาตให้ดาวน์โหลดรายงานนี้ในขณะนี้';
      vi.stubGlobal('fetch', vi.fn().mockImplementation(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes('/api/v3/views/admin_overview')) {
          return new Response(JSON.stringify(OVERVIEW_OK), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          });
        }
        return new Response(
          JSON.stringify({
            error: {
              code: 'exportTemporarilyBlocked',
              message: serverSentence,
            },
          }),
          {
            status: 400,
            headers: { 'content-type': 'application/json' },
          },
        );
      }));

      setLang('th');
      dispose = render(
        () => (
          <>
            <Overview {...dummyProps()} />
            <Dialog />
            <Toast />
          </>
        ),
        container,
      );
      await settle();

      const exportBtn = container.querySelector<HTMLButtonElement>('.btn-export');
      exportBtn!.click();
      await settle();

      const banner = container.querySelector('.error-banner');
      expect(banner?.textContent).toContain(serverSentence);
      expect(banner?.textContent).not.toContain(th['error.serverError']);
    });
  });

  describe('Criterion 3: Branch switch clears stale data', () => {
    it('after branch switch, a failed read does not leave the previous branch data showing', async () => {
      const [currentBranch, setCurrentBranch] = createSignal('BRANCH-A');

      vi.stubGlobal('fetch', vi.fn().mockImplementation(async () => {
        const b = currentBranch();
        if (b === 'BRANCH-A') {
          return new Response(JSON.stringify({
            version: 1,
            changed: true,
            data: {
              period: { from: '2026-10-01', to: '2026-10-31' },
              totals: {
                volume: '999,999.00',
                buyPrincipal: '999,999.00',
                sellPrincipal: '0.00',
                buyCount: 1,
                sellCount: 0,
                buyShare: '100',
                sellShare: '0',
                received: '0.00',
                paid: '999,999.00',
                net: '-999,999.00',
                fees: '0.00',
                completedCount: 1,
                cancelledCount: 0,
                pendingCount: 0,
              },
              latest: [],
              status: { shiftOpen: true, pendingCount: 0, openShifts: [{ shiftId: 'SH-1', tellerId: 'U-1', tellerName: 'Staff A', openedAt: '2026-10-06T01:00:00Z' }] },
            },
          }), { status: 200, headers: { 'content-type': 'application/json' } });
        }
        return new Response(JSON.stringify({
          error: { code: 'serverError', message: 'Branch B failed' },
        }), { status: 500, headers: { 'content-type': 'application/json' } });
      }));

      dispose = render(() => <Overview {...dummyProps({ branch: currentBranch })} />, container);
      await settle();

      expect(container.textContent).toContain('999,999.00');

      // Switch to BRANCH-B which fails
      setCurrentBranch('BRANCH-B');
      await settle();

      expect(container.textContent).toContain(th['error.serverError']);
      expect(container.textContent).not.toContain('999,999.00');
    });
  });
});
