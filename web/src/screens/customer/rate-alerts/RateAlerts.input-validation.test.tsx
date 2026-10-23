// #4923 (T23) up-front tests: the rate-alert create form checks the target as the server does
// (`numeric(20, 8)`, positive; rate_alert_create.rs:92-96), offers only the currencies the chosen branch publishes
// (its public rates, `GET /api/public/orgs/{org}/branches/{code}/rates`), and words every refusal through
// `fieldError` (design web-input-validation.md §2) at its field.
// A line at a field is `[data-field-error="<field>"]` (the convention of RatesPolicyCard / CashLimits):
// `target`, `currency`, `expiresAt`, `branch`.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import { fill, setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import {
  makeMyRateAlertsData,
  makePublishedLineDto,
  makePublishedRatesData,
} from '../../../shared/testing/fixtures';
import * as table from './i18n';
import { RateAlertsScreen } from './RateAlerts';

const THAI_SCRIPT = /[฀-๿]/;
const CREATE_PATH = '/api/app/rate-alerts';

interface Sent {
  body: Record<string, unknown>;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

// Two branches: the id is the key the create body sends; the public rates path is keyed by code.
const BRANCHES = {
  branches: [
    { id: 'br-main', code: 'MAIN', name: 'Head Office' },
    { id: 'br-2', code: 'B2', name: 'Second' },
  ],
};

const RATES_BY_CODE: Record<string, unknown> = {
  MAIN: makePublishedRatesData({
    lines: [
      makePublishedLineDto({ currency: 'USD', group: 'large', lineId: 'l-1', unitSize: '1' }),
      makePublishedLineDto({ currency: 'USD', group: 'small', lineId: 'l-2', unitSize: '1' }),
      makePublishedLineDto({ currency: 'EUR', group: 'main', lineId: 'l-3', unitSize: '1' }),
    ],
  }),
  B2: makePublishedRatesData({
    lines: [makePublishedLineDto({ currency: 'JPY', group: 'main', lineId: 'l-4', unitSize: '100' })],
  }),
};

function harness(createReply: () => Response) {
  const sent: Sent[] = [];
  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const method = (init?.method ?? 'GET').toUpperCase();
    const u = new URL(input, 'http://localhost');
    if (method === 'GET' && u.pathname === '/api/public/host') return json({ org: 'tk' });
    if (method === 'GET' && u.pathname === '/api/public/orgs/tk/branches') return json(BRANCHES);
    const rates = /^\/api\/public\/orgs\/tk\/branches\/([^/]+)\/rates$/.exec(u.pathname);
    if (method === 'GET' && rates) {
      const data = RATES_BY_CODE[decodeURIComponent(rates[1]!)];
      return data ? json(data) : json({ error: { code: 'notFound', message: 'Not found' } }, 404);
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/my_rate_alerts') {
      return json({ version: 1, changed: true, data: makeMyRateAlertsData({ alerts: [] }) });
    }
    if (method === 'POST' && u.pathname === CREATE_PATH) {
      sent.push({ body: JSON.parse(typeof init?.body === 'string' ? init.body : '{}') });
      return createReply();
    }
    return json({ error: { code: 'notFound', message: 'Not found' } }, 404);
  });
  return { api: new ApiClient(fetchFn), fetchFn, sent };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('#4923 T23: RateAlerts create form input validation', () => {
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

  async function mount(createReply: () => Response = () => json({ id: 'al-1', version: 1 })) {
    const h = harness(createReply);
    dispose = render(
      () => <RateAlertsScreen client={h.api} fetch={h.fetchFn} org="tk" defaultBranch="br-main" />,
      host,
    );
    await vi.waitFor(() => {
      expect(host.querySelector('#alert-target')).not.toBeNull();
      expect(host.querySelector<HTMLSelectElement>('#alert-branch')?.value).toBe('br-main');
    });
    await flush();
    return h;
  }

  function fillTarget(value: string): void {
    const input = host.querySelector<HTMLInputElement>('#alert-target')!;
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function fillExpiry(value: string): void {
    const input = host.querySelector<HTMLInputElement>('#alert-expires')!;
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function submit(): void {
    host
      .querySelector<HTMLFormElement>('form.xc-create-alert-form')!
      .dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  }

  function lineAt(field: string): string {
    return host.querySelector(`[data-field-error="${field}"]`)?.textContent?.trim() ?? '';
  }

  function currencyOptions(): string[] {
    const el = host.querySelector('#alert-currency');
    return el ? [...el.querySelectorAll('option')].map((o) => o.value).filter((v) => v !== '') : [];
  }

  it('RA1: target 35.123456789 (9 decimals; the server takes at most 8) → a worded line at the target; no request', async () => {
    const h = await mount();
    fillTarget('35.123456789');
    submit();
    await flush();

    expect(h.sent).toHaveLength(0);
    expect(lineAt('target')).not.toBe('');
    expect(lineAt('target')).toMatch(THAI_SCRIPT);
  });

  it("RA2: currency is a select of the chosen branch's published codes, it follows the branch, and the pick is sent", async () => {
    const h = await mount();

    await vi.waitFor(() => {
      const el = host.querySelector('#alert-currency');
      expect(el?.tagName).toBe('SELECT');
      expect([...currencyOptions()].sort()).toEqual(['EUR', 'USD']);
    });
    expect(host.querySelector('input#alert-currency')).toBeNull();

    const branchSelect = host.querySelector<HTMLSelectElement>('#alert-branch')!;
    branchSelect.value = 'br-2';
    branchSelect.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => {
      expect(currencyOptions()).toEqual(['JPY']);
    });

    branchSelect.value = 'br-main';
    branchSelect.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => {
      expect([...currencyOptions()].sort()).toEqual(['EUR', 'USD']);
    });
    const currency = host.querySelector<HTMLSelectElement>('#alert-currency')!;
    currency.value = 'EUR';
    currency.dispatchEvent(new Event('change', { bubbles: true }));
    fillTarget('35.50');
    submit();

    await vi.waitFor(() => expect(h.sent).toHaveLength(1));
    expect(h.sent[0]!.body).toMatchObject({ branch: 'br-main', currency: 'EUR', target: '35.50' });
  });

  it('RA3: server 400 expiresAtNotInFuture → its input.code wording at the expiry field, never the English server text', async () => {
    await mount(() =>
      json(
        { error: { code: 'expiresAtNotInFuture', message: 'expires_at must be in the future' } },
        400,
      ),
    );
    fillTarget('35.50');
    fillExpiry('2020-01-01T00:00:00Z');
    submit();

    await vi.waitFor(() => {
      expect(lineAt('expiresAt')).toBe(sharedTh['input.code.expiresAtNotInFuture']);
    });
    expect(host.textContent).not.toContain('expires_at must be in the future');
  });

  it('RA4: server 400 validationFailed with field target → "<target label> ไม่ถูกต้อง" at the target', async () => {
    await mount(() =>
      json(
        { error: { code: 'validationFailed', field: 'target', message: 'ข้อมูลไม่ถูกต้อง' } },
        400,
      ),
    );
    fillTarget('35.50');
    submit();

    await vi.waitFor(() => {
      expect(lineAt('target')).toBe(fill(sharedTh['input.invalid'], { field: table.th.target }));
    });
  });
});
