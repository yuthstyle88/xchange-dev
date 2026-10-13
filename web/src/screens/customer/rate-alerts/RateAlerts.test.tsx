import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MyRateAlertItem } from '../../../shared/api/generated/my_rate_alerts';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import { lang, setAudience, setLang } from '../../../shared/i18n';
import { RateAlertsScreen } from './RateAlerts';
import * as table from './i18n';

type Texts = Record<string, string>;
const TH = table.th as unknown as Texts;
const EN = table.en as unknown as Texts;
const ZH = table.zh as unknown as Texts;

type NodeFs = {
  readFileSync: (p: string, enc?: string) => string;
  existsSync: (p: string) => boolean;
};

let fs: NodeFs;
let dir = '';

beforeAll(async () => {
  const fsName = 'node:fs';
  const pathName = 'node:path';
  const urlName = 'node:url';
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  const path = (await import(/* @vite-ignore */ pathName)) as unknown as { dirname: (p: string) => string };
  const { fileURLToPath } = (await import(/* @vite-ignore */ urlName)) as unknown as {
    fileURLToPath: (u: string) => string;
  };
  dir = path.dirname(fileURLToPath(import.meta.url)) + '/';
});

interface RecordedCall {
  method: string;
  url: string;
  headers: Record<string, string>;
  body: unknown;
}

