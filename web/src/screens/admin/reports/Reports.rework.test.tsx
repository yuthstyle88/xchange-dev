import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { en } from '../../../shared/i18n/en';
import { th } from '../../../shared/i18n/th';
import { en as reportsEn, th as reportsTh } from './i18n';
import { Reports } from './Reports';

interface RefusalMock {
  status: number;
  body?: unknown;
  rawText?: string;
  networkError?: boolean;
}

describe('Admin reports failed reads show worded errors (#1697)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let branchesRefusal: RefusalMock | null = null;
  let employeeTxRefusal: RefusalMock | null = null;

  const validReportData = {
    period: { from: '2026-10-05', to: '2026-10-06' },
    stats: { received: '100.00', paid: '50.00', fees: '10.00', bankFees: '2.00' },
    byChannel: [],
    byCurrency: [],
    byEmployee: [
      {
        staff: 'anan',
        count: 1,
        completed: 1,
        pending: 0,
        cancelled: 0,
        received: '100.00',
        paid: '50.00',
        net: '50.00',
        fees: '10.00',
        bankFees: '2.00',
        methods: [],
        currencies: [],
      },
    ],
    employeeTotals: { people: 1, completed: 1, received: '100.00', paid: '50.00', net: '50.00' },
    reference: { referenced: 0, unreferenced: 0, value: '0.00', spread: '0.00', percent: '0.00' },
  };

  beforeEach(() => {
    setLang('th');
    branchesRefusal = null;
    employeeTxRefusal = null;
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown) => {
        const url = String(input);
        if (url.includes('/api/v3/views/admin_branches') && branchesRefusal) {
          if (branchesRefusal.networkError) {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
          if (branchesRefusal.rawText !== undefined) {
            return Promise.resolve(
              new Response(branchesRefusal.rawText, {
                status: branchesRefusal.status,
                headers: { 'content-type': 'text/html' },
              }),
            );
          }
          return Promise.resolve(
            new Response(JSON.stringify(branchesRefusal.body), {
              status: branchesRefusal.status,
              headers: { 'content-type': 'application/json' },
            }),
          );
        }
        if (url.includes('/api/v3/views/admin_transactions') && employeeTxRefusal) {
          if (employeeTxRefusal.networkError) {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
          if (employeeTxRefusal.rawText !== undefined) {
            return Promise.resolve(
              new Response(employeeTxRefusal.rawText, {
                status: employeeTxRefusal.status,
                headers: { 'content-type': 'text/html' },
              }),
            );
          }
          return Promise.resolve(
            new Response(JSON.stringify(employeeTxRefusal.body), {
              status: employeeTxRefusal.status,
              headers: { 'content-type': 'application/json' },
            }),
          );
        }
        if (url.includes('/api/v3/views/admin_reports')) {
          const u = new URL(url, 'http://desk.test');
          const since = u.searchParams.get('since');
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: since !== '1',
                data: since !== '1' ? validReportData : undefined,
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
        }
        if (url.includes('/api/v3/views/admin_branches')) {
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: {
                  branches: [
                    { id: 'MAIN', name: 'Silom #2' },
                    { id: 'B9', name: 'Ari' },
                  ],
                },
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
            ),
          );
        }
        return Promise.resolve(
          new Response(JSON.stringify({ version: 1, changed: false }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
        );
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(): HTMLElement {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-05', to: '2026-10-06' });
    const [shown] = createSignal(true);
    dispose = render(
      () => <Reports branch={branch} period={period} shown={shown} />,
      root,
    );
    return root;
  }

  describe('Row 1: admin_branches failure (Reports.tsx:82)', () => {
    it('500 serverError shows th error.serverError and not raw message', async () => {
      branchesRefusal = {
        status: 500,
        body: {
          error: {
            code: 'serverError',
            message: 'RAW-branch-fail-500',
          },
        },
      };
      mount();
      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(th['error.serverError']);
      expect(alertText).not.toContain('RAW-branch-fail-500');
    });

    it('500 serverError in en shows en error.serverError and not raw message', async () => {
      setLang('en');
      branchesRefusal = {
        status: 500,
        body: {
          error: {
            code: 'serverError',
            message: 'RAW-branch-fail-500',
          },
        },
      };
      mount();
      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(en['error.serverError']);
      expect(alertText).not.toContain('RAW-branch-fail-500');
    });

    it('refusal code (roleNotAllowed) shows worded error through errorText', async () => {
      branchesRefusal = {
        status: 403,
        body: {
          error: {
            code: 'roleNotAllowed',
            message: 'RAW-branch-refused',
          },
        },
      };
      mount();
      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(th['error.roleNotAllowed']);
      expect(alertText).not.toContain('RAW-branch-refused');
    });

    it('unknown refusal code shows generic worded message, never raw code or err.message', async () => {
      branchesRefusal = {
        status: 400,
        body: {
          error: {
            code: 'somethingNew',
            message: 'RAW-branch-unknown-code',
          },
        },
      };
      mount();
      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(reportsTh.loadFailed);
      expect(alertText).not.toContain('somethingNew');
      expect(alertText).not.toContain('RAW-branch-unknown-code');
    });

    it('unknown refusal code in en shows en generic worded message', async () => {
      setLang('en');
      branchesRefusal = {
        status: 400,
        body: {
          error: {
            code: 'somethingNew',
            message: 'RAW-branch-unknown-code',
          },
        },
      };
      mount();
      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(reportsEn.loadFailed);
      expect(alertText).not.toContain('somethingNew');
      expect(alertText).not.toContain('RAW-branch-unknown-code');
    });

    it('non-JSON 5xx (httpError) shows generic worded message, never raw html/body', async () => {
      branchesRefusal = {
        status: 502,
        rawText: '<html>502 Bad Gateway</html>',
      };
      mount();
      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(reportsTh.loadFailed);
      expect(alertText).not.toContain('502 Bad Gateway');
      expect(alertText).not.toContain('httpError');
    });

    it('network failure (TypeError) shows generic worded message, never TypeError or raw message', async () => {
      branchesRefusal = {
        status: 0,
        networkError: true,
      };
      mount();
      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(reportsTh.loadFailed);
      expect(alertText).not.toContain('TypeError');
      expect(alertText).not.toContain('Failed to fetch');
    });
  });

  describe('Row 2: employee drill-down failure (Reports.tsx:136)', () => {
    it('500 serverError on employee drill-down shows th error.serverError and not raw message', async () => {
      employeeTxRefusal = {
        status: 500,
        body: {
          error: {
            code: 'serverError',
            message: 'RAW-employee-fail-500',
          },
        },
      };
      mount();
      // Wait for initial render with reports data and details element
      await vi.waitFor(() => {
        const details = root.querySelector('details');
        expect(details).not.toBeNull();
      });

      const details = root.querySelector('details')!;
      details.open = true;
      details.dispatchEvent(new Event('toggle'));

      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(th['error.serverError']);
      expect(alertText).not.toContain('RAW-employee-fail-500');
    });

    it('500 serverError on employee drill-down in en shows en error.serverError', async () => {
      setLang('en');
      employeeTxRefusal = {
        status: 500,
        body: {
          error: {
            code: 'serverError',
            message: 'RAW-employee-fail-500',
          },
        },
      };
      mount();
      await vi.waitFor(() => {
        const details = root.querySelector('details');
        expect(details).not.toBeNull();
      });

      const details = root.querySelector('details')!;
      details.open = true;
      details.dispatchEvent(new Event('toggle'));

      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(en['error.serverError']);
      expect(alertText).not.toContain('RAW-employee-fail-500');
    });

    it('refusal code (roleNotAllowed) on employee drill-down shows worded error', async () => {
      employeeTxRefusal = {
        status: 403,
        body: {
          error: {
            code: 'roleNotAllowed',
            message: 'RAW-employee-refused',
          },
        },
      };
      mount();
      await vi.waitFor(() => {
        const details = root.querySelector('details');
        expect(details).not.toBeNull();
      });

      const details = root.querySelector('details')!;
      details.open = true;
      details.dispatchEvent(new Event('toggle'));

      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(th['error.roleNotAllowed']);
      expect(alertText).not.toContain('RAW-employee-refused');
    });

    it('unknown refusal code on employee drill-down shows generic worded message, never raw code or err.message', async () => {
      employeeTxRefusal = {
        status: 400,
        body: {
          error: {
            code: 'somethingNew',
            message: 'RAW-employee-unknown-code',
          },
        },
      };
      mount();
      await vi.waitFor(() => {
        const details = root.querySelector('details');
        expect(details).not.toBeNull();
      });

      const details = root.querySelector('details')!;
      details.open = true;
      details.dispatchEvent(new Event('toggle'));

      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(reportsTh.loadFailed);
      expect(alertText).not.toContain('somethingNew');
      expect(alertText).not.toContain('RAW-employee-unknown-code');
    });

    it('unknown refusal code on employee drill-down in en shows en generic worded message', async () => {
      setLang('en');
      employeeTxRefusal = {
        status: 400,
        body: {
          error: {
            code: 'somethingNew',
            message: 'RAW-employee-unknown-code',
          },
        },
      };
      mount();
      await vi.waitFor(() => {
        const details = root.querySelector('details');
        expect(details).not.toBeNull();
      });

      const details = root.querySelector('details')!;
      details.open = true;
      details.dispatchEvent(new Event('toggle'));

      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(reportsEn.loadFailed);
      expect(alertText).not.toContain('somethingNew');
      expect(alertText).not.toContain('RAW-employee-unknown-code');
    });

    it('non-JSON 5xx (httpError) on employee drill-down shows generic worded message, never raw html/body', async () => {
      employeeTxRefusal = {
        status: 502,
        rawText: '<html>502 Bad Gateway</html>',
      };
      mount();
      await vi.waitFor(() => {
        const details = root.querySelector('details');
        expect(details).not.toBeNull();
      });

      const details = root.querySelector('details')!;
      details.open = true;
      details.dispatchEvent(new Event('toggle'));

      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(reportsTh.loadFailed);
      expect(alertText).not.toContain('502 Bad Gateway');
      expect(alertText).not.toContain('httpError');
    });

    it('network failure (TypeError) on employee drill-down shows generic worded message, never TypeError or raw message', async () => {
      employeeTxRefusal = {
        status: 0,
        networkError: true,
      };
      mount();
      await vi.waitFor(() => {
        const details = root.querySelector('details');
        expect(details).not.toBeNull();
      });

      const details = root.querySelector('details')!;
      details.open = true;
      details.dispatchEvent(new Event('toggle'));

      await vi.waitFor(() => {
        const alert = root.querySelector('[role="alert"]');
        expect(alert).not.toBeNull();
        expect(alert!.textContent).toBeTruthy();
      });
      const alertText = root.querySelector('[role="alert"]')!.textContent!;
      expect(alertText).toContain(reportsTh.loadFailed);
      expect(alertText).not.toContain('TypeError');
      expect(alertText).not.toContain('Failed to fetch');
    });
  });
});
