// #4826: cooldown floor specs (§3 specs: C1-C4)
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { RateAlertSettings } from './RateAlertSettings';
import * as table from './i18n';

interface SentRequest {
  method: string;
  path: string;
  body: Record<string, unknown> | null;
}

describe('#4826: RateAlertSettings cooldown floor specs (§3)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];

  const initialData = {
    defaultCooldownSeconds: 86400,
    maxActivePerCustomer: 10,
    defaultLifetimeDays: 30,
    observePolicy: 'ALL_FRESH',
  };

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      let body: Record<string, unknown> | null = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = null;
        }
      }

      sentRequests.push({ method, path: u.pathname, body });

      if (u.pathname === '/api/v3/views/rate_alert_settings') {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: structuredClone(initialData) }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (u.pathname === '/api/rate-alert-settings' && method === 'PUT') {
        return new Response(
          JSON.stringify({ ...initialData, ...(body ?? {}) }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      return new Response(JSON.stringify({ error: 'not found' }), { status: 404 });
    });
  });

  afterEach(() => {
    if (dispose) {
      dispose();
      dispose = null;
    }
    container.remove();
    vi.restoreAllMocks();
  });

  function mount() {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-23', to: '2026-10-23' });
    const [shown] = createSignal(true);
    dispose = render(() => <RateAlertSettings branch={branch} period={period} shown={shown} />, container);
  }

  // C1: 0.0001 hours (< 0.5s, rounds to 0s) -> errorCooldownInvalid shows, no save request
  it('C1: cooldown 0.0001 hours shows errorCooldownInvalid and sends no request', async () => {
    mount();
    await vi.waitFor(() => {
      expect(container.querySelector('form')).not.toBeNull();
    });
    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    const saveBtn = container.querySelector('button[type="submit"]') as HTMLButtonElement;

    cooldownInput.value = '0.0001';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="cooldown"]')).not.toBeNull();
    });
    expect(container.querySelector('[data-error="cooldown"]')?.textContent).toContain(table.th.errorCooldownInvalid);
    expect(sentRequests.some((r) => r.method === 'PUT')).toBe(false);
  });

  // C2: 0.0002 hours (rounds to 1s) -> save is sent with defaultCooldownSeconds: 1
  it('C2: cooldown 0.0002 hours sends save with defaultCooldownSeconds 1', async () => {
    mount();
    await vi.waitFor(() => {
      expect(container.querySelector('form')).not.toBeNull();
    });
    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    const saveBtn = container.querySelector('button[type="submit"]') as HTMLButtonElement;

    cooldownInput.value = '0.0002';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();

    await vi.waitFor(() => {
      expect(sentRequests.some((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT')).toBe(true);
    });
    const put = sentRequests.find((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT')!;
    expect(put.body?.defaultCooldownSeconds).toBe(1);
    expect(container.querySelector('[data-error="cooldown"]')).toBeNull();
  });

  // C3: 0 hours -> errorCooldownInvalid shows, no request
  it('C3: cooldown 0 hours shows errorCooldownInvalid and sends no request', async () => {
    mount();
    await vi.waitFor(() => {
      expect(container.querySelector('form')).not.toBeNull();
    });
    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    const saveBtn = container.querySelector('button[type="submit"]') as HTMLButtonElement;

    cooldownInput.value = '0';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();

    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="cooldown"]')).not.toBeNull();
    });
    expect(container.querySelector('[data-error="cooldown"]')?.textContent).toContain(table.th.errorCooldownInvalid);
    expect(sentRequests.some((r) => r.method === 'PUT')).toBe(false);
  });

  // C4: 1.5 hours -> save is sent with defaultCooldownSeconds: 5400
  it('C4: cooldown 1.5 hours sends save with defaultCooldownSeconds 5400', async () => {
    mount();
    await vi.waitFor(() => {
      expect(container.querySelector('form')).not.toBeNull();
    });
    const cooldownInput = container.querySelector('input[name="defaultCooldownHours"]') as HTMLInputElement;
    const saveBtn = container.querySelector('button[type="submit"]') as HTMLButtonElement;

    cooldownInput.value = '1.5';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();

    await vi.waitFor(() => {
      expect(sentRequests.some((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT')).toBe(true);
    });
    const put = sentRequests.find((r) => r.path === '/api/rate-alert-settings' && r.method === 'PUT')!;
    expect(put.body?.defaultCooldownSeconds).toBe(5400);
    expect(container.querySelector('[data-error="cooldown"]')).toBeNull();
  });
});
