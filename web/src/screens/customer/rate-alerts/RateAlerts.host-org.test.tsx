import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { errorText, setAudience, setLang } from '../../../shared/i18n';
import { RateAlertsScreen } from './RateAlerts';

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('PH-rate-alerts (#4651): RateAlerts host with no org is worded, not silent', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.restoreAllMocks();
  });

  it('shows worded notFound text when host answers 200 {} and sends no alert request', async () => {
    const calls: Array<{ method: string; url: string }> = [];
    const fetchFn = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
      const method = (init?.method ?? 'GET').toUpperCase();
      calls.push({ method, url });
      if (url === '/api/public/host') {
        return json({});
      }
      if (url.includes('/api/v3/views/my_rate_alerts')) {
        return json({ version: 1, changed: true, data: { alerts: [] } });
      }
      return json({ error: { code: 'notFound', message: 'not found' } }, 404);
    });

    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    const expected = errorText('notFound', 'ไม่พบข้อมูล');
    await vi.waitFor(() => {
      const alerts = Array.from(host.querySelectorAll('[role="alert"]'));
      expect(alerts.some((a) => a.textContent?.includes(expected))).toBe(true);
    });

    const targetInput = host.querySelector<HTMLInputElement>('#alert-target')!;
    targetInput.value = '35.00';
    targetInput.dispatchEvent(new Event('input', { bubbles: true }));

    const form = host.querySelector<HTMLFormElement>('.xc-create-alert-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

    expect(calls.some((c) => c.url.includes('/api/public/orgs/'))).toBe(false);
    expect(calls.some((c) => c.url.startsWith('/api/app/rate-alerts'))).toBe(false);
  });
});
