import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { CashPolicy } from './CashPolicy';
import { setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import {
  makeAdminCashData,
  makeCashTransferPolicyData,
} from '../../../shared/testing/fixtures';

const SAMPLE_POLICY = makeCashTransferPolicyData({
  version: 'v-0001',
  limits: { USD: '300.00', EUR: '12.5' },
  valid: true,
  invalidEntries: [],
  setBy: 'u-exec',
  createdAt: '2026-10-12T01:00:00Z',
});

const SAMPLE_CASH = makeAdminCashData({
  openShifts: [],
  stock: [],
  stats: null,
  reference: { withReference: 0, withoutReference: 0, spread: '0.00', percent: '0.00' },
  currencies: ['THB', 'USD', 'EUR'],
  lastCounted: null,
});

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let viewHandlers: Record<string, (params?: URLSearchParams) => Promise<Response>>;
let saveHandler: (body: unknown) => Promise<Response>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stubServer(overrides: {
  views?: Record<string, (params?: URLSearchParams) => Promise<Response>>;
  save?: (body: unknown) => Promise<Response>;
} = {}) {
  viewHandlers = {
    cash_transfer_policy: async () => json(200, { version: 1, changed: true, data: SAMPLE_POLICY }),
    admin_cash: async () => json(200, { version: 1, changed: true, data: SAMPLE_CASH }),
    ...overrides.views,
  };
  saveHandler = overrides.save ?? (async () => json(200, { version: 1, changed: true, data: SAMPLE_POLICY }));

  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const [path, qs] = url.split('?');
      const params = new URLSearchParams(qs ?? '');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && path!.startsWith('/api/v3/views/')) {
        const viewName = path!.slice('/api/v3/views/'.length);
        if (viewHandlers[viewName]) {
          return viewHandlers[viewName]!(params);
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }

      if (method === 'PUT' && path === '/api/cash-transfer-policy') {
        const body = init?.body ? JSON.parse(String(init.body)) : {};
        return saveHandler(body);
      }

      return json(200, {});
    }),
  );
}

function mount(branchSignal?: () => string) {
  const [defaultBranch] = createSignal('MAIN');
  const b = branchSignal ?? defaultBranch;
  const [p] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [s] = createSignal(true);
  dispose = render(() => <CashPolicy branch={b} period={p} shown={s} />, host);
}

function getAlertText(): string | null {
  const el = host.querySelector('[role="alert"]');
  return el ? el.textContent?.trim() ?? '' : null;
}

describe('CashPolicy branch register read and error (#2669)', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
    setLang('th');
  });

  it('P1: MAIN admin_cash 500 (alert shown) -> switch to B02, which reads fine -> B02 rows shown, no register alert', async () => {
    const [branch, setBranch] = createSignal('MAIN');
    stubServer({
      views: {
        admin_cash: async (params) => {
          const b = params?.get('branch');
          if (b === 'MAIN') {
            return json(500, { error: { code: 'serverError', message: 'Failed MAIN' } });
          }
          return json(200, { version: 1, changed: true, data: { ...SAMPLE_CASH, currencies: ['B02_CUR'] } });
        },
      },
    });

    mount(() => branch());
    await vi.waitFor(() => expect(getAlertText()).toBe(th['error.serverError']));

    setBranch('B02');
    await vi.waitFor(() => expect(host.querySelector('input[data-currency="B02_CUR"]')).not.toBeNull());

    expect(getAlertText(), 'no register alert after successful read on B02').toBeNull();
  });

  it("P2: MAIN read held, switch to B02 (answers ['ZZB']), then MAIN answers ['ZZA'] -> ZZB shown, ZZA not", async () => {
    const [branch, setBranch] = createSignal('MAIN');
    let resolveMain!: (res: Response) => void;
    const mainPromise = new Promise<Response>((res) => {
      resolveMain = res;
    });

    stubServer({
      views: {
        admin_cash: async (params) => {
          const b = params?.get('branch');
          if (b === 'MAIN') {
            return mainPromise;
          }
          return json(200, { version: 1, changed: true, data: { ...SAMPLE_CASH, currencies: ['ZZB'] } });
        },
      },
    });

    mount(() => branch());

    // Switch to B02 before MAIN resolves
    setBranch('B02');
    await vi.waitFor(() => expect(host.querySelector('input[data-currency="ZZB"]')).not.toBeNull());

    // Now MAIN resolves late with ZZA
    resolveMain(json(200, { version: 1, changed: true, data: { ...SAMPLE_CASH, currencies: ['ZZA'] } }));
    await new Promise((r) => setTimeout(r, 20));

    expect(host.querySelector('input[data-currency="ZZB"]'), 'ZZB stays shown').not.toBeNull();
    expect(host.querySelector('input[data-currency="ZZA"]'), 'ZZA is not shown').toBeNull();
  });

  it("P3: MAIN read held, switch to B02 (fine), then MAIN read fails -> B02 rows stay, no alert", async () => {
    const [branch, setBranch] = createSignal('MAIN');
    let resolveMain!: (res: Response) => void;
    const mainPromise = new Promise<Response>((res) => {
      resolveMain = res;
    });

    stubServer({
      views: {
        admin_cash: async (params) => {
          const b = params?.get('branch');
          if (b === 'MAIN') {
            return mainPromise;
          }
          return json(200, { version: 1, changed: true, data: { ...SAMPLE_CASH, currencies: ['ZZB'] } });
        },
      },
    });

    mount(() => branch());

    setBranch('B02');
    await vi.waitFor(() => expect(host.querySelector('input[data-currency="ZZB"]')).not.toBeNull());

    // MAIN fails late
    resolveMain(json(500, { error: { code: 'serverError', message: 'MAIN failed' } }));
    await new Promise((r) => setTimeout(r, 20));

    expect(host.querySelector('input[data-currency="ZZB"]'), 'B02 rows stay').not.toBeNull();
    expect(getAlertText(), 'no alert from older dropped read').toBeNull();
  });

  it('P4: MAIN fine -> switch to B02, whose read fails -> B02 rows cleared, alert error.serverError', async () => {
    const [branch, setBranch] = createSignal('MAIN');
    stubServer({
      views: {
        admin_cash: async (params) => {
          const b = params?.get('branch');
          if (b === 'MAIN') {
            return json(200, { version: 1, changed: true, data: { ...SAMPLE_CASH, currencies: ['THB'] } });
          }
          return json(500, { error: { code: 'serverError', message: 'B02 failed' } });
        },
      },
    });

    mount(() => branch());
    await vi.waitFor(() => expect(host.querySelector('input[data-currency="THB"]')).not.toBeNull());

    setBranch('B02');
    await vi.waitFor(() => expect(getAlertText()).toBe(th['error.serverError']));

    expect(host.querySelector('input[data-currency="THB"]'), 'register currencies cleared').toBeNull();
  });

  it('P5: cash_transfer_policy fails on first load, then a branch switch reads register fine -> policy alert stays', async () => {
    const [branch, setBranch] = createSignal('MAIN');
    stubServer({
      views: {
        cash_transfer_policy: async () => json(500, { error: { code: 'serverError', message: 'policy failed' } }),
        admin_cash: async () => json(200, { version: 1, changed: true, data: SAMPLE_CASH }),
      },
    });

    mount(() => branch());
    await vi.waitFor(() => expect(getAlertText()).toBe(th['error.serverError']));

    setBranch('B02');
    await vi.waitFor(() => expect(host.querySelector('input[data-currency="THB"]')).not.toBeNull());

    // The policy-read alert stays (it is refusal, not registerError)
    expect(getAlertText()).toBe(th['error.serverError']);
  });
});
