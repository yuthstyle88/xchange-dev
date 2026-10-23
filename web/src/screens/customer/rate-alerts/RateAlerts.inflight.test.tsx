import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  makeMyRateAlertItem,
  makePublishedLineDto,
  makePublishedRatesData,
  type MyRateAlertItem,
} from '../../../shared/testing/fixtures';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { setAudience, setLang } from '../../../shared/i18n';
import { RateAlertsScreen } from './RateAlerts';
import * as table from './i18n';

interface RecordedCall {
  method: string;
  url: string;
  headers: Record<string, string>;
  body: unknown;
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

const SAMPLE_BRANCHES: PublicBranchesData = {
  branches: [
    {
      id: 'XC-B01',
      code: 'XC-B01',
      name: 'สาขาสยามพารากอน',
      address: '991 ถ.พระราม 1',
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
  ],
};

function makeAlert(overrides: Partial<MyRateAlertItem> = {}) {
  return makeMyRateAlertItem({
    id: 'alert-1',
    version: 1,
    branch: 'XC-B01',
    currency: 'USD',
    customerSide: 'CUSTOMER_BUY',
    condition: 'ABOVE',
    target: '34.00',
    unitSize: '1',
    currentRate: '34.10',
    ratePublishedAt: '2026-10-13T10:00:00Z',
    enabled: true,
    relation: 'UNKNOWN',
    expiresAt: null,
    expired: false,
    rateStale: null,
    lastEvent: null,
    ...overrides,
  });
}

const SAMPLE_RATES = makePublishedRatesData({
  lines: [makePublishedLineDto({ currency: 'USD' })],
});

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('RateAlerts in-flight action guard and loadSeq poll guard (#2278)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    if (dispose) {
      dispose();
      dispose = undefined;
    }
    if (host.parentNode) {
      host.parentNode.removeChild(host);
    }
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  it('row 1: Double tap Delete -> One POST; no error', async () => {
    const calls: RecordedCall[] = [];
    const pendingDelete = deferred<Response>();
    let deleteInFlight = false;
    let alerts = [makeAlert({ id: 'alert-1', version: 1 })];

    const fetchFn = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
      const method = init?.method ?? 'GET';
      const headers: Record<string, string> = {};
      if (init?.headers) {
        new Headers(init.headers).forEach((v, k) => {
          headers[k.toLowerCase()] = v;
        });
      }
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ method, url, headers, body });

      if (url === '/api/public/host') return jsonResponse({ org: 'XC-ORG' });
      if (/^\/api\/public\/orgs\/[^/]+\/branches\/[^/]+\/rates$/.test(url)) return jsonResponse(SAMPLE_RATES);
      if (url.includes('/branches')) return jsonResponse(SAMPLE_BRANCHES);
      if (url.includes('/api/v3/views/my_rate_alerts')) {
        return jsonResponse({ version: 1, changed: true, data: { alerts } });
      }
      if (method === 'POST' && url.includes('/delete')) {
        if (deleteInFlight) {
          return jsonResponse(
            { error: { code: 'idempotencyConflict', message: 'Duplicate request in flight' } },
            409,
          );
        }
        deleteInFlight = true;
        const res = await pendingDelete.promise;
        deleteInFlight = false;
        alerts = [];
        return res;
      }
      return jsonResponse({});
    });

    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);
    await settle();

    const deleteBtn = host.querySelector<HTMLButtonElement>('.xc-btn-delete');
    const pauseBtn = host.querySelector<HTMLButtonElement>('.xc-btn-pause');
    expect(deleteBtn).not.toBeNull();
    expect(pauseBtn).not.toBeNull();

    // First tap starts delete
    deleteBtn!.click();
    expect(deleteBtn!.disabled).toBe(true);
    expect(pauseBtn!.disabled).toBe(true);

    // Second tap while in flight (even if invoked directly) must not send a second POST
    deleteBtn!.click();
    await settle();

    const deleteCallsDuringFlight = calls.filter((c) => c.method === 'POST' && c.url.includes('/delete'));
    expect(deleteCallsDuringFlight.length).toBe(1);

    pendingDelete.resolve(jsonResponse({ id: 'alert-1', version: 2 }));
    await settle();

    const allDeleteCalls = calls.filter((c) => c.method === 'POST' && c.url.includes('/delete'));
    expect(allDeleteCalls.length).toBe(1);
    expect(host.querySelector('.xc-form-error')).toBeNull();
  });

  it('row 2: Poll in flight, then Pause and forced re-read, poll answers last -> The alert shows as paused', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const stalePoll = deferred<Response>();
    let viewCallCount = 0;

    const fetchFn = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
      const method = init?.method ?? 'GET';
      if (url === '/api/public/host') return jsonResponse({ org: 'XC-ORG' });
      if (/^\/api\/public\/orgs\/[^/]+\/branches\/[^/]+\/rates$/.test(url)) return jsonResponse(SAMPLE_RATES);
      if (url.includes('/branches')) return jsonResponse(SAMPLE_BRANCHES);
      if (url.includes('/api/v3/views/my_rate_alerts')) {
        viewCallCount++;
        if (viewCallCount === 1) {
          // Initial load: active (version 1)
          return jsonResponse({
            version: 1,
            changed: true,
            data: { alerts: [makeAlert({ id: 'alert-1', enabled: true, version: 1 })] },
          });
        }
        if (viewCallCount === 2) {
          // Poll started before Pause: hangs in flight
          return stalePoll.promise;
        }
        // Forced re-read after Pause: paused (version 2)
        return jsonResponse({
          version: 2,
          changed: true,
          data: { alerts: [makeAlert({ id: 'alert-1', enabled: false, version: 2 })] },
        });
      }
      if (method === 'POST' && url.includes('/pause')) {
        return jsonResponse({ id: 'alert-1', version: 2 });
      }
      return jsonResponse({});
    });

    dispose = render(() => <RateAlertsScreen fetch={fetchFn} pollIntervalMs={1000} />, host);
    await settle();

    expect(host.querySelector('.xc-badge')?.textContent).toBe(table.th.statusActive);

    // Fire the single poll (#2), which stays in flight
    vi.advanceTimersByTime(1000);
    expect(viewCallCount).toBe(2);

    // Customer taps Pause; pause + forced re-read (#3) complete while poll #2 is still in flight
    const pauseBtn = host.querySelector<HTMLButtonElement>('.xc-btn-pause');
    expect(pauseBtn).not.toBeNull();
    pauseBtn!.click();
    await settle();

    expect(host.querySelector('.xc-badge')?.textContent).toBe(table.th.statusPaused);

    // Now the stale poll (#2) answers with the old active state (version 1)
    stalePoll.resolve(
      jsonResponse({
        version: 1,
        changed: true,
        data: { alerts: [makeAlert({ id: 'alert-1', enabled: true, version: 1 })] },
      }),
    );
    await settle();

    // Must still show as paused
    expect(host.querySelector('.xc-badge')?.textContent).toBe(table.th.statusPaused);
  });

  it("row 3: Pause alert A while alert B's action is in flight -> Both are sent", async () => {
    const calls: RecordedCall[] = [];
    const pendingB = deferred<Response>();
    const pendingA = deferred<Response>();

    const fetchFn = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
      const method = init?.method ?? 'GET';
      const headers: Record<string, string> = {};
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ method, url, headers, body });

      if (url === '/api/public/host') return jsonResponse({ org: 'XC-ORG' });
      if (/^\/api\/public\/orgs\/[^/]+\/branches\/[^/]+\/rates$/.test(url)) return jsonResponse(SAMPLE_RATES);
      if (url.includes('/branches')) return jsonResponse(SAMPLE_BRANCHES);
      if (url.includes('/api/v3/views/my_rate_alerts')) {
        return jsonResponse({
          version: 1,
          changed: true,
          data: {
            alerts: [
              makeAlert({ id: 'alert-a', currency: 'USD', version: 1 }),
              makeAlert({ id: 'alert-b', currency: 'EUR', version: 1 }),
            ],
          },
        });
      }
      if (method === 'POST' && url.includes('/alert-b/delete')) {
        return pendingB.promise;
      }
      if (method === 'POST' && url.includes('/alert-a/pause')) {
        return pendingA.promise;
      }
      return jsonResponse({});
    });

    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);
    await settle();

    const cardA = host.querySelector('[data-alert-id="alert-a"]')!;
    const cardB = host.querySelector('[data-alert-id="alert-b"]')!;
    const pauseA = cardA.querySelector<HTMLButtonElement>('.xc-btn-pause')!;
    const deleteB = cardB.querySelector<HTMLButtonElement>('.xc-btn-delete')!;

    // Start action on alert B
    deleteB.click();
    expect(deleteB.disabled).toBe(true);
    expect(pauseA.disabled).toBe(false);

    // Pause alert A while alert B is still in flight
    pauseA.click();
    expect(pauseA.disabled).toBe(true);
    await settle();

    expect(calls.filter((c) => c.method === 'POST' && c.url.includes('/alert-b/delete')).length).toBe(1);
    expect(calls.filter((c) => c.method === 'POST' && c.url.includes('/alert-a/pause')).length).toBe(1);

    pendingB.resolve(jsonResponse({ id: 'alert-b', version: 2 }));
    pendingA.resolve(jsonResponse({ id: 'alert-a', version: 2 }));
    await settle();
  });

  it('row 4: Pause/resume/delete send expectedVersion with an Idempotency-Key and disable buttons while in flight', async () => {
    const calls: RecordedCall[] = [];
    const pendingPause = deferred<Response>();
    let alerts = [makeAlert({ id: 'alert-1', version: 5, enabled: true })];

    const fetchFn = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
      const method = init?.method ?? 'GET';
      const headers: Record<string, string> = {};
      if (init?.headers) {
        new Headers(init.headers).forEach((v, k) => {
          headers[k.toLowerCase()] = v;
        });
      }
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ method, url, headers, body });

      if (url === '/api/public/host') return jsonResponse({ org: 'XC-ORG' });
      if (/^\/api\/public\/orgs\/[^/]+\/branches\/[^/]+\/rates$/.test(url)) return jsonResponse(SAMPLE_RATES);
      if (url.includes('/branches')) return jsonResponse(SAMPLE_BRANCHES);
      if (url.includes('/api/v3/views/my_rate_alerts')) {
        return jsonResponse({ version: alerts[0]?.version ?? 8, changed: true, data: { alerts } });
      }
      if (method === 'POST' && url.includes('/pause')) {
        const res = await pendingPause.promise;
        alerts = [makeAlert({ id: 'alert-1', version: 6, enabled: false })];
        return res;
      }
      if (method === 'POST' && url.includes('/resume')) {
        alerts = [makeAlert({ id: 'alert-1', version: 7, enabled: true })];
        return jsonResponse({ id: 'alert-1', version: 7 });
      }
      if (method === 'POST' && url.includes('/delete')) {
        alerts = [];
        return jsonResponse({ id: 'alert-1', version: 8 });
      }
      return jsonResponse({});
    });

    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);
    await settle();

    const pauseBtn = host.querySelector<HTMLButtonElement>('.xc-btn-pause')!;
    pauseBtn.click();
    expect(pauseBtn.disabled).toBe(true);

    pendingPause.resolve(jsonResponse({ id: 'alert-1', version: 6 }));
    await settle();

    const pauseCall = calls.find((c) => c.url.includes('/pause'));
    expect(pauseCall?.body).toEqual({ expectedVersion: 5 });
    expect(pauseCall?.headers['idempotency-key']).toBeTruthy();

    const resumeBtn = host.querySelector<HTMLButtonElement>('.xc-btn-resume')!;
    resumeBtn.click();
    await settle();

    const resumeCall = calls.find((c) => c.url.includes('/resume'));
    expect(resumeCall?.body).toEqual({ expectedVersion: 6 });
    expect(resumeCall?.headers['idempotency-key']).toBeTruthy();

    const deleteBtn = host.querySelector<HTMLButtonElement>('.xc-btn-delete')!;
    deleteBtn.click();
    await settle();

    const deleteCall = calls.find((c) => c.url.includes('/delete'));
    expect(deleteCall?.body).toEqual({ expectedVersion: 7 });
    expect(deleteCall?.headers['idempotency-key']).toBeTruthy();
  });

  it('row 5: Create is still guarded by isCreating while alert actions are guarded per alert', async () => {
    const calls: RecordedCall[] = [];
    const pendingCreate = deferred<Response>();
    const pendingPause = deferred<Response>();

    const fetchFn = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
      const method = init?.method ?? 'GET';
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ method, url, headers: {}, body });

      if (url === '/api/public/host') return jsonResponse({ org: 'XC-ORG' });
      if (/^\/api\/public\/orgs\/[^/]+\/branches\/[^/]+\/rates$/.test(url)) return jsonResponse(SAMPLE_RATES);
      if (url.includes('/branches')) return jsonResponse(SAMPLE_BRANCHES);
      if (url.includes('/api/v3/views/my_rate_alerts')) {
        return jsonResponse({
          version: 1,
          changed: true,
          data: { alerts: [makeAlert({ id: 'alert-1', version: 1 })] },
        });
      }
      if (method === 'POST' && url === '/api/app/rate-alerts') {
        return pendingCreate.promise;
      }
      if (method === 'POST' && url.includes('/pause')) {
        return pendingPause.promise;
      }
      return jsonResponse({});
    });

    dispose = render(() => <RateAlertsScreen fetch={fetchFn} defaultBranch="XC-B01" />, host);
    await settle();

    const targetInput = host.querySelector<HTMLInputElement>('#alert-target')!;
    targetInput.value = '35.00';
    targetInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    const form = host.querySelector<HTMLFormElement>('.xc-create-alert-form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    expect(submitBtn.disabled).toBe(true);

    // Second submit while creating is ignored
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    expect(calls.filter((c) => c.method === 'POST' && c.url === '/api/app/rate-alerts').length).toBe(1);

    // Alert pause also guarded while in flight
    const pauseBtn = host.querySelector<HTMLButtonElement>('.xc-btn-pause')!;
    pauseBtn.click();
    expect(pauseBtn.disabled).toBe(true);
    pauseBtn.click();
    expect(calls.filter((c) => c.method === 'POST' && c.url.includes('/pause')).length).toBe(1);

    pendingCreate.resolve(jsonResponse({ id: 'alert-new', version: 1 }));
    pendingPause.resolve(jsonResponse({ id: 'alert-1', version: 2 }));
    await settle();
  });

  it('edge case 1: staleVersion re-read goes through the loadSeq guard so an older poll cannot overwrite it', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    const stalePoll = deferred<Response>();
    let viewCallCount = 0;

    const fetchFn = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
      const method = init?.method ?? 'GET';
      if (url === '/api/public/host') return jsonResponse({ org: 'XC-ORG' });
      if (/^\/api\/public\/orgs\/[^/]+\/branches\/[^/]+\/rates$/.test(url)) return jsonResponse(SAMPLE_RATES);
      if (url.includes('/branches')) return jsonResponse(SAMPLE_BRANCHES);
      if (url.includes('/api/v3/views/my_rate_alerts')) {
        viewCallCount++;
        if (viewCallCount === 1) {
          return jsonResponse({
            version: 1,
            changed: true,
            data: { alerts: [makeAlert({ id: 'alert-1', enabled: true, version: 1 })] },
          });
        }
        if (viewCallCount === 2) {
          return stalePoll.promise;
        }
        // Re-read after staleVersion returns the updated server state (paused, version 2)
        return jsonResponse({
          version: 2,
          changed: true,
          data: { alerts: [makeAlert({ id: 'alert-1', enabled: false, version: 2 })] },
        });
      }
      if (method === 'POST' && url.includes('/pause')) {
        return jsonResponse({ error: { code: 'staleVersion', message: 'Stale version' } }, 409);
      }
      return jsonResponse({});
    });

    dispose = render(() => <RateAlertsScreen fetch={fetchFn} pollIntervalMs={1000} />, host);
    await settle();

    vi.advanceTimersByTime(1000);
    expect(viewCallCount).toBe(2);

    const pauseBtn = host.querySelector<HTMLButtonElement>('.xc-btn-pause')!;
    pauseBtn.click();
    await settle();

    expect(host.querySelector('.xc-badge')?.textContent).toBe(table.th.statusPaused);

    // Resolve the older poll that started before the staleVersion re-read
    stalePoll.resolve(
      jsonResponse({
        version: 1,
        changed: true,
        data: { alerts: [makeAlert({ id: 'alert-1', enabled: true, version: 1 })] },
      }),
    );
    await settle();

    expect(host.querySelector('.xc-badge')?.textContent).toBe(table.th.statusPaused);
  });

  it('edge case 2: poll timer is cleared on cleanup and in-flight action guard prevents duplicate pause before unmount', async () => {
    vi.useFakeTimers();
    const calls: RecordedCall[] = [];
    const pendingPause = deferred<Response>();

    const fetchFn = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
      const method = init?.method ?? 'GET';
      calls.push({ method, url, headers: {}, body: undefined });
      if (url === '/api/public/host') return jsonResponse({ org: 'XC-ORG' });
      if (/^\/api\/public\/orgs\/[^/]+\/branches\/[^/]+\/rates$/.test(url)) return jsonResponse(SAMPLE_RATES);
      if (url.includes('/branches')) return jsonResponse(SAMPLE_BRANCHES);
      if (url.includes('/api/v3/views/my_rate_alerts')) {
        return jsonResponse({
          version: 1,
          changed: true,
          data: { alerts: [makeAlert({ id: 'alert-1', enabled: true, version: 1 })] },
        });
      }
      if (method === 'POST' && url.includes('/pause')) {
        return pendingPause.promise;
      }
      return jsonResponse({});
    });

    dispose = render(() => <RateAlertsScreen fetch={fetchFn} pollIntervalMs={1000} />, host);
    await vi.advanceTimersByTimeAsync(10);

    const pauseBtn = host.querySelector<HTMLButtonElement>('.xc-btn-pause')!;
    pauseBtn.click();
    expect(pauseBtn.disabled).toBe(true);

    // Unmount and verify polling stops
    dispose();
    dispose = undefined;

    const viewsBefore = calls.filter((c) => c.url.includes('/api/v3/views/my_rate_alerts')).length;
    await vi.advanceTimersByTimeAsync(5000);
    const viewsAfter = calls.filter((c) => c.url.includes('/api/v3/views/my_rate_alerts')).length;
    expect(viewsAfter).toBe(viewsBefore);
  });
});