const SAMPLE_BRANCHES: PublicBranchesData = {
  branches: [
    {
      code: 'XC-B01',
      name: 'สาขาสยามพารากอน',
      address: '991 ถ.พระราม 1',
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
    {
      code: 'XC-B02',
      name: 'สาขาสนามบินดอนเมือง',
      address: null,
      openingHours: null,
      timezone: 'Asia/Bangkok',
      bookingEnabled: false,
      queueOpen: false,
    },
  ],
};

function mockBackend(options: {
  initialAlerts?: MyRateAlertItem[];
  createResponse?: { id: string; version: number } | { error: { code: string; message: string } };
  pauseResponse?: { id: string; version: number } | { error: { code: string; message: string } };
  resumeResponse?: { id: string; version: number } | { error: { code: string; message: string } };
  deleteResponse?: { id: string; version: number } | { error: { code: string; message: string } };
  viewFailOnce?: boolean;
  branchesData?: PublicBranchesData;
  org?: string;
}) {
  let alerts = options.initialAlerts ? [...options.initialAlerts] : [];
  let viewFail = options.viewFailOnce ?? false;
  const calls: RecordedCall[] = [];
  const org = options.org ?? 'XC-ORG';
  const branchesData = options.branchesData ?? SAMPLE_BRANCHES;

  const fetchFn = vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
    const method = init?.method ?? 'GET';
    const headerObj: Record<string, string> = {};
    if (init?.headers) {
      new Headers(init.headers).forEach((v, k) => {
        headerObj[k.toLowerCase()] = v;
      });
    }

    let parsedBody: unknown = undefined;
    if (init?.body && typeof init.body === 'string') {
      try {
        parsedBody = JSON.parse(init.body);
      } catch {
        parsedBody = init.body;
      }
    }

    calls.push({ method, url, headers: headerObj, body: parsedBody });

    // GET /api/public/host
    if (url === '/api/public/host') {
      return new Response(JSON.stringify({ org }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    // GET /api/public/orgs/{org}/branches
    if (url.includes(`/api/public/orgs/${org}/branches`)) {
      return new Response(JSON.stringify(branchesData), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    // GET /api/v3/views/my_rate_alerts
    if (url.includes('/api/v3/views/my_rate_alerts')) {
      if (viewFail) {
        viewFail = false;
        return new Response(
          JSON.stringify({ error: { code: 'serverError', message: 'Failed to fetch' } }),
          { status: 500, headers: { 'Content-Type': 'application/json' } },
        );
      }
      return new Response(
        JSON.stringify({
          version: 1,
          changed: true,
          data: { alerts },
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }

    // POST /api/app/rate-alerts (create)
    if (method === 'POST' && url === '/api/app/rate-alerts') {
      if (options.createResponse && 'error' in options.createResponse) {
        return new Response(JSON.stringify(options.createResponse), {
          status: 400,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      const b = parsedBody as {
        branch: string;
        currency: string;
        customerSide: string;
        condition: string;
        target: string;
      };
      const createdItem: MyRateAlertItem = {
        id: 'alert-new-1',
        version: 1,
        branch: b.branch,
        currency: b.currency,
        customerSide: b.customerSide,
        condition: b.condition,
        target: b.target,
        unitSize: '1',
        currentRate: null,
        ratePublishedAt: null,
        enabled: true,
      };
      alerts = [createdItem, ...alerts];
      return new Response(
        JSON.stringify(options.createResponse ?? { id: 'alert-new-1', version: 1 }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }

    // POST /api/app/rate-alerts/{id}/pause
    if (method === 'POST' && url.includes('/pause')) {
      if (options.pauseResponse && 'error' in options.pauseResponse) {
        return new Response(JSON.stringify(options.pauseResponse), {
          status: 400,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      const idMatch = url.match(/\/rate-alerts\/([^/]+)\/pause/);
      if (idMatch) {
        const id = decodeURIComponent(idMatch[1]);
        alerts = alerts.map((a) => (a.id === id ? { ...a, enabled: false, version: a.version + 1 } : a));
      }
      return new Response(
        JSON.stringify(options.pauseResponse ?? { id: 'alert-1', version: 2 }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }

    // POST /api/app/rate-alerts/{id}/resume
    if (method === 'POST' && url.includes('/resume')) {
      if (options.resumeResponse && 'error' in options.resumeResponse) {
        return new Response(JSON.stringify(options.resumeResponse), {
          status: 400,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      const idMatch = url.match(/\/rate-alerts\/([^/]+)\/resume/);
      if (idMatch) {
        const id = decodeURIComponent(idMatch[1]);
        alerts = alerts.map((a) => (a.id === id ? { ...a, enabled: true, version: a.version + 1 } : a));
      }
      return new Response(
        JSON.stringify(options.resumeResponse ?? { id: 'alert-1', version: 3 }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }

    // POST /api/app/rate-alerts/{id}/delete
    if (method === 'POST' && url.includes('/delete')) {
      if (options.deleteResponse && 'error' in options.deleteResponse) {
        return new Response(JSON.stringify(options.deleteResponse), {
          status: 400,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      const idMatch = url.match(/\/rate-alerts\/([^/]+)\/delete/);
      if (idMatch) {
        const id = decodeURIComponent(idMatch[1]);
        alerts = alerts.filter((a) => a.id !== id);
      }
      return new Response(
        JSON.stringify(options.deleteResponse ?? { id: 'alert-1', version: 4 }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }

    return new Response('{}', { status: 404 });
  });

  return { fetchFn, calls, getAlerts: () => alerts };
}

describe('AL-5 customer rate-alerts screen (design §4.4, §9.1 C11)', () => {
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
  });

  it('the picker lists the fetched branches from GET /api/public/host and GET /api/public/orgs/{org}/branches', async () => {
    const { fetchFn } = mockBackend({ initialAlerts: [] });
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      const branchSelect = host.querySelector('#alert-branch') as HTMLSelectElement | null;
      expect(branchSelect).not.toBeNull();
      const options = branchSelect?.querySelectorAll('option');
      expect(options?.length).toBe(2);
      expect(options?.[0].value).toBe('XC-B01');
      expect(options?.[0].textContent).toContain('XC-B01');
      expect(options?.[1].value).toBe('XC-B02');
      expect(options?.[1].textContent).toContain('XC-B02');
    });

    const branchSelect = host.querySelector('#alert-branch') as HTMLSelectElement;
    expect(branchSelect.value).toBe('XC-B01');
  });

  it('the create body branch is the one picked from the branch picker', async () => {
    const { fetchFn, calls } = mockBackend({ initialAlerts: [] });
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      const branchSelect = host.querySelector('#alert-branch') as HTMLSelectElement | null;
      expect(branchSelect?.querySelectorAll('option').length).toBe(2);
    });

    // Select second branch: XC-B02
    const branchSelect = host.querySelector('#alert-branch') as HTMLSelectElement;
    branchSelect.value = 'XC-B02';
    branchSelect.dispatchEvent(new Event('change', { bubbles: true }));

    const targetInput = host.querySelector('#alert-target') as HTMLInputElement;
    targetInput.value = '35.00';
    targetInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = host.querySelector('button[type="submit"]') as HTMLButtonElement;
    submitBtn.click();

    await vi.waitFor(() => {
      const createCall = calls.find((c) => c.method === 'POST' && c.url === '/api/app/rate-alerts');
      expect(createCall).toBeDefined();
      expect(createCall?.body).toEqual({
        branch: 'XC-B02',
        currency: 'USD',
        customerSide: 'CUSTOMER_BUY',
        condition: 'ABOVE',
        target: '35.00',
      });
    });
  });

  it('renders loading state initially and empty state when no alerts exist', async () => {
    const { fetchFn } = mockBackend({ initialAlerts: [] });
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-alerts-empty')).not.toBeNull();
      expect(host.querySelector('.xc-alerts-empty')?.textContent).toContain(TH.empty);
    });
  });

  it('renders alerts with side, unit, branch, and current rate with published time in words; when null, shows "no current rate"', async () => {
    const sampleAlerts: MyRateAlertItem[] = [
      {
        id: 'alert-1',
        version: 1,
        branch: 'XC-B01',
        currency: 'USD',
        customerSide: 'CUSTOMER_BUY',
        condition: 'ABOVE',
        target: '34.50',
        unitSize: '1.0000',
        currentRate: '34.20',
        ratePublishedAt: '2026-10-13T10:00:00Z',
        enabled: true,
      },
      {
        id: 'alert-2',
        version: 2,
        branch: 'XC-B02',
        currency: 'JPY',
        customerSide: 'CUSTOMER_SELL',
        condition: 'BELOW',
        target: '23.80',
        unitSize: '100.0000',
        currentRate: null,
        ratePublishedAt: null,
        enabled: false,
      },
    ];

    const { fetchFn } = mockBackend({ initialAlerts: sampleAlerts });
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      const cards = host.querySelectorAll('.xc-alert-card');
      expect(cards.length).toBe(2);
    });

    const card1 = host.querySelector('[data-alert-id="alert-1"]');
    expect(card1).not.toBeNull();
    // Customer side in customer words: "ฉันซื้อ"
    expect(card1?.querySelector('.xc-alert-side')?.textContent).toBe(TH.sideBuy);
    // Unit "per {unitSize}": "ต่อ 1"
    expect(card1?.querySelector('.xc-alert-unit')?.textContent).toContain('ต่อ 1');
    // Branch
    expect(card1?.querySelector('.xc-alert-branch-badge')?.textContent).toBe('XC-B01');
    // Target amount with MoneyText
    expect(card1?.querySelector('.xc-alert-target-amount')?.textContent).toContain('34.50 THB');
    // Status active
    expect(card1?.querySelector('.xc-badge')?.textContent).toBe(TH.statusActive);
    // Current rate and published time in words
    const rateEl1 = card1?.querySelector('.xc-alert-current-rate');
    expect(rateEl1).not.toBeNull();
    expect(rateEl1?.textContent).toContain(TH.currentRate);
    expect(rateEl1?.textContent).toContain('34.20 THB');
    const expectedTime1 = new Date('2026-10-13T10:00:00Z').toLocaleString('th-TH', { timeZone: 'Asia/Bangkok' });
    expect(rateEl1?.textContent).toContain(expectedTime1);
    expect(card1?.querySelector('.xc-alert-no-rate')).toBeNull();

    const card2 = host.querySelector('[data-alert-id="alert-2"]');
    expect(card2).not.toBeNull();
    // Customer side in customer words: "ฉันขาย"
    expect(card2?.querySelector('.xc-alert-side')?.textContent).toBe(TH.sideSell);
    // Unit "per {unitSize}": "ต่อ 100"
    expect(card2?.querySelector('.xc-alert-unit')?.textContent).toContain('ต่อ 100');
    // Branch
    expect(card2?.querySelector('.xc-alert-branch-badge')?.textContent).toBe('XC-B02');
    // Target amount
    expect(card2?.querySelector('.xc-alert-target-amount')?.textContent).toContain('23.80 THB');
    // Status paused
    expect(card2?.querySelector('.xc-badge')?.textContent).toBe(TH.statusPaused);
    // Null current rate and ratePublishedAt: shows "no current rate"
    expect(card2?.querySelector('.xc-alert-current-rate')).toBeNull();
    const noRateEl2 = card2?.querySelector('.xc-alert-no-rate');
    expect(noRateEl2).not.toBeNull();
    expect(noRateEl2?.textContent).toBe(TH.noCurrentRate);
  });

  it('creates an alert with Idempotency-Key and refreshes the list', async () => {
    const { fetchFn, calls } = mockBackend({ initialAlerts: [] });
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-alerts-empty')).not.toBeNull();
    });

    const targetInput = host.querySelector('#alert-target') as HTMLInputElement;
    targetInput.value = '35.00';
    targetInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = host.querySelector('button[type="submit"]') as HTMLButtonElement;
    submitBtn.click();

    await vi.waitFor(() => {
      expect(host.querySelectorAll('.xc-alert-card').length).toBe(1);
    });

    const createCall = calls.find((c) => c.method === 'POST' && c.url === '/api/app/rate-alerts');
    expect(createCall).toBeDefined();
    expect(createCall?.headers['idempotency-key']).toBeDefined();
    expect(createCall?.headers['idempotency-key'].length).toBeGreaterThan(0);
    expect(createCall?.body).toEqual({
      branch: 'XC-B01',
      currency: 'USD',
      customerSide: 'CUSTOMER_BUY',
      condition: 'ABOVE',
      target: '35.00',
    });

    // Form target input was cleared on success
    expect(targetInput.value).toBe('');
  });

  it('pause, resume, and delete commands call backend with expectedVersion and Idempotency-Key', async () => {
    const initial: MyRateAlertItem[] = [
      {
        id: 'alert-1',
        version: 5,
        branch: 'XC-B01',
        currency: 'USD',
        customerSide: 'CUSTOMER_BUY',
        condition: 'ABOVE',
        target: '34.00',
        unitSize: '1',
        currentRate: '34.10',
        ratePublishedAt: '2026-10-13T10:00:00Z',
        enabled: true,
      },
    ];

    const { fetchFn, calls } = mockBackend({ initialAlerts: initial });
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-btn-pause')).not.toBeNull();
    });

    // 1. Pause
    const pauseBtn = host.querySelector('.xc-btn-pause') as HTMLButtonElement;
    pauseBtn.click();

    await vi.waitFor(() => {
      const pauseCall = calls.find((c) => c.url.includes('/pause'));
      expect(pauseCall).toBeDefined();
      expect(pauseCall?.body).toEqual({ expectedVersion: 5 });
      expect(pauseCall?.headers['idempotency-key']).toBeDefined();
      expect(host.querySelector('.xc-btn-resume')).not.toBeNull();
    });

    // 2. Resume
    const resumeBtn = host.querySelector('.xc-btn-resume') as HTMLButtonElement;
    resumeBtn.click();

    await vi.waitFor(() => {
      const resumeCall = calls.find((c) => c.url.includes('/resume'));
      expect(resumeCall).toBeDefined();
      expect(resumeCall?.body).toEqual({ expectedVersion: 6 });
      expect(resumeCall?.headers['idempotency-key']).toBeDefined();
      expect(host.querySelector('.xc-btn-pause')).not.toBeNull();
    });

    // 3. Delete
    const deleteBtn = host.querySelector('.xc-btn-delete') as HTMLButtonElement;
    deleteBtn.click();

    await vi.waitFor(() => {
      const deleteCall = calls.find((c) => c.url.includes('/delete'));
      expect(deleteCall).toBeDefined();
      expect(deleteCall?.body).toEqual({ expectedVersion: 7 });
      expect(deleteCall?.headers['idempotency-key']).toBeDefined();
      expect(host.querySelector('.xc-alerts-empty')).not.toBeNull();
    });
  });

  it('handles alertLimitReached: displays error message and keeps form inputs intact with same idempotency key', async () => {
    const { fetchFn, calls } = mockBackend({
      initialAlerts: [],
      createResponse: { error: { code: 'alertLimitReached', message: 'Maximum alerts limit reached' } },
    });

    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('#alert-branch option')).not.toBeNull();
    });

    const targetInput = host.querySelector('#alert-target') as HTMLInputElement;
    targetInput.value = '36.50';
    targetInput.dispatchEvent(new Event('input', { bubbles: true }));

    const submitBtn = host.querySelector('button[type="submit"]') as HTMLButtonElement;
    submitBtn.click();

    await vi.waitFor(() => {
      const err = host.querySelector('.xc-form-error');
      expect(err).not.toBeNull();
      expect(err?.textContent).toContain(TH.alertLimitReached);
    });

    // The form was kept!
    expect(targetInput.value).toBe('36.50');

    // First key
    const firstKey = calls[calls.length - 1].headers['idempotency-key'];

    // Submitting again (retry) reuses the same idempotency key
    submitBtn.click();

    await vi.waitFor(() => {
      expect(calls.filter((c) => c.method === 'POST' && c.url === '/api/app/rate-alerts').length).toBe(2);
    });

    const secondKey = calls[calls.length - 1].headers['idempotency-key'];
    expect(secondKey).toBe(firstKey);
  });

  it('renders error state on view failure and recovers via retry button', async () => {
    const { fetchFn } = mockBackend({
      initialAlerts: [
        {
          id: 'alert-recovered',
          version: 1,
          branch: 'XC-B01',
          currency: 'EUR',
          customerSide: 'CUSTOMER_BUY',
          condition: 'ABOVE',
          target: '38.00',
          unitSize: '1',
          currentRate: null,
          ratePublishedAt: null,
          enabled: true,
        },
      ],
      viewFailOnce: true,
    });

    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      const errEl = host.querySelector('.xc-alerts-error');
      expect(errEl).not.toBeNull();
      expect(errEl?.querySelector('.xc-alerts-retry-btn')).not.toBeNull();
    });

    const retryBtn = host.querySelector('.xc-alerts-retry-btn') as HTMLButtonElement;
    retryBtn.click();

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-alerts-error')).toBeNull();
      expect(host.querySelectorAll('.xc-alert-card').length).toBe(1);
    });
  });

  it('switches languages smoothly across TH, EN, and ZH with identical keys and displays both null and non-null rate texts', async () => {
    // Assert identical keys across TH, EN, and ZH
    const thKeys = Object.keys(TH).sort();
    const enKeys = Object.keys(EN).sort();
    const zhKeys = Object.keys(ZH).sort();
    expect(thKeys).toEqual(enKeys);
    expect(thKeys).toEqual(zhKeys);

    const sample: MyRateAlertItem[] = [
      {
        id: 'alert-lang-non-null',
        version: 1,
        branch: 'XC-B01',
        currency: 'USD',
        customerSide: 'CUSTOMER_BUY',
        condition: 'ABOVE',
        target: '34.00',
        unitSize: '1',
        currentRate: '33.90',
        ratePublishedAt: '2026-10-13T10:00:00Z',
        enabled: true,
      },
      {
        id: 'alert-lang-null',
        version: 1,
        branch: 'XC-B02',
        currency: 'JPY',
        customerSide: 'CUSTOMER_SELL',
        condition: 'BELOW',
        target: '23.80',
        unitSize: '100',
        currentRate: null,
        ratePublishedAt: null,
        enabled: false,
      },
    ];

    const { fetchFn } = mockBackend({ initialAlerts: sample });
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-alerts-title-group h1')?.textContent).toBe(TH.title);
      expect(host.querySelector('[data-alert-id="alert-lang-non-null"] .xc-alert-side')?.textContent).toBe(TH.sideBuy);
      expect(host.querySelector('[data-alert-id="alert-lang-non-null"] .xc-alert-unit')?.textContent).toContain('ต่อ 1');
      expect(host.querySelector('[data-alert-id="alert-lang-non-null"] .xc-alert-current-rate')?.textContent).toContain(TH.currentRate);
      expect(host.querySelector('[data-alert-id="alert-lang-null"] .xc-alert-no-rate')?.textContent).toBe(TH.noCurrentRate);
    });

    // Switch to EN
    setLang('en');
    expect(lang()).toBe('en');
    expect(host.querySelector('.xc-alerts-title-group h1')?.textContent).toBe(EN.title);
    expect(host.querySelector('[data-alert-id="alert-lang-non-null"] .xc-alert-side')?.textContent).toBe(EN.sideBuy);
    expect(host.querySelector('[data-alert-id="alert-lang-non-null"] .xc-alert-unit')?.textContent).toContain('per 1');
    expect(host.querySelector('[data-alert-id="alert-lang-non-null"] .xc-alert-current-rate')?.textContent).toContain(EN.currentRate);
    expect(host.querySelector('[data-alert-id="alert-lang-null"] .xc-alert-no-rate')?.textContent).toBe(EN.noCurrentRate);
    expect(host.querySelector('.xc-btn-pause')?.textContent).toBe(EN.pause);

    // Switch to ZH
    setLang('zh');
    expect(lang()).toBe('zh');
    expect(host.querySelector('.xc-alerts-title-group h1')?.textContent).toBe(ZH.title);
    expect(host.querySelector('[data-alert-id="alert-lang-non-null"] .xc-alert-side')?.textContent).toBe(ZH.sideBuy);
    expect(host.querySelector('[data-alert-id="alert-lang-non-null"] .xc-alert-unit')?.textContent).toContain('每 1');
    expect(host.querySelector('[data-alert-id="alert-lang-non-null"] .xc-alert-current-rate')?.textContent).toContain(ZH.currentRate);
    expect(host.querySelector('[data-alert-id="alert-lang-null"] .xc-alert-no-rate')?.textContent).toBe(ZH.noCurrentRate);
    expect(host.querySelector('.xc-btn-pause')?.textContent).toBe(ZH.pause);
  });

  it('renders a non-null currentRate with MoneyText and formatted published time, without no-rate text', async () => {
    const alerts: MyRateAlertItem[] = [
      {
        id: 'alert-has-rate',
        version: 1,
        branch: 'XC-B01',
        currency: 'USD',
        customerSide: 'CUSTOMER_BUY',
        condition: 'ABOVE',
        target: '35.00',
        unitSize: '1.0000',
        currentRate: '34.75',
        ratePublishedAt: '2026-10-13T12:00:00Z',
        enabled: true,
      },
    ];

    const { fetchFn } = mockBackend({ initialAlerts: alerts });
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-alert-id="alert-has-rate"]')).not.toBeNull();
    });

    const card = host.querySelector('[data-alert-id="alert-has-rate"]');
    const rateEl = card?.querySelector('.xc-alert-current-rate');
    expect(rateEl).not.toBeNull();
    expect(rateEl?.textContent).toContain(TH.currentRate);
    expect(rateEl?.textContent).toContain('34.75 THB');
    const expectedTime = new Date('2026-10-13T12:00:00Z').toLocaleString('th-TH', { timeZone: 'Asia/Bangkok' });
    expect(rateEl?.textContent).toContain(expectedTime);
    expect(card?.querySelector('.xc-alert-no-rate')).toBeNull();
  });

  it('renders a null currentRate showing i18n no-rate text, never 0 or an empty number', async () => {
    const alerts: MyRateAlertItem[] = [
      {
        id: 'alert-no-rate',
        version: 1,
        branch: 'XC-B02',
        currency: 'EUR',
        customerSide: 'CUSTOMER_SELL',
        condition: 'BELOW',
        target: '38.00',
        unitSize: '1.0000',
        currentRate: null,
        ratePublishedAt: null,
        enabled: true,
      },
    ];

    const { fetchFn } = mockBackend({ initialAlerts: alerts });
    dispose = render(() => <RateAlertsScreen fetch={fetchFn} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('[data-alert-id="alert-no-rate"]')).not.toBeNull();
    });

    const card = host.querySelector('[data-alert-id="alert-no-rate"]');
    // Must NOT render current rate element or amounts
    expect(card?.querySelector('.xc-alert-current-rate')).toBeNull();
    const noRateEl = card?.querySelector('.xc-alert-no-rate');
    expect(noRateEl).not.toBeNull();
    expect(noRateEl?.textContent).toBe(TH.noCurrentRate);
    // Never 0 or an empty number
    expect(noRateEl?.textContent).not.toContain('0');
    expect(noRateEl?.textContent).not.toContain('0.00');
  });

  it('main.tsx and app-rate-alerts.html fulfill the shell and gate contract', () => {
    // 1. HTML entry
    const htmlPath = dir + '../../../../pages/app-rate-alerts.html';
    expect(fs.existsSync(htmlPath), 'pages/app-rate-alerts.html exists').toBe(true);
    const html = fs.readFileSync(htmlPath, 'utf-8');
    expect(html).toContain('name="xc-path" content="/app/rate-alerts"');
    expect(html).toContain('name="xc-gate" content="anyone"');
    expect(html).toContain('src="/src/screens/customer/rate-alerts/main.tsx"');

    // 2. main.tsx
    const mainPath = dir + 'main.tsx';
    expect(fs.existsSync(mainPath), 'main.tsx exists').toBe(true);
    const mainSrc = fs.readFileSync(mainPath, 'utf-8');

    // LN-1: calls setAudience('customer')
    expect(mainSrc).toMatch(/setAudience\(['"]customer['"]\)/);

    // Shell contract: imports CustomerShell from ../_shell/Shell
    expect(mainSrc).toMatch(/import\s*\{[^}]*\bCustomerShell\b[^}]*\}\s*from\s*['"][./]*_shell\/Shell['"]/);
    expect(mainSrc).toMatch(/<CustomerShell\s+route=['"]\/app\/rate-alerts['"]/);

    // Must not import AppShell directly or render LangSwitch
    expect(mainSrc).not.toMatch(/import\s*\{[^}]*\bAppShell\b[^}]*\}/);
    expect(mainSrc).not.toMatch(/<LangSwitch/);
  });
});
