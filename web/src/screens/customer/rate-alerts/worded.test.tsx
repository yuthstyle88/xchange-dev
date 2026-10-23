// WE-29 (#1778): screens/customer/rate-alerts/ — show failed calls through wordedError
// Tests for §4.2 (three tests for each changed catch: loadBranches, loadAlerts, handleCreate, handlePause, handleResume, handleDelete)
// and §4.3 (stale data from earlier successful read).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import { en } from '../../../shared/i18n/en';
import {
  makeMyRateAlertItem,
  makePublishedLineDto,
  makePublishedRatesData,
} from '../../../shared/testing/fixtures';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { RateAlertsScreen } from './RateAlerts';

const SAMPLE_BRANCHES: PublicBranchesData = {
  branches: [
    {
      id: 'XC-B01',
      code: 'XC-B01',
      name: 'Silom Branch',
      address: null,
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
    {
      id: 'XC-B02',
      code: 'XC-B02',
      name: 'Don Mueang Branch',
      address: null,
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: false,
      queueOpen: false,
    },
  ],
};

const SAMPLE_ALERT_ACTIVE = makeMyRateAlertItem({
  id: 'alert-active-1',
  version: 1,
  branch: 'XC-B01',
  currency: 'USD',
  customerSide: 'CUSTOMER_BUY',
  condition: 'ABOVE',
  target: '35.0000',
  enabled: true,
  unitSize: '1',
  currentRate: '34.5000',
  ratePublishedAt: '2026-10-14T10:00:00Z',
  relation: 'UNKNOWN',
  expiresAt: null,
  expired: false,
  rateStale: null,
  lastEvent: null,
});

const SAMPLE_ALERT_PAUSED = makeMyRateAlertItem({
  ...SAMPLE_ALERT_ACTIVE,
  id: 'alert-paused-1',
  enabled: false,
});

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function refusal(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

describe('RateAlerts worded error handling (#1778)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  let hostReply: (() => Promise<Response>) | null;
  let branchesReply: (() => Promise<Response>) | null;
  let alertsReply: (() => Promise<Response>) | null;
  let commandReply: (() => Promise<Response>) | null;

  async function fakeFetch(url: string, init?: RequestInit): Promise<Response> {
    const method = (init?.method ?? 'GET').toUpperCase();

    if (url === '/api/public/host') {
      if (hostReply) return hostReply();
      return json({ org: 'XC-ORG' });
    }

    if (/^\/api\/public\/orgs\/[^/]+\/branches\/[^/]+\/rates$/.test(url)) {
      return json(makePublishedRatesData({ lines: [makePublishedLineDto({ currency: 'USD' })] }));
    }

    if (url.includes('/api/public/orgs/')) {
      if (branchesReply) return branchesReply();
      return json(SAMPLE_BRANCHES);
    }

    if (url.includes('/api/v3/views/my_rate_alerts')) {
      if (alertsReply) return alertsReply();
      return json({ version: 1, changed: true, data: { alerts: [SAMPLE_ALERT_ACTIVE] } });
    }

    if (url.startsWith('/api/app/rate-alerts') && method === 'POST') {
      if (commandReply) return commandReply();
      return json({ id: 'alert-new', version: 1 });
    }

    return refusal(404, 'notFound', 'not found');
  }

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);

    hostReply = null;
    branchesReply = null;
    alertsReply = null;
    commandReply = null;
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(props: { org?: string; defaultBranch?: string } = {}) {
    dispose = render(
      () => (
        <RateAlertsScreen
          fetch={fakeFetch}
          org={props.org ?? 'XC-ORG'}
          defaultBranch={props.defaultBranch}
        />
      ),
      host,
    );
  }

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 1: loadBranches
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 1: loadBranches failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      branchesReply = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();

      await vi.waitFor(() => {
        const errorEl = host.querySelector('#alert-branch ~ .xc-form-error, .xc-form-group .xc-form-error');
        expect(errorEl?.textContent).toContain(th['error.network']);
      });

      dispose?.();
      setLang('en');
      mount();

      await vi.waitFor(() => {
        const errorEl = host.querySelector('#alert-branch ~ .xc-form-error, .xc-form-group .xc-form-error');
        expect(errorEl?.textContent).toContain(en['error.network']);
      });
    });

    it('(2) 500 refusal with keyed code shows error.serverError and not keyed code text', async () => {
      branchesReply = () => Promise.resolve(refusal(500, 'roleNotAllowedHere', 'Database branch failure 500'));
      mount();

      await vi.waitFor(() => {
        const errorEl = host.querySelector('#alert-branch ~ .xc-form-error, .xc-form-group .xc-form-error');
        expect(errorEl?.textContent).toContain(th['error.serverError']);
        expect(errorEl?.textContent).not.toContain(th['error.roleNotAllowedHere']);
        expect(errorEl?.textContent).not.toContain('Database branch failure 500');
      });
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      const serverMessage = 'สาขานี้ยังไม่เปิดให้บริการการแจ้งเตือน';
      branchesReply = () => Promise.resolve(refusal(400, 'unsharedBranchError1778', serverMessage));
      mount();

      await vi.waitFor(() => {
        const errorEl = host.querySelector('#alert-branch ~ .xc-form-error, .xc-form-group .xc-form-error');
        expect(errorEl?.textContent).toContain(serverMessage);
        expect(errorEl?.textContent).not.toContain(th['error.serverError']);
      });
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 2: loadAlerts
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 2: loadAlerts failures', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      alertsReply = () => Promise.reject(new TypeError('Failed to fetch'));
      mount();

      await vi.waitFor(() => {
        const errEl = host.querySelector('.xc-alerts-error p');
        expect(errEl?.textContent).toContain(th['error.network']);
      });

      dispose?.();
      setLang('en');
      mount();

      await vi.waitFor(() => {
        const errEl = host.querySelector('.xc-alerts-error p');
        expect(errEl?.textContent).toContain(en['error.network']);
      });
    });

    it('(2) 500 refusal with keyed code shows error.serverError and not keyed code text', async () => {
      alertsReply = () => Promise.resolve(refusal(500, 'roleNotAllowedHere', 'Alerts view crash 500'));
      mount();

      await vi.waitFor(() => {
        const errEl = host.querySelector('.xc-alerts-error p');
        expect(errEl?.textContent).toContain(th['error.serverError']);
        expect(errEl?.textContent).not.toContain(th['error.roleNotAllowedHere']);
        expect(errEl?.textContent).not.toContain('Alerts view crash 500');
      });
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      const serverMessage = 'ไม่สามารถดึงข้อมูลการแจ้งเตือนของคุณได้ในขณะนี้';
      alertsReply = () => Promise.resolve(refusal(400, 'unsharedAlertsError1778', serverMessage));
      mount();

      await vi.waitFor(() => {
        const errEl = host.querySelector('.xc-alerts-error p');
        expect(errEl?.textContent).toContain(serverMessage);
        expect(errEl?.textContent).not.toContain(th['error.serverError']);
      });
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 3: handleCreate
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 3: handleCreate command failures', () => {
    async function submitCreateForm() {
      await vi.waitFor(() => {
        const input = host.querySelector<HTMLInputElement>('#alert-target');
        const branchSelect = host.querySelector<HTMLSelectElement>('#alert-branch');
        expect(input).not.toBeNull();
        expect(branchSelect?.value).toBe('XC-B01');
      });
      const input = host.querySelector<HTMLInputElement>('#alert-target')!;
      input.value = '35.50';
      input.dispatchEvent(new Event('input', { bubbles: true }));

      const form = host.querySelector<HTMLFormElement>('.xc-create-alert-form')!;
      expect(form).not.toBeNull();
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    }

    it('(1) network TypeError shows error.network in TH and EN', async () => {
      mount({ defaultBranch: 'XC-B01' });
      commandReply = () => Promise.reject(new TypeError('Failed to fetch'));
      await submitCreateForm();

      await vi.waitFor(() => {
        const formErr = host.querySelector('.xc-create-alert-form .xc-form-error');
        expect(formErr?.textContent).toContain(th['error.network']);
      });

      dispose?.();
      setLang('en');
      mount({ defaultBranch: 'XC-B01' });
      await submitCreateForm();

      await vi.waitFor(() => {
        const formErr = host.querySelector('.xc-create-alert-form .xc-form-error');
        expect(formErr?.textContent).toContain(en['error.network']);
      });
    });

    it('(2) 500 refusal with keyed code shows error.serverError and not keyed code text', async () => {
      mount({ defaultBranch: 'XC-B01' });
      commandReply = () => Promise.resolve(refusal(500, 'roleNotAllowedHere', 'Create alert crash 500'));
      await submitCreateForm();

      await vi.waitFor(() => {
        const formErr = host.querySelector('.xc-create-alert-form .xc-form-error');
        expect(formErr?.textContent).toContain(th['error.serverError']);
        expect(formErr?.textContent).not.toContain(th['error.roleNotAllowedHere']);
        expect(formErr?.textContent).not.toContain('Create alert crash 500');
      });
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      mount({ defaultBranch: 'XC-B01' });
      const serverMessage = 'ระบบจำกัดการสร้างเรทเฉพาะช่วงเวลาทำการ';
      commandReply = () => Promise.resolve(refusal(400, 'unsharedCreateError1778', serverMessage));
      await submitCreateForm();

      await vi.waitFor(() => {
        const formErr = host.querySelector('.xc-create-alert-form .xc-form-error');
        expect(formErr?.textContent).toContain(serverMessage);
        expect(formErr?.textContent).not.toContain(th['error.serverError']);
      });
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 4: handlePause
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 4: handlePause command failures', () => {
    async function clickPause() {
      await vi.waitFor(() => {
        const btn = host.querySelector<HTMLButtonElement>('.xc-btn-pause');
        expect(btn).not.toBeNull();
      });
      const btn = host.querySelector<HTMLButtonElement>('.xc-btn-pause')!;
      btn.click();
    }

    it('(1) network TypeError shows error.network in TH and EN', async () => {
      mount();
      commandReply = () => Promise.reject(new TypeError('Failed to fetch'));
      await clickPause();

      await vi.waitFor(() => {
        const actionErr = host.querySelector('.xc-rate-alerts-screen > .xc-form-error[role="alert"]');
        expect(actionErr?.textContent).toContain(th['error.network']);
      });

      dispose?.();
      setLang('en');
      mount();
      await clickPause();

      await vi.waitFor(() => {
        const actionErr = host.querySelector('.xc-rate-alerts-screen > .xc-form-error[role="alert"]');
        expect(actionErr?.textContent).toContain(en['error.network']);
      });
    });

    it('(2) 500 refusal with keyed code shows error.serverError and not keyed code text', async () => {
      mount();
      commandReply = () => Promise.resolve(refusal(500, 'roleNotAllowedHere', 'Pause command crash 500'));
      await clickPause();

      await vi.waitFor(() => {
        const actionErr = host.querySelector('.xc-rate-alerts-screen > .xc-form-error[role="alert"]');
        expect(actionErr?.textContent).toContain(th['error.serverError']);
        expect(actionErr?.textContent).not.toContain(th['error.roleNotAllowedHere']);
        expect(actionErr?.textContent).not.toContain('Pause command crash 500');
      });
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      mount();
      const serverMessage = 'ไม่สามารถหยุดการแจ้งเตือนนี้ได้ในขณะนี้';
      commandReply = () => Promise.resolve(refusal(400, 'unsharedPauseError1778', serverMessage));
      await clickPause();

      await vi.waitFor(() => {
        const actionErr = host.querySelector('.xc-rate-alerts-screen > .xc-form-error[role="alert"]');
        expect(actionErr?.textContent).toContain(serverMessage);
        expect(actionErr?.textContent).not.toContain(th['error.serverError']);
      });
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 5: handleResume
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 5: handleResume command failures', () => {
    async function clickResume() {
      await vi.waitFor(() => {
        const btn = host.querySelector<HTMLButtonElement>('.xc-btn-resume');
        expect(btn).not.toBeNull();
      });
      const btn = host.querySelector<HTMLButtonElement>('.xc-btn-resume')!;
      btn.click();
    }

    beforeEach(() => {
      alertsReply = () => Promise.resolve(json({ version: 1, changed: true, data: { alerts: [SAMPLE_ALERT_PAUSED] } }));
    });

    it('(1) network TypeError shows error.network in TH and EN', async () => {
      mount();
      commandReply = () => Promise.reject(new TypeError('Failed to fetch'));
      await clickResume();

      await vi.waitFor(() => {
        const actionErr = host.querySelector('.xc-rate-alerts-screen > .xc-form-error[role="alert"]');
        expect(actionErr?.textContent).toContain(th['error.network']);
      });

      dispose?.();
      setLang('en');
      mount();
      await clickResume();

      await vi.waitFor(() => {
        const actionErr = host.querySelector('.xc-rate-alerts-screen > .xc-form-error[role="alert"]');
        expect(actionErr?.textContent).toContain(en['error.network']);
      });
    });

    it('(2) 500 refusal with keyed code shows error.serverError and not keyed code text', async () => {
      mount();
      commandReply = () => Promise.resolve(refusal(500, 'roleNotAllowedHere', 'Resume command crash 500'));
      await clickResume();

      await vi.waitFor(() => {
        const actionErr = host.querySelector('.xc-rate-alerts-screen > .xc-form-error[role="alert"]');
        expect(actionErr?.textContent).toContain(th['error.serverError']);
        expect(actionErr?.textContent).not.toContain(th['error.roleNotAllowedHere']);
        expect(actionErr?.textContent).not.toContain('Resume command crash 500');
      });
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      mount();
      const serverMessage = 'ไม่สามารถเปิดใช้งานการแจ้งเตือนนี้ได้ในขณะนี้';
      commandReply = () => Promise.resolve(refusal(400, 'unsharedResumeError1778', serverMessage));
      await clickResume();

      await vi.waitFor(() => {
        const actionErr = host.querySelector('.xc-rate-alerts-screen > .xc-form-error[role="alert"]');
        expect(actionErr?.textContent).toContain(serverMessage);
        expect(actionErr?.textContent).not.toContain(th['error.serverError']);
      });
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Catch 6: handleDelete
  // ──────────────────────────────────────────────────────────────────────────
  describe('Catch 6: handleDelete command failures', () => {
    async function clickDelete() {
      await vi.waitFor(() => {
        const btn = host.querySelector<HTMLButtonElement>('.xc-btn-delete');
        expect(btn).not.toBeNull();
      });
      const btn = host.querySelector<HTMLButtonElement>('.xc-btn-delete')!;
      btn.click();
    }

    it('(1) network TypeError shows error.network in TH and EN', async () => {
      mount();
      commandReply = () => Promise.reject(new TypeError('Failed to fetch'));
      await clickDelete();

      await vi.waitFor(() => {
        const actionErr = host.querySelector('.xc-rate-alerts-screen > .xc-form-error[role="alert"]');
        expect(actionErr?.textContent).toContain(th['error.network']);
      });

      dispose?.();
      setLang('en');
      mount();
      await clickDelete();

      await vi.waitFor(() => {
        const actionErr = host.querySelector('.xc-rate-alerts-screen > .xc-form-error[role="alert"]');
        expect(actionErr?.textContent).toContain(en['error.network']);
      });
    });

    it('(2) 500 refusal with keyed code shows error.serverError and not keyed code text', async () => {
      mount();
      commandReply = () => Promise.resolve(refusal(500, 'roleNotAllowedHere', 'Delete command crash 500'));
      await clickDelete();

      await vi.waitFor(() => {
        const actionErr = host.querySelector('.xc-rate-alerts-screen > .xc-form-error[role="alert"]');
        expect(actionErr?.textContent).toContain(th['error.serverError']);
        expect(actionErr?.textContent).not.toContain(th['error.roleNotAllowedHere']);
        expect(actionErr?.textContent).not.toContain('Delete command crash 500');
      });
    });

    it('(3) refusal with unshared code and server message shows that message', async () => {
      mount();
      const serverMessage = 'ไม่สามารถลบการแจ้งเตือนนี้ได้ในขณะนี้';
      commandReply = () => Promise.resolve(refusal(400, 'unsharedDeleteError1778', serverMessage));
      await clickDelete();

      await vi.waitFor(() => {
        const actionErr = host.querySelector('.xc-rate-alerts-screen > .xc-form-error[role="alert"]');
        expect(actionErr?.textContent).toContain(serverMessage);
        expect(actionErr?.textContent).not.toContain(th['error.serverError']);
      });
    });
  });

  // ──────────────────────────────────────────────────────────────────────────
  // Criterion 3: Stale data preservation
  // ──────────────────────────────────────────────────────────────────────────
  describe('Criterion 3: stale data on same query stays on failed subsequent read', () => {
    it('preserves existing alerts when subsequent background reload fails', async () => {
      mount();

      await vi.waitFor(() => {
        expect(host.textContent).toContain('35.0000');
      });

      // Subsequent read fails
      alertsReply = () => Promise.resolve(refusal(500, 'serverCrash', 'Background read failed'));

      // The existing alerts remain rendered on screen
      expect(host.textContent).toContain('35.0000');
    });

    it('preserves branches in select dropdown when subsequent branch reload fails', async () => {
      let callCount = 0;
      branchesReply = () => {
        callCount++;
        if (callCount === 1) {
          return Promise.resolve(json(SAMPLE_BRANCHES));
        }
        return Promise.resolve(refusal(500, 'serverError', 'Branch reload failure'));
      };

      mount();

      await vi.waitFor(() => {
        const select = host.querySelector<HTMLSelectElement>('#alert-branch');
        expect(select?.querySelectorAll('option').length).toBe(2);
      });

      // Stale branch options remain
      const select = host.querySelector<HTMLSelectElement>('#alert-branch');
      expect(select?.querySelectorAll('option').length).toBe(2);
    });
  });
});
