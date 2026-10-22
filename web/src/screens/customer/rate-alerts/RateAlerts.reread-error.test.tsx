import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { makeMyRateAlertItem } from '../../../shared/testing/fixtures';
import { errorText, setLang } from '../../../shared/i18n';
import { RateAlertsScreen } from './RateAlerts';
import * as table from './i18n';

const disposers: Array<() => void> = [];

const SAMPLE_BRANCHES = {
  branches: [
    {
      code: 'XC-B01',
      name: 'สาขาสยามพารากอน',
      address: null,
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
  ],
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function refusal(status: number, code: string, message: string): Response {
  return json({ error: { code, message } }, status);
}

function sampleAlert(id = 'alert-1', enabled = true) {
  return makeMyRateAlertItem({
    id,
    version: 1,
    branch: 'XC-B01',
    currency: 'USD',
    customerSide: 'CUSTOMER_BUY',
    condition: 'ABOVE',
    target: '35.0000',
    unitSize: '1',
    currentRate: null,
    ratePublishedAt: null,
    enabled,
    relation: 'UNKNOWN',
    expiresAt: null,
    expired: false,
    rateStale: null,
    lastEvent: null,
  });
}

let alertsReply: (() => Response) | null = null;
let postDeleteReply: (() => Response) | null = null;
let postPauseReply: (() => Response) | null = null;
let postCreateReply: (() => Response) | null = null;

async function fakeFetch(url: string, init?: RequestInit): Promise<Response> {
  const method = init?.method ?? 'GET';

  if (url === '/api/public/host') {
    return json({ org: 'XC-ORG' });
  }

  if (url.includes('/api/public/orgs/')) {
    return json(SAMPLE_BRANCHES);
  }

  if (url.includes('/api/v3/views/my_rate_alerts')) {
    if (alertsReply) return alertsReply();
    return json({ version: 1, changed: true, data: { alerts: [] } });
  }

  if (method === 'POST' && url.includes('/delete')) {
    if (postDeleteReply) return postDeleteReply();
    return json({ id: 'alert-1', version: 2 });
  }

  if (method === 'POST' && url.includes('/pause')) {
    if (postPauseReply) return postPauseReply();
    return json({ id: 'alert-1', version: 2 });
  }

  if (method === 'POST' && url === '/api/app/rate-alerts') {
    if (postCreateReply) return postCreateReply();
    return json({ id: 'alert-new-1', version: 1 });
  }

  return json({});
}

function mount() {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const dispose = render(() => <RateAlertsScreen fetch={fakeFetch} />, host);
  disposers.push(() => {
    dispose();
    host.remove();
  });
  return host;
}

beforeEach(() => {
  setLang('th');
  alertsReply = null;
  postDeleteReply = null;
  postPauseReply = null;
  postCreateReply = null;
});

afterEach(() => {
  while (disposers.length > 0) {
    disposers.pop()!();
  }
});

describe('RateAlerts re-read error behaviour (§8.2 R4-3)', () => {
  it('A1: one alert on screen; delete answers 2xx; the re-read fails -> the worded line with a retry shows', async () => {
    alertsReply = () => json({ version: 1, changed: true, data: { alerts: [sampleAlert('alert-1')] } });
    const host = mount();

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-alert-card')).not.toBeNull();
    });

    alertsReply = () => refusal(500, 'serverError', 'Failed to reload alerts');
    const deleteBtn = host.querySelector('.xc-alert-card .xc-btn-delete') as HTMLButtonElement;
    deleteBtn.click();

    await vi.waitFor(() => {
      const rereadErr = host.querySelector('.xc-alerts-reread-error');
      expect(rereadErr).not.toBeNull();
      expect(rereadErr?.getAttribute('role')).toBe('alert');
      expect(rereadErr?.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
      const retryBtn = rereadErr?.querySelector('button[type="button"]');
      expect(retryBtn).not.toBeNull();
      expect(retryBtn?.textContent).toContain(table.th.retry);
    });

    expect(host.querySelector('.xc-alert-card')).not.toBeNull();
    expect(host.querySelector('.xc-alerts-error')).toBeNull();
  });

  it('A2: A1, then retry, and the read answers 2xx -> the line is gone; the alert is gone', async () => {
    alertsReply = () => json({ version: 1, changed: true, data: { alerts: [sampleAlert('alert-1')] } });
    const host = mount();

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-alert-card')).not.toBeNull();
    });

    alertsReply = () => refusal(500, 'serverError', 'Failed to reload alerts');
    const deleteBtn = host.querySelector('.xc-alert-card .xc-btn-delete') as HTMLButtonElement;
    deleteBtn.click();

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-alerts-reread-error')).not.toBeNull();
    });

    alertsReply = () => json({ version: 2, changed: true, data: { alerts: [] } });
    const retryBtn = host.querySelector('.xc-alerts-reread-error button[type="button"]') as HTMLButtonElement;
    retryBtn.click();

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-alerts-reread-error')).toBeNull();
      expect(host.querySelector('.xc-alert-card')).toBeNull();
      expect(host.querySelector('.xc-alerts-empty')).not.toBeNull();
    });
  });

  it('A3: pause answers 2xx; the re-read fails on the network -> the worded line shows', async () => {
    alertsReply = () => json({ version: 1, changed: true, data: { alerts: [sampleAlert('alert-1', true)] } });
    const host = mount();

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-alert-card')).not.toBeNull();
    });

    alertsReply = () => {
      throw new TypeError('Failed to fetch');
    };
    const pauseBtn = host.querySelector('.xc-alert-card .xc-btn-pause') as HTMLButtonElement;
    pauseBtn.click();

    await vi.waitFor(() => {
      const rereadErr = host.querySelector('.xc-alerts-reread-error');
      expect(rereadErr).not.toBeNull();
      expect(rereadErr?.getAttribute('role')).toBe('alert');
      expect(rereadErr?.textContent).toContain(errorText('network', 'เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่'));
      expect(rereadErr?.querySelector('button[type="button"]')).not.toBeNull();
    });

    expect(host.querySelector('.xc-alert-card')).not.toBeNull();
    expect(host.querySelector('.xc-alerts-error')).toBeNull();
  });

  it('A4: create answers 2xx; the re-read fails -> the worded line shows', async () => {
    alertsReply = () => json({ version: 1, changed: true, data: { alerts: [sampleAlert('alert-1')] } });
    const host = mount();

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-alert-card')).not.toBeNull();
      expect(host.querySelector('#alert-branch option[value="XC-B01"]')).not.toBeNull();
    });

    const targetInput = host.querySelector('#alert-target') as HTMLInputElement;
    targetInput.value = '36.50';
    targetInput.dispatchEvent(new Event('input', { bubbles: true }));

    alertsReply = () => refusal(500, 'serverError', 'Failed to reload alerts');
    const form = host.querySelector('.xc-create-alert-form') as HTMLFormElement;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    await vi.waitFor(() => {
      const rereadErr = host.querySelector('.xc-alerts-reread-error');
      expect(rereadErr).not.toBeNull();
      expect(rereadErr?.getAttribute('role')).toBe('alert');
      expect(rereadErr?.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
      expect(rereadErr?.querySelector('button[type="button"]')).not.toBeNull();
    });

    expect(host.querySelector('.xc-alert-card')).not.toBeNull();
    expect(host.querySelector('.xc-alerts-error')).toBeNull();
  });

  it("A5: no alerts; the first read fails -> today's empty-list .xc-alerts-error shows", async () => {
    alertsReply = () => refusal(500, 'serverError', 'Initial load failure');
    const host = mount();

    await vi.waitFor(() => {
      const emptyErr = host.querySelector('.xc-alerts-error');
      expect(emptyErr).not.toBeNull();
      expect(emptyErr?.getAttribute('role')).toBe('alert');
      expect(emptyErr?.textContent).toContain(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
      expect(emptyErr?.querySelector('.xc-alerts-retry-btn')).not.toBeNull();
    });

    expect(host.querySelector('.xc-alerts-reread-error')).toBeNull();
    expect(host.querySelector('.xc-alert-card')).toBeNull();
  });
});
