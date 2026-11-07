// OL-6 wave B (cust-rate-alerts-expiry, audit #564 issuecomment-6702970101): owner ruling A (#564 issuecomment-6703555417): a date field; "expires on" is the end of that day in the branch timezone, as RFC 3339.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { makeMyRateAlertsData, makePublishedLineDto, makePublishedRatesData } from '../../../shared/testing/fixtures';
import { RateAlertsScreen } from './RateAlerts';

const CREATE_PATH = '/api/app/rate-alerts';

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function harness(zones: { main: string; second: string }, createReply: () => Response) {
  const sent: Record<string, unknown>[] = [];
  const branches = {
    branches: [
      { id: 'br-main', code: 'MAIN', name: 'Head Office', address: null, openingHours: null, timezone: zones.main, bookingEnabled: true, queueOpen: true },
      { id: 'br-2', code: 'B2', name: 'Second', address: null, openingHours: null, timezone: zones.second, bookingEnabled: true, queueOpen: true },
    ],
  };
  const rates = makePublishedRatesData({
    lines: [makePublishedLineDto({ currency: 'USD', group: 'main', lineId: 'l-1', unitSize: '1' })],
  });
  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const method = (init?.method ?? 'GET').toUpperCase();
    const u = new URL(input, 'http://localhost');
    if (method === 'GET' && u.pathname === '/api/public/orgs/tk/branches') return json(branches);
    if (method === 'GET' && u.pathname.endsWith('/rates')) return json(rates);
    if (method === 'GET' && u.pathname === '/api/v3/views/my_rate_alerts') {
      return json({ version: 1, changed: true, data: makeMyRateAlertsData({ alerts: [] }) });
    }
    if (method === 'POST' && u.pathname === CREATE_PATH) {
      sent.push(JSON.parse(typeof init?.body === 'string' ? init.body : '{}'));
      return createReply();
    }
    return json({ error: { code: 'notFound', message: 'Not found' } }, 404);
  });
  return { api: new ApiClient(fetchFn), fetchFn, sent };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

describe('cust-rate-alerts-expiry', () => {
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
  });

  async function mount(
    main: string,
    second = 'Asia/Bangkok',
    createReply: () => Response = () => json({ id: 'al-1', version: 1 }),
  ) {
    const h = harness({ main, second }, createReply);
    dispose = render(
      () => <RateAlertsScreen client={h.api} fetch={h.fetchFn} org="tk" defaultBranch="br-main" />,
      host,
    );
    await vi.waitFor(() => {
      expect(host.querySelector<HTMLSelectElement>('#alert-branch')?.value).toBe('br-main');
    });
    await flush();
    return h;
  }

  function fill(selector: string, value: string): void {
    const input = host.querySelector<HTMLInputElement>(selector)!;
    input.value = value;
    input.dispatchEvent(new Event('input', { bubbles: true }));
  }

  function submit(): void {
    host
      .querySelector<HTMLFormElement>('form.xc-create-alert-form')!
      .dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  }

  async function sentFor(main: string, date: string): Promise<unknown> {
    const h = await mount(main);
    fill('#alert-target', '35.50');
    fill('#alert-expires', date);
    submit();
    await vi.waitFor(() => expect(h.sent).toHaveLength(1));
    return h.sent[0]!.expiresAt;
  }

  it('EX-1 the field is a date input', async () => {
    await mount('Asia/Bangkok');
    expect(host.querySelector<HTMLInputElement>('#alert-expires')?.type).toBe('date');
  });

  it('EX-2 empty: no expiresAt in the body', async () => {
    const h = await mount('Asia/Bangkok');
    fill('#alert-target', '35.50');
    submit();
    await vi.waitFor(() => expect(h.sent).toHaveLength(1));
    expect('expiresAt' in h.sent[0]!).toBe(false);
  });

  it('EX-3 a Bangkok branch: the end of that day at +07:00', async () => {
    expect(await sentFor('Asia/Bangkok', '2026-12-31')).toBe('2026-12-31T23:59:59+07:00');
  });

  it('EX-4 a whole offset is not assumed: Kathmandu is +05:45', async () => {
    expect(await sentFor('Asia/Kathmandu', '2026-12-31')).toBe('2026-12-31T23:59:59+05:45');
  });

  it('EX-5 UTC is written +00:00', async () => {
    expect(await sentFor('UTC', '2026-12-31')).toBe('2026-12-31T23:59:59+00:00');
  });

  it.each([
    ['winter', '2026-01-15', '-05:00'],
    ['summer', '2026-07-15', '-04:00'],
    ['the day clocks go forward', '2026-03-08', '-04:00'],
    ['the day clocks go back', '2026-11-01', '-05:00'],
  ])('EX-6 the offset is the one at the end of that day: New York, %s', async (_name, date, offset) => {
    expect(await sentFor('America/New_York', date)).toBe(`${date}T23:59:59${offset}`);
  });

  it('EX-7 the zone is the chosen branch, not the first one', async () => {
    const h = await mount('Asia/Bangkok', 'Asia/Kathmandu');
    const branch = host.querySelector<HTMLSelectElement>('#alert-branch')!;
    branch.value = 'br-2';
    branch.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();
    fill('#alert-target', '35.50');
    fill('#alert-expires', '2026-12-31');
    submit();
    await vi.waitFor(() => expect(h.sent).toHaveLength(1));
    expect(h.sent[0]!.branch).toBe('br-2');
    expect(h.sent[0]!.expiresAt).toBe('2026-12-31T23:59:59+05:45');
  });

  it('EX-8 a past date goes to the server, and its refusal is worded at the field', async () => {
    const h = await mount('Asia/Bangkok', 'Asia/Bangkok', () =>
      json({ error: { code: 'expiresAtNotInFuture', message: 'expires_at must be in the future' } }, 400),
    );
    fill('#alert-target', '35.50');
    fill('#alert-expires', '2020-01-01');
    submit();
    await vi.waitFor(() => {
      expect(host.querySelector('[data-field-error="expiresAt"]')?.textContent).toBe(
        sharedTh['input.code.expiresAtNotInFuture'],
      );
    });
    expect(h.sent[0]!.expiresAt).toBe('2020-01-01T23:59:59+07:00');
  });

  it.each([
    ['empty', ''],
    ['not a zone', 'Not/AZone'],
  ])('EX-9 a branch with no usable timezone (%s): no guess and no request, a line at the field', async (_name, zone) => {
    const h = await mount(zone);
    fill('#alert-target', '35.50');
    fill('#alert-expires', '2026-12-31');
    submit();
    await vi.waitFor(() => {
      expect(host.querySelector('[data-field-error="expiresAt"]')?.textContent).toBeTruthy();
    });
    await flush();
    expect(h.sent).toHaveLength(0);
  });

  it('EX-10 after a successful create the field is empty', async () => {
    const h = await mount('Asia/Bangkok');
    fill('#alert-target', '35.50');
    fill('#alert-expires', '2026-12-31');
    submit();
    await vi.waitFor(() => expect(h.sent).toHaveLength(1));
    await flush();
    expect(host.querySelector<HTMLInputElement>('#alert-expires')?.value).toBe('');
  });
});
