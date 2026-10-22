import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import {
  makePlannerPreviewAnswer,
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { HomeScreen } from './HomeScreen';
import { th as screenTh } from './i18n';

const BRANCHES: PublicBranchesData = {
  branches: [
    {
      code: 'XC-B01',
      name: 'สาขาสยามพารากอน',
      address: 'ชั้น G สยามพารากอน',
      openingHours: { mon: [{ open: '10:00', close: '20:00' }] },
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
  ],
};

const RATES = makePublishedRatesData({
  publication: makePublishedPublicationDto({ id: 'pub-1', version: 1, publishedAt: '2026-10-12T08:00:00Z' }),
  sourceDate: '2026-10-12',
  stale: false,
  lines: [
    makePublishedLineDto({
      currency: 'USD',
      group: 'ALL',
      unitSize: '1',
      buyRate: '35.12000000',
      sellRate: '35.42000000',
      lineId: 'l-usd',
      bands: [],
    }),
  ],
  bandHeadings: [],
});

const PREVIEW_ANSWER = makePlannerPreviewAnswer({
  thb: '3542.00',
  rate: '35.42000000',
  unitSize: '1',
  stale: false,
  sourceDate: '2026-10-12',
});

interface RecordedRequest {
  method: string;
  url: string;
  path: string;
  headers: Headers;
  rawBody: string | undefined;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('Customer home: calculate branch requirement (#4819)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: RecordedRequest[];
  let branchResponder: () => Promise<Response> | Response;

  beforeAll(() => warmModules(() => import('./HomeScreen')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    sentRequests = [];
    branchResponder = () => json(200, BRANCHES);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.restoreAllMocks();
    setAudience('customer');
    setLang('th');
  });

  const testFetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const u = new URL(input, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    const headers = new Headers(init?.headers);
    const rawBody = typeof init?.body === 'string' ? init.body : undefined;

    const rec: RecordedRequest = {
      method,
      url: input,
      path: u.pathname,
      headers,
      rawBody,
    };
    sentRequests.push(rec);

    if (u.pathname === '/api/public/host') {
      return json(200, { org: 'XC' });
    }
    if (u.pathname.includes('/branches') && !u.pathname.includes('/rates')) {
      return branchResponder();
    }
    if (u.pathname.includes('/rates')) {
      return json(200, RATES);
    }
    if (u.pathname === '/api/v3/compute/planner_preview' && method === 'POST') {
      return json(200, PREVIEW_ANSWER);
    }
    return json(404, { error: { code: 'notFound', message: u.pathname } });
  };

  async function mountScreen(): Promise<void> {
    dispose = render(
      () => (
        <HomeScreen
          fetch={testFetch}
          org="XC"
          ratesIntervalMs={600_000}
          branchesIntervalMs={600_000}
        />
      ),
      host,
    );
    await flush();
  }

  function calcButton(): HTMLButtonElement {
    const btn = host.querySelector<HTMLButtonElement>('.xc-home-calc-actions button');
    expect(btn, 'calculate button').not.toBeNull();
    return btn!;
  }

  it('C1: the branch read has not answered -> Calculate is disabled; clicking it does nothing', async () => {
    // Keep branch read pending indefinitely
    branchResponder = () => new Promise<Response>(() => {});
    await mountScreen();

    const btn = calcButton();
    expect(btn.disabled).toBe(true);

    btn.click();
    await flush();

    const previewCalls = sentRequests.filter((r) => r.path === '/api/v3/compute/planner_preview');
    expect(previewCalls.length).toBe(0);
    expect(host.querySelector('.xc-home-alert-error')).toBeNull();
  });

  it('C2: the branch read answers (auto-select) -> Calculate is enabled; amount 100 calculates', async () => {
    await mountScreen();
    const btn = calcButton();
    expect(btn.disabled).toBe(false);

    btn.click();
    await flush();

    const previewCalls = sentRequests.filter((r) => r.path === '/api/v3/compute/planner_preview');
    expect(previewCalls.length).toBe(1);
    expect(previewCalls[0]?.rawBody).toBeDefined();
    const body = JSON.parse(previewCalls[0]!.rawBody!);
    expect(body.branch).toBe('XC-B01');
    expect(body.amount).toEqual({ quantity: '100' });
    expect(host.querySelector('.xc-home-estimate-card')).not.toBeNull();
  });

  it('C3: the branch read answers, then amount 0 + Calculate -> the validationError text shows and stays; no request', async () => {
    await mountScreen();
    const btn = calcButton();
    expect(btn.disabled).toBe(false);

    const amountInput = host.querySelector<HTMLInputElement>('#amount-input')!;
    amountInput.value = '0';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    btn.click();
    await flush();

    const err = host.querySelector<HTMLElement>('.xc-home-alert-error');
    expect(err).not.toBeNull();
    expect(err!.textContent).toContain(screenTh.validationError);

    // A further settle: nothing (such as a branch auto-select) wipes the error.
    await flush();
    expect(host.querySelector('.xc-home-alert-error')?.textContent).toContain(screenTh.validationError);
    const previewCalls = sentRequests.filter((r) => r.path === '/api/v3/compute/planner_preview');
    expect(previewCalls.length).toBe(0);
  });
});
