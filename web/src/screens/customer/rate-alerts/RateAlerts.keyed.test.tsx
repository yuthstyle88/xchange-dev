// #2023 KW-4 Phase 1: RateAlertsScreen idempotency-key retry behaviour (§9 rows 1–5) through HTTP only.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import {
  makeMyRateAlertsData,
  makePublishedLineDto,
  makePublishedRatesData,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { RateAlertsScreen } from './RateAlerts';

interface WriteCall {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

type StepAnswer =
  | { kind: 'network' }
  | { kind: 'http'; status: number; body: unknown };

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const ALERTS_DATA = makeMyRateAlertsData({
  alerts: [],
});

function makeHarness(steps: StepAnswer[]) {
  const writes: WriteCall[] = [];
  let stepIdx = 0;

  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const method = (init?.method ?? 'GET').toUpperCase();
    const u = new URL(input, 'http://localhost');

    if (method === 'GET' && u.pathname === '/api/public/host') {
      return jsonResponse(200, { org: 'tk' });
    }
    if (method === 'GET' && u.pathname === '/api/public/orgs/tk/branches') {
      return jsonResponse(200, { branches: [{ id: 'MAIN', code: 'MAIN', name: 'Siam Head Office' }] });
    }
    if (method === 'GET' && /^\/api\/public\/orgs\/[^/]+\/branches\/[^/]+\/rates$/.test(u.pathname)) {
      return jsonResponse(
        200,
        makePublishedRatesData({ lines: [makePublishedLineDto({ currency: 'USD' })] }),
      );
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/my_rate_alerts') {
      return jsonResponse(200, { version: 1, changed: true, data: ALERTS_DATA });
    }

    if (method === 'POST' && u.pathname === '/api/app/rate-alerts') {
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? init.body : '';
      writes.push({
        method,
        path: u.pathname,
        key: headers.get('idempotency-key'),
        body,
      });
      const step = steps[stepIdx++] ?? { kind: 'http', status: 200, body: { id: 'al-1' } };
      if (step.kind === 'network') {
        throw new TypeError('Failed to fetch');
      }
      return jsonResponse(step.status, step.body);
    }

    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  return {
    api: new ApiClient(fetchFn),
    fetchFn,
    writes,
  };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('RateAlerts.tsx — §9 idempotency-key retry behaviour (#2023 KW-4)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
    setLang('th');
  });

  async function mountScreen(api: ApiClient, fetchFn: Fetch): Promise<void> {
    dispose = render(
      () => <RateAlertsScreen client={api} fetch={fetchFn} org="tk" defaultBranch="MAIN" />,
      host,
    );
    await vi.waitFor(() => {
      expect(host.querySelector('#alert-target')).not.toBeNull();
    });
  }

  function fillTarget(value: string): void {
    const input = host.querySelector('#alert-target') as HTMLInputElement;
    expect(input).not.toBeNull();
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function submitForm(): void {
    const form = host.querySelector('form.xc-create-alert-form') as HTMLFormElement;
    expect(form).not.toBeNull();
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  }

  it('row 1: submit → network TypeError → submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    const { api, fetchFn, writes } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: { id: 'al-1' } },
    ]);

    await mountScreen(api, fetchFn);
    fillTarget('35.50');

    submitForm();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    submitForm();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 2: submit → 500 → submit again sends 2 requests with the same key', async () => {
    const { api, fetchFn, writes } = makeHarness([
      { kind: 'http', status: 500, body: { error: { code: 'internalError', message: 'Server error' } } },
      { kind: 'http', status: 200, body: { id: 'al-1' } },
    ]);

    await mountScreen(api, fetchFn);
    fillTarget('35.50');

    submitForm();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    submitForm();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 3: submit → 4xx refusal → submit again sends a new key', async () => {
    const { api, fetchFn, writes } = makeHarness([
      { kind: 'http', status: 400, body: { error: { code: 'validationFailed', message: 'Invalid rate' } } },
      { kind: 'http', status: 200, body: { id: 'al-1' } },
    ]);

    await mountScreen(api, fetchFn);
    fillTarget('35.50');

    submitForm();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    submitForm();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 4: submit → network error → edit the input → submit sends a new key', async () => {
    const { api, fetchFn, writes } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: { id: 'al-1' } },
    ]);

    await mountScreen(api, fetchFn);
    fillTarget('35.50');

    submitForm();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    fillTarget('36.00');
    submitForm();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).not.toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 5: submit → 2xx → submit the same input again sends a new key', async () => {
    const { api, fetchFn, writes } = makeHarness([
      { kind: 'http', status: 200, body: { id: 'al-1' } },
      { kind: 'http', status: 200, body: { id: 'al-2' } },
    ]);

    await mountScreen(api, fetchFn);
    fillTarget('35.50');

    submitForm();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await flush();

    fillTarget('35.50');
    submitForm();
    await vi.waitFor(() => expect(writes).toHaveLength(2));

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
