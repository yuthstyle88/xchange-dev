import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeMyRateAlertItem,
  makeMyRateAlertsData,
  makePublishedLineDto,
  makePublishedRatesData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { RateAlertsScreen } from './RateAlerts';

const BRANCH_FIXTURE = {
  id: 'br-1',
  code: 'MAIN',
  name: 'Siam',
  address: null,
  openingHours: null,
  timezone: 'Asia/Bangkok',
  bookingEnabled: true,
  queueOpen: true,
};

function mockBackend(options: { alerts?: ReturnType<typeof makeMyRateAlertItem>[] } = {}) {
  const alerts = options.alerts ?? [];
  const calls: Array<{ method: string; url: string; body: any }> = [];

  const fetchFn = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
    const method = (init?.method ?? 'GET').toUpperCase();
    let body: any = undefined;
    if (init?.body && typeof init.body === 'string') {
      try {
        body = JSON.parse(init.body);
      } catch {
        body = init.body;
      }
    }
    calls.push({ method, url, body });

    if (url === '/api/public/host') {
      return new Response(JSON.stringify({ org: 'XC-ORG' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (/^\/api\/public\/orgs\/[^/]+\/branches\/[^/]+\/rates$/.test(url)) {
      return new Response(
        JSON.stringify(
          makePublishedRatesData({ lines: [makePublishedLineDto({ currency: 'USD' })] }),
        ),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      );
    }

    if (url.includes('/branches')) {
      return new Response(JSON.stringify({ branches: [BRANCH_FIXTURE] }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    if (url.includes('/api/v3/views/my_rate_alerts')) {
      return new Response(
        JSON.stringify({
          version: 1,
          changed: true,
          data: makeMyRateAlertsData({ alerts }),
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }

    if (method === 'POST' && url === '/api/app/rate-alerts') {
      return new Response(JSON.stringify({ id: 'al-new', version: 1 }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    return new Response('{}', { status: 404 });
  });

  return { fetchFn, calls };
}

describe('RateAlerts branch-id behaviour (CB-1, CB-2)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('en');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
  });

  it('A1: the list holds br-1 / MAIN; no defaultBranch -> the branch select value is br-1; its option text is Siam (MAIN)', async () => {
    const { fetchFn } = mockBackend();
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      const select = host.querySelector('#alert-branch') as HTMLSelectElement | null;
      expect(select).not.toBeNull();
      expect(select?.value).toBe('br-1');
    });

    const select = host.querySelector('#alert-branch') as HTMLSelectElement;
    const option = select.querySelector('option');
    expect(option?.value).toBe('br-1');
    expect(option?.textContent?.trim()).toBe('Siam (MAIN)');
  });

  it('A2: A1; the user fills a valid alert and creates it -> the POST /api/app/rate-alerts body has branch: "br-1"', async () => {
    const { fetchFn, calls } = mockBackend();
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      const select = host.querySelector('#alert-branch') as HTMLSelectElement | null;
      expect(select?.value).toBe('br-1');
    });

    const targetInput = host.querySelector('#alert-target') as HTMLInputElement;
    targetInput.value = '35.00';
    targetInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = host.querySelector('button[type="submit"]') as HTMLButtonElement;
    submitBtn.click();

    await vi.waitFor(() => {
      const createCall = calls.find((c) => c.method === 'POST' && c.url === '/api/app/rate-alerts');
      expect(createCall).toBeDefined();
    });

    const createCall = calls.find((c) => c.method === 'POST' && c.url === '/api/app/rate-alerts');
    expect(createCall?.body.branch).toBe('br-1');
  });

  it('A3: an alert with branch: "br-1" -> its card badge shows MAIN, not br-1', async () => {
    const alert = makeMyRateAlertItem({
      id: 'al-1',
      branch: 'br-1',
      currency: 'USD',
      customerSide: 'CUSTOMER_BUY',
      condition: 'ABOVE',
      target: '35.00',
      enabled: true,
    });
    const { fetchFn } = mockBackend({ alerts: [alert] });
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      const badge = host.querySelector('.xc-alert-branch-badge');
      expect(badge).not.toBeNull();
      expect(badge?.textContent).toBe('MAIN');
    });
  });

  it('A4: an alert with branch: "br-9", which is not in the list -> its badge shows br-9 (as today)', async () => {
    const alert = makeMyRateAlertItem({
      id: 'al-2',
      branch: 'br-9',
      currency: 'USD',
      customerSide: 'CUSTOMER_BUY',
      condition: 'ABOVE',
      target: '35.00',
      enabled: true,
    });
    const { fetchFn } = mockBackend({ alerts: [alert] });
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      const badge = host.querySelector('.xc-alert-branch-badge');
      expect(badge).not.toBeNull();
      expect(badge?.textContent).toBe('br-9');
    });
  });
});
