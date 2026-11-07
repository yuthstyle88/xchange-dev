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
      id: 'XC-B01',
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
      buyRate: '32.00000000',
      sellRate: '32.15000000',
      lineId: 'l-usd',
      bands: [],
    }),
  ],
  bandHeadings: [],
});

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

describe('Customer home: quote applied rate formatting (UX-2 #6461)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let previewRate = '32.15000000';
  let previewThb = '3215.00';
  let previewUnitSize = '1';

  beforeAll(() => warmModules(() => import('./HomeScreen')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    previewRate = '32.15000000';
    previewThb = '3215.00';
    previewUnitSize = '1';
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

    if (u.pathname === '/api/public/host') {
      return json(200, { org: 'XC' });
    }
    if (u.pathname.includes('/branches') && !u.pathname.includes('/rates')) {
      return json(200, BRANCHES);
    }
    if (u.pathname.includes('/rates')) {
      return json(200, RATES);
    }
    if (u.pathname === '/api/v3/compute/planner_preview' && method === 'POST') {
      return json(
        200,
        makePlannerPreviewAnswer({
          thb: previewThb,
          rate: previewRate,
          unitSize: previewUnitSize,
          stale: false,
          sourceDate: '2026-10-12',
        }),
      );
    }
    return json(404, { error: { code: 'notFound', message: u.pathname } });
  };

  async function mountAndCalculate(): Promise<HTMLElement> {
    dispose = render(
      () => (
        <HomeScreen
          fetch={testFetch}
          org="XC"
          ratesIntervalMs={60_000}
          branchesIntervalMs={600_000}
        />
      ),
      host,
    );
    await flush();
    const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
      b.textContent?.includes(screenTh.calculate),
    )!;
    calcBtn.click();
    await flush();
    const card = host.querySelector<HTMLElement>('.xc-home-estimate-card');
    expect(card).not.toBeNull();
    return card!;
  }

  it('row 1: quote with rate 32.15000000 shows 32.15', async () => {
    previewRate = '32.15000000';
    const card = await mountAndCalculate();
    const meta = card.querySelector('.xc-home-estimate-meta')!;
    expect(meta.textContent).toContain('32.15');
    expect(meta.textContent).not.toContain('32.15000000');
  });

  it('row 2: rate 36.00000000 shows 36.00', async () => {
    previewRate = '36.00000000';
    const card = await mountAndCalculate();
    const meta = card.querySelector('.xc-home-estimate-meta')!;
    expect(meta.textContent).toContain('36.00');
    expect(meta.textContent).not.toContain('36.00000000');
  });

  it('row 3: rate 0.27350000 shows 0.2735', async () => {
    previewRate = '0.27350000';
    const card = await mountAndCalculate();
    const meta = card.querySelector('.xc-home-estimate-meta')!;
    expect(meta.textContent).toContain('0.2735');
    expect(meta.textContent).not.toContain('0.27350000');
  });

  it('row 4: the THB estimate and unit line remain unchanged alongside formatted rate', async () => {
    previewRate = '32.15000000';
    previewThb = '3215.00';
    previewUnitSize = '1';
    const card = await mountAndCalculate();
    const value = card.querySelector('.xc-home-estimate-value')!;
    const meta = card.querySelector('.xc-home-estimate-meta')!;
    expect(value.textContent).toContain('3,215.00');
    expect(meta.textContent).toBe(`${screenTh.appliedRate}: 32.15 · ${screenTh.unit}: 1 USD`);
  });
});
