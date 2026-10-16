// RR-2 (#2033): C04 history reads my_transactions through ApiClient.view, words failures
// through wordedError, and routes a 401 to sign-in (design xchange-v3-web.md §12.25).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import type { MyTransactionItem, MyTransactionsData } from '../../../shared/api/generated/my_transactions';
import { setAudience, setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { warmModules } from '../../../shared/testing/warm-modules';
import * as shellI18n from '../_shell/i18n';
import { HistoryScreen } from './HistoryScreen';
import { th as historyTh } from './i18n';

beforeAll(
  () =>
    warmModules(
      () => import('./HistoryScreen'),
      () => import('./i18n'),
      () => import('../../../shared/api/client'),
      () => import('../../../shared/i18n'),
      () => import('../../../shared/i18n/worded-error'),
    ),
  60_000,
);

const SAMPLE_ROW: MyTransactionItem = {
  id: 'tx-rr2-1',
  date: '2026-03-31 14:30',
  branch: 'SIAM',
  branchName: 'Siam Paragon',
  type: 'BUY',
  currency: 'USD',
  amount: '100.00',
  rate: '33.50',
  principal: '3350.00',
  fee: '0.00',
  settlement: '3350.00',
  status: 'COMPLETED',
  receiptRef: 'RCP-20260331-001',
};

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

describe('HistoryScreen ApiClient.view & wordedError contract (RR-2 #2033)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  it("row 1: the read goes through ApiClient.view('my_transactions'), with the same branch/filter as today", async () => {
    const viewSpy = vi.spyOn(ApiClient.prototype, 'view');
    const fetchMock = vi.fn(async (url: string) => {
      expect(url).toBe('/api/v3/views/my_transactions');
      return jsonResponse(200, {
        version: 1,
        changed: true,
        data: { transactions: [SAMPLE_ROW] } satisfies MyTransactionsData,
      });
    });

    dispose = render(() => <HistoryScreen fetch={fetchMock} />, host);
    await flush();

    expect(viewSpy).toHaveBeenCalledTimes(1);
    expect(viewSpy.mock.calls[0]?.[0]).toBe('my_transactions');
    expect(viewSpy.mock.calls[0]?.[1]).toBeUndefined();
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("row 2: 500 {code:'roleNotAllowedHere'} shows th['error.serverError'] (W10 (5) 5xx-first)", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse(500, {
        error: {
          code: 'roleNotAllowedHere',
          message: 'Role is not allowed on this route',
        },
      }),
    );

    dispose = render(() => <HistoryScreen fetch={fetchMock} />, host);
    await flush();

    const alert = host.querySelector('.xc-history-error');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    expect(alert!.textContent).not.toContain(sharedTh['error.roleNotAllowedHere']);
    expect(alert!.textContent).not.toContain('Role is not allowed on this route');
  });

  it('row 3: a 4xx keyed refusal shows its key text', async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse(403, {
        error: {
          code: 'roleNotAllowedHere',
          message: 'Role is not allowed on this route',
        },
      }),
    );

    dispose = render(() => <HistoryScreen fetch={fetchMock} />, host);
    await flush();

    const alert = host.querySelector('.xc-history-error');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(sharedTh['error.roleNotAllowedHere']);
    expect(alert!.textContent).not.toContain(sharedTh['error.serverError']);
    expect(alert!.textContent).not.toContain('Role is not allowed on this route');
  });

  it('row 4: network TypeError shows error.network', async () => {
    const fetchMock = vi.fn(async () => {
      throw new TypeError('Failed to fetch');
    });

    dispose = render(() => <HistoryScreen fetch={fetchMock} />, host);
    await flush();

    const alert = host.querySelector('.xc-history-error');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(sharedTh['error.network']);
    expect(alert!.textContent).not.toContain(sharedTh['error.serverError']);
    expect(alert!.textContent).not.toContain('Failed to fetch');
  });

  it('row 5: 401 routes to sign-in, with no error text shown', async () => {
    const navigate = vi.fn<(url: string) => void>();
    const fetchMock = vi.fn(async () =>
      jsonResponse(401, {
        error: {
          code: 'signIn',
          message: 'กรุณาเข้าสู่ระบบ',
        },
      }),
    );

    dispose = render(() => <HistoryScreen fetch={fetchMock} navigate={navigate} />, host);
    await flush();

    expect(navigate).toHaveBeenCalledWith('/app/sign-in');
    const signInLink = host.querySelector('a.xc-customer-signin');
    expect(signInLink).not.toBeNull();
    expect(signInLink?.getAttribute('href')).toBe('/app/sign-in');
    expect(signInLink?.textContent).toContain(shellI18n.th.signIn);

    expect(host.querySelector('.xc-history-error')).toBeNull();
    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(host.textContent).not.toContain(historyTh.loadFailed);
    expect(host.textContent).not.toContain(sharedTh['error.serverError']);
    expect(host.textContent).not.toContain(sharedTh['error.signIn']);
  });

  it('row 6: success renders transaction rows as today', async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse(200, {
        version: 1,
        changed: true,
        data: { transactions: [SAMPLE_ROW] } satisfies MyTransactionsData,
      }),
    );

    dispose = render(() => <HistoryScreen fetch={fetchMock} />, host);
    await flush();

    expect(host.querySelector('.xc-history-error')).toBeNull();
    expect(host.querySelector('.xc-history-empty')).toBeNull();
    const item = host.querySelector('.xc-history-item');
    expect(item).not.toBeNull();
    expect(item!.textContent).toContain('2026-03-31 14:30');
    expect(item!.textContent).toContain('Siam Paragon');
    expect(item!.textContent).toContain(historyTh.status_completed);
    expect(item!.textContent).toContain('100.00');
    expect(item!.textContent).toContain('3,350.00');
    const link = item!.querySelector('a.xc-history-receipt-link');
    expect(link?.getAttribute('href')).toBe('/api/app/receipts/RCP-20260331-001');
  });
});
