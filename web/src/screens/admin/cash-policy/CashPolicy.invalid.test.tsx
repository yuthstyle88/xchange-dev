// CA-9c (#2131): invalid stored limits notice and save field-specific refusals on CashPolicy.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { CashPolicy } from './CashPolicy';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { th as screenTh, en as screenEn } from './i18n';
import type { AdminCashData } from '../../../shared/api/generated/admin_cash';
import type { CashTransferPolicyData } from '../../../shared/api/generated/cash_transfer_policy';

const VALID_POLICY: CashTransferPolicyData = {
  version: 'v-0001',
  limits: { USD: '300.00', EUR: '12.5' },
  valid: true,
  invalidEntries: [],
  setBy: 'u-exec',
  createdAt: '2026-10-12T01:00:00Z',
};

const INVALID_POLICY: CashTransferPolicyData = {
  version: 'v-0002',
  limits: { EUR: '500' },
  valid: false,
  invalidEntries: [{ currency: 'USD', raw: '12.345', reason: 'too many decimal places' }],
  setBy: 'u-exec',
  createdAt: '2026-10-12T02:00:00Z',
};

const SAMPLE_CASH: AdminCashData = {
  openShifts: [],
  stock: [],
  stats: null,
  reference: { withReference: 0, withoutReference: 0, spread: '0.00', percent: '0.00' },
  currencies: ['THB', 'USD', 'EUR'],
  lastCounted: null,
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stubServer(overrides: {
  policy?: CashTransferPolicyData | null | Array<CashTransferPolicyData | null>;
  save?: (body: unknown) => Promise<Response>;
} = {}) {
  const policies = Array.isArray(overrides.policy)
    ? [...overrides.policy]
    : [overrides.policy === undefined ? VALID_POLICY : overrides.policy];
  let readIdx = 0;
  const saveFn = overrides.save ?? (async () => json(200, VALID_POLICY));

  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const [path] = url.split('?');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && path === '/api/v3/views/cash_transfer_policy') {
        const data = policies[Math.min(readIdx, policies.length - 1)] ?? null;
        readIdx++;
        return json(200, { version: 1, changed: true, data });
      }
      if (method === 'GET' && path === '/api/v3/views/admin_cash') {
        return json(200, { version: 1, changed: true, data: SAMPLE_CASH });
      }
      if (method === 'PUT' && path === '/api/cash-transfer-policy') {
        const body = init?.body ? JSON.parse(String(init.body)) : {};
        return saveFn(body);
      }
      return json(404, { error: { code: 'notFound', message: 'not found' } });
    }),
  );
}

function mount() {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-31' });
  const [shown] = createSignal(true);
  dispose = render(() => <CashPolicy branch={branch} period={period} shown={shown} />, host);
}

