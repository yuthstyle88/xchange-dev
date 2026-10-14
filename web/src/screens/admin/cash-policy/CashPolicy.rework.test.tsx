import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { CashPolicy } from './CashPolicy';
import { errorText, setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th } from './i18n';
import type { AdminCashData } from '../../../shared/api/generated/admin_cash';
import type { CashTransferPolicyData } from '../../../shared/api/generated/cash_transfer_policy';

const SAMPLE_POLICY: CashTransferPolicyData = {
  version: 'v-0001',
  limits: { USD: '300.00', EUR: '12.5' },
  setBy: 'u-exec',
  createdAt: '2026-10-12T01:00:00Z',
};

const SAMPLE_CASH: AdminCashData = {
  shift: null,
  stock: [],
  stats: null,
  reference: { withReference: 0, withoutReference: 0, spread: '0.00', percent: '0.00' },
  currencies: ['THB', 'USD', 'EUR'],
  lastCounted: null,
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let viewHandlers: Record<string, () => Promise<Response>>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stubViews(overrides: Record<string, () => Promise<Response>> = {}) {
  viewHandlers = {
    cash_transfer_policy: async () => json(200, { version: 1, changed: true, data: SAMPLE_POLICY }),
    admin_cash: async () => json(200, { version: 1, changed: true, data: SAMPLE_CASH }),
    ...overrides,
  };

  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const path = url.split('?')[0]!;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET' && path.startsWith('/api/v3/views/')) {
        const viewName = path.slice('/api/v3/views/'.length);
        if (viewHandlers[viewName]) {
          return viewHandlers[viewName]();
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }
      return json(200, {});
    }),
  );
}

function mount(branch = 'MAIN') {
  const [b] = createSignal(branch);
  const [p] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [s] = createSignal(true);
  dispose = render(() => <CashPolicy branch={b} period={p} shown={s} />, host);
}

function q<T extends Element = HTMLElement>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

function must<T extends Element = HTMLElement>(sel: string): T {
  const el = q<T>(sel);
  expect(el, sel).not.toBeNull();
  return el!;
}

describe('#1718 CashPolicy read error handling', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
  });

  it('1. admin_cash read failure shows worded error', async () => {
    stubViews({
      admin_cash: async () =>
        json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }),
    });

    mount();
    await vi.waitFor(() => expect(q('[role="alert"]')).not.toBeNull());

    const alert = must('[role="alert"]');
    expect(alert.textContent?.trim()).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
  });

  it('2. admin_cash refusal with known code shows worded error', async () => {
    stubViews({
      admin_cash: async () =>
        json(403, { error: { code: 'roleNotAllowedHere', message: 'refused' } }),
    });

    mount();
    await vi.waitFor(() => expect(q('[role="alert"]')).not.toBeNull());

    const alert = must('[role="alert"]');
    expect(alert.textContent?.trim()).toBe(th['error.roleNotAllowedHere']);
  });

  it('3. network failure shows worded generic message in TH and EN', async () => {
    // In Thai:
    stubViews({
      admin_cash: async () => {
        throw new TypeError('Failed to fetch');
      },
    });

    mount();
    await vi.waitFor(() => expect(q('[role="alert"]')).not.toBeNull());

    let alert = must('[role="alert"]');
    expect(alert.textContent?.trim()).toBe(sharedTh['error.network']);
    expect(alert.textContent).not.toContain('Failed to fetch');

    // In English:
    dispose?.();
    dispose = undefined;
    host.innerHTML = '';
    setLang('en');

    mount();
    await vi.waitFor(() => expect(q('[role="alert"]')).not.toBeNull());

    alert = must('[role="alert"]');
    expect(alert.textContent?.trim()).toBe(sharedEn['error.network']);
    expect(alert.textContent).not.toContain('Failed to fetch');
  });

  it('4. unknown refusal code shows server message (W10 point 4)', async () => {
    stubViews({
      admin_cash: async () =>
        json(400, { error: { code: 'UNKNOWN_REFUSAL_CODE', message: 'raw wire message' } }),
    });

    mount();
    await vi.waitFor(() => expect(q('[role="alert"]')).not.toBeNull());

    const alert = must('[role="alert"]');
    expect(alert.textContent?.trim()).toBe('raw wire message');
    expect(alert.textContent).not.toContain('UNKNOWN_REFUSAL_CODE');
  });

  it('5. currencies and inputs stay visible when policy was read, but error shows', async () => {
    stubViews({
      admin_cash: async () =>
        json(500, { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } }),
    });

    mount();
    await vi.waitFor(() => expect(q('[role="alert"]')).not.toBeNull());

    // Policy defines USD and EUR, so those currencies should still be present in the table
    expect(q('input[data-currency="USD"]')).not.toBeNull();
    expect(q('input[data-currency="EUR"]')).not.toBeNull();

    const alert = must('[role="alert"]');
    expect(alert.textContent?.trim()).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
  });
});
