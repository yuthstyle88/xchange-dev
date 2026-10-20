// #2023 KW-4 Phase 1: DevicesScreen idempotency-key retry behaviour (§9 rows 1–5) through HTTP only.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../shared/api/client';
import { makeDeviceBranch, makeDevicesOverview } from '../../shared/testing/fixtures';
import { setLang } from '../../shared/i18n';
import { DevicesScreen } from './DevicesScreen';
import { th } from './i18n';

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

const OVERVIEW_DATA = makeDevicesOverview({
  branchId: 'B1',
  branches: [makeDeviceBranch({ id: 'B1', name: 'สาขาหนึ่ง' })],
  printerOnline: true,
  printerKind: 'SIMULATED',
  printerModel: 'NEOCAL NTP-8220BT',
  hasDrawer: null,
  drawerAgentActive: false,
  balanceThb: '1000.00',
  balanceUsd: '50.00',
  completedCount: 1,
});

function makeHarness(steps: StepAnswer[]) {
  const writes: WriteCall[] = [];
  let stepIdx = 0;

  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const method = (init?.method ?? 'GET').toUpperCase();
    const u = new URL(input, 'http://localhost');

    if (method === 'GET' && u.pathname === '/api/v3/views/devices') {
      return jsonResponse(200, { version: 1, changed: true, data: OVERVIEW_DATA });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/device_jobs') {
      return jsonResponse(200, { version: 1, changed: true, data: [] });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/device_events') {
      return jsonResponse(200, { version: 1, changed: true, data: [] });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/device_transactions') {
      return jsonResponse(200, [{ id: 'TX-1', type: 'BUY', amount: '100.00', currency: 'USD' }]);
    }

    if (method === 'POST' && u.pathname === '/api/devices') {
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? init.body : '';
      writes.push({
        method,
        path: u.pathname,
        key: headers.get('idempotency-key'),
        body,
      });
      const step = steps[stepIdx++] ?? { kind: 'http', status: 200, body: { saved: true } };
      if (step.kind === 'network') {
        throw new TypeError('Failed to fetch');
      }
      return jsonResponse(step.status, step.body);
    }

    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  return {
    api: new ApiClient(fetchFn),
    writes,
  };
}

describe('DevicesScreen.tsx — §9 idempotency-key retry behaviour (#2023 KW-4)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('offline in tests'))));
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  function findDrawerBtn(): HTMLButtonElement {
    const btns = [...host.querySelectorAll('button')].filter((b) => (b.textContent ?? '').trim() === th.drawer);
    expect(btns).toHaveLength(1);
    return btns[0]!;
  }

  function findReasonInput(): HTMLInputElement {
    const input = host.querySelector('.request-grid input[type="text"]') as HTMLInputElement;
    expect(input).not.toBeNull();
    return input;
  }

  function fillReason(value: string): void {
    const input = findReasonInput();
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  }

  async function mountReady(api: ApiClient): Promise<void> {
    dispose = render(() => <DevicesScreen api={api} branch="B1" />, host);
    await vi.waitFor(() => {
      const btn = findDrawerBtn();
      expect(btn.disabled).toBe(false);
    });
  }

  async function waitIdle(): Promise<void> {
    await vi.waitFor(() => {
      expect(findDrawerBtn().disabled).toBe(false);
    });
  }

  it('row 1: submit → network TypeError → submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    const { api, writes } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: { saved: true } },
    ]);

    await mountReady(api);
    fillReason('ส่งมอบเงินให้ลูกค้า');

    findDrawerBtn().click();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await waitIdle();

    findDrawerBtn().click();
    await vi.waitFor(() => expect(writes).toHaveLength(2));
    await waitIdle();

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 2: submit → 500 → submit again sends 2 requests with the same key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'http', status: 500, body: { error: { code: 'internalError', message: 'Server error' } } },
      { kind: 'http', status: 200, body: { saved: true } },
    ]);

    await mountReady(api);
    fillReason('ส่งมอบเงินให้ลูกค้า');

    findDrawerBtn().click();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await waitIdle();

    findDrawerBtn().click();
    await vi.waitFor(() => expect(writes).toHaveLength(2));
    await waitIdle();

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 3: submit → 4xx refusal → submit again sends a new key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'http', status: 400, body: { error: { code: 'refused', message: 'บทบาทนี้ไม่มีสิทธิ์ทำขั้นตอนนี้' } } },
      { kind: 'http', status: 200, body: { saved: true } },
    ]);

    await mountReady(api);
    fillReason('ส่งมอบเงินให้ลูกค้า');

    findDrawerBtn().click();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await waitIdle();

    findDrawerBtn().click();
    await vi.waitFor(() => expect(writes).toHaveLength(2));
    await waitIdle();

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 4: submit → network error → edit the input → submit sends a new key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: { saved: true } },
    ]);

    await mountReady(api);
    fillReason('ส่งมอบเงินให้ลูกค้า');

    findDrawerBtn().click();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await waitIdle();

    fillReason('ตรวจสอบเงินทอน');
    findDrawerBtn().click();
    await vi.waitFor(() => expect(writes).toHaveLength(2));
    await waitIdle();

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).not.toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 5: submit → 2xx → submit the same input again sends a new key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'http', status: 200, body: { saved: true } },
      { kind: 'http', status: 200, body: { saved: true } },
    ]);

    await mountReady(api);
    fillReason('ส่งมอบเงินให้ลูกค้า');

    findDrawerBtn().click();
    await vi.waitFor(() => expect(writes).toHaveLength(1));
    await waitIdle();

    findDrawerBtn().click();
    await vi.waitFor(() => expect(writes).toHaveLength(2));
    await waitIdle();

    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