describe('CA-9c (#2131): CashPolicy invalid entries and field-specific save refusals', () => {
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

  it('row 1: view valid: false renders invalidEntries (currency, raw, reason), parsed EUR 500 in form, and saving-replaces sentence', async () => {
    stubServer({ policy: INVALID_POLICY });
    mount();

    await vi.waitFor(() => {
      const eurInput = host.querySelector<HTMLInputElement>('input[data-currency="EUR"]');
      expect(eurInput?.value).toBe('500');
    });

    const usdInput = host.querySelector<HTMLInputElement>('input[data-currency="USD"]');
    expect(usdInput?.value).toBe('');

    const status = host.querySelector('[role="status"]');
    expect(status, 'status section for invalid entries').not.toBeNull();
    const statusText = status!.textContent ?? '';
    expect(statusText).toContain('USD');
    expect(statusText).toContain('12.345');
    expect(statusText).toContain('too many decimal places');
    expect(statusText).toContain(screenTh.invalidPolicyNote);
  });

  it('row 2: view valid: true renders no invalid list and no saving-replaces sentence', async () => {
    stubServer({ policy: [VALID_POLICY, INVALID_POLICY, VALID_POLICY] });
    mount();

    await vi.waitFor(() => {
      const usdInput = host.querySelector<HTMLInputElement>('input[data-currency="USD"]');
      expect(usdInput?.value).toBe('300.00');
    });

    expect(host.querySelector('[role="status"]')).toBeNull();
    expect(host.textContent ?? '').not.toContain(screenTh.invalidPolicyNote);

    // Save once -> re-reads INVALID_POLICY (valid: false -> status shown)
    host.querySelector<HTMLButtonElement>('button[data-action="save"]')!.click();
    await vi.waitFor(() => expect(host.querySelector('[role="status"]')).not.toBeNull());

    // Save again -> re-reads VALID_POLICY (valid: true, invalidEntries: [] -> status hidden)
    host.querySelector<HTMLButtonElement>('button[data-action="save"]')!.click();
    await vi.waitFor(() => expect(host.querySelector('[role="status"]')).toBeNull());
    expect(host.textContent ?? '').not.toContain(screenTh.invalidPolicyNote);
  });

  it('row 3: save -> 400 validationFailed field limits shows screen own limits sentence beside form, not serverError or generic text', async () => {
    const genericText = 'generic validation failed text';
    stubServer({
      save: async () =>
        json(400, {
          error: { code: 'validationFailed', field: 'limits', message: genericText },
        }),
    });

    mount();
    await vi.waitFor(() => expect(host.querySelector('button[data-action="save"]')).not.toBeNull());

    host.querySelector<HTMLButtonElement>('button[data-action="save"]')!.click();

    await vi.waitFor(() => expect(host.querySelector('[role="alert"]')).not.toBeNull());
    const alert = host.querySelector('[role="alert"]')!;
    expect(alert.textContent?.trim()).toBe(screenTh['error.limits']);
    expect(alert.textContent).not.toContain(sharedTh['error.serverError']);
    expect(alert.textContent).not.toContain(genericText);
    expect(host.querySelector('form')?.contains(alert)).toBe(true);

    // Also verify EN
    dispose?.();
    dispose = undefined;
    host.innerHTML = '';
    setLang('en');

    mount();
    await vi.waitFor(() => expect(host.querySelector('button[data-action="save"]')).not.toBeNull());
    host.querySelector<HTMLButtonElement>('button[data-action="save"]')!.click();

    await vi.waitFor(() => expect(host.querySelector('[role="alert"]')).not.toBeNull());
    expect(host.querySelector('[role="alert"]')!.textContent?.trim()).toBe(screenEn['error.limits']);
  });

  it('row 4: save -> 400 validationFailed field currency shows screen own currency sentence beside form', async () => {
    const genericText = 'generic currency error';
    stubServer({
      save: async () =>
        json(400, {
          error: { code: 'validationFailed', field: 'currency', message: genericText },
        }),
    });

    mount();
    await vi.waitFor(() => expect(host.querySelector('button[data-action="save"]')).not.toBeNull());

    host.querySelector<HTMLButtonElement>('button[data-action="save"]')!.click();

    await vi.waitFor(() => expect(host.querySelector('[role="alert"]')).not.toBeNull());
    const alert = host.querySelector('[role="alert"]')!;
    expect(alert.textContent?.trim()).toBe(screenTh['error.currency']);
    expect(alert.textContent).not.toContain(sharedTh['error.serverError']);
    expect(alert.textContent).not.toContain(genericText);
    expect(host.querySelector('form')?.contains(alert)).toBe(true);

    // Also verify EN
    dispose?.();
    dispose = undefined;
    host.innerHTML = '';
    setLang('en');

    mount();
    await vi.waitFor(() => expect(host.querySelector('button[data-action="save"]')).not.toBeNull());
    host.querySelector<HTMLButtonElement>('button[data-action="save"]')!.click();

    await vi.waitFor(() => expect(host.querySelector('[role="alert"]')).not.toBeNull());
    expect(host.querySelector('[role="alert"]')!.textContent?.trim()).toBe(screenEn['error.currency']);
  });

  it('row 5: save -> 500 goes through wordedError (server error), even when field is present', async () => {
    let call = 0;
    stubServer({
      save: async () => {
        call++;
        if (call === 1) {
          return json(400, {
            error: { code: 'validationFailed', field: 'limits', message: 'bad limits' },
          });
        }
        return json(500, {
          error: { code: 'validationFailed', field: 'limits', message: 'Internal Server Error' },
        });
      },
    });

    mount();
    await vi.waitFor(() => expect(host.querySelector('button[data-action="save"]')).not.toBeNull());

    // First save: 400 field 'limits' words via screen's own sentence
    host.querySelector<HTMLButtonElement>('button[data-action="save"]')!.click();
    await vi.waitFor(() => expect(host.querySelector('[role="alert"]')?.textContent?.trim()).toBe(screenTh['error.limits']));

    // Second save: 500 with field 'limits' goes through wordedError (serverError), not field wording
    host.querySelector<HTMLButtonElement>('button[data-action="save"]')!.click();
    await vi.waitFor(() => expect(host.querySelector('[role="alert"]')?.textContent?.trim()).toBe(sharedTh['error.serverError']));
  });
});
