import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { PublicBranchesData } from '../../../shared/api/public/public_branches';
import type { PlannerPreviewAnswer } from '../../../shared/api/generated/planner_preview';
import type { PlannerDraftDto } from '../../../shared/api/generated/my_planner_drafts';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { HomeScreen } from './HomeScreen';
import { th as screenTh } from './i18n';

const BRANCHES_BOTH: PublicBranchesData = {
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
    {
      code: 'XC-B02',
      name: 'สาขาอโศก',
      address: 'อโศก',
      openingHours: { mon: [{ open: '10:00', close: '20:00' }] },
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
      queueOpen: true,
    },
  ],
};

const BRANCHES_B02_ONLY: PublicBranchesData = {
  branches: [BRANCHES_BOTH.branches[1]!],
};

const RATES: PublishedRatesData = {
  publication: { id: 'pub-1', version: 1, publishedAt: '2026-10-12T08:00:00Z' },
  sourceDate: '2026-10-12',
  stale: false,
  lines: [
    {
      currency: 'USD',
      group: 'ALL',
      unitSize: '1',
      buyRate: '35.12000000',
      sellRate: '35.42000000',
      lineId: 'l-usd',
      bands: [],
    },
    {
      currency: 'EUR',
      group: 'ALL',
      unitSize: '1',
      buyRate: '38.10000000',
      sellRate: '38.50000000',
      lineId: 'l-eur',
      bands: [],
    },
  ],
  bandHeadings: [],
};

const PREVIEW_ANSWER: PlannerPreviewAnswer = {
  thb: '3542.00',
  rate: '35.42000000',
  unitSize: '1',
  stale: false,
  sourceDate: '2026-10-12',
};

const DRAFT_ANSWER: PlannerDraftDto = {
  id: 'draft-2367-001',
  branch: 'XC-B01',
  currency: 'USD',
  customerSide: 'CUSTOMER_BUY',
  amount: { quantity: '100' },
  version: 1,
  expiresAt: '2026-10-19T08:00:00Z',
  createdAt: '2026-10-12T08:00:00Z',
  updatedAt: '2026-10-12T08:00:00Z',
};

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

describe('Customer Home: drop stale savePlan answers and clear preview on automatic branch switch (#2367)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: RecordedRequest[];
  let branchesResponder: (req: RecordedRequest) => Response | Promise<Response>;
  let previewResponder: (req: RecordedRequest) => Response | Promise<Response>;
  let draftResponder: (req: RecordedRequest) => Response | Promise<Response>;

  beforeAll(() => warmModules(() => import('./HomeScreen')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    sentRequests = [];

    branchesResponder = () => json(200, BRANCHES_BOTH);
    previewResponder = () => json(200, PREVIEW_ANSWER);
    draftResponder = () => json(200, DRAFT_ANSWER);
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
      return branchesResponder(rec);
    }
    if (u.pathname.includes('/rates')) {
      return json(200, RATES);
    }
    if (u.pathname === '/api/v3/compute/planner_preview' && method === 'POST') {
      return previewResponder(rec);
    }
    if (u.pathname === '/api/app/planner-drafts' && method === 'POST') {
      return draftResponder(rec);
    }
    return json(404, { error: { code: 'notFound', message: u.pathname } });
  };

  async function mountScreen(branchesIntervalMs = 600_000): Promise<void> {
    dispose = render(
      () => (
        <HomeScreen
          fetch={testFetch}
          org="XC"
          ratesIntervalMs={60_000}
          branchesIntervalMs={branchesIntervalMs}
        />
      ),
      host,
    );
    await flush();
  }

  function clickCalculate(): void {
    const calcBtn = Array.from(host.querySelectorAll('button')).find((b) =>
      b.textContent?.includes(screenTh.calculate),
    );
    expect(calcBtn, 'calculate button').toBeDefined();
    calcBtn!.click();
  }

  function estimateCard(): HTMLElement | null {
    return host.querySelector<HTMLElement>('.xc-home-estimate-card');
  }

  function savePlanBtn(): HTMLButtonElement | undefined {
    return Array.from(host.querySelectorAll<HTMLButtonElement>('.xc-home-save-actions button'))[0];
  }

  it('row 1: Save pending -> edit amount -> Calculate -> old save answers OK -> no "plan saved" shown under the new estimate', async () => {
    let resolveDraft!: (res: Response) => void;
    draftResponder = () =>
      new Promise<Response>((r) => {
        resolveDraft = r;
      });

    await mountScreen();
    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();

    savePlanBtn()!.click();
    await flush();

    const amountInput = host.querySelector<HTMLInputElement>('#amount-input')!;
    amountInput.value = '250';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();
    expect(estimateCard()).toBeNull();

    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();

    resolveDraft(json(200, DRAFT_ANSWER));
    await flush();

    expect(estimateCard()).not.toBeNull();
    expect(host.querySelector('.xc-home-alert-success')).toBeNull();
  });

  it('row 2: Save pending -> edit amount -> Calculate -> old save answers 500 -> no save error shown under the new estimate', async () => {
    let resolveDraft!: (res: Response) => void;
    draftResponder = () =>
      new Promise<Response>((r) => {
        resolveDraft = r;
      });

    await mountScreen();
    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();

    savePlanBtn()!.click();
    await flush();

    const amountInput = host.querySelector<HTMLInputElement>('#amount-input')!;
    amountInput.value = '250';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();
    expect(estimateCard()).toBeNull();

    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();

    resolveDraft(json(500, { error: { code: 'internalError', message: 'Internal server error' } }));
    await flush();

    expect(estimateCard()).not.toBeNull();
    expect(host.querySelector('.xc-home-alert-error')).toBeNull();
  });

  it('row 3: Selected branch disappears from the branches poll -> branch switches and the estimate + Save plan are hidden', async () => {
    await mountScreen(20);
    const branchSelect = host.querySelector<HTMLSelectElement>('#branch-select')!;
    expect(branchSelect.value).toBe('XC-B01');

    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();
    expect(savePlanBtn()).toBeDefined();

    branchesResponder = () => json(200, BRANCHES_B02_ONLY);
    await new Promise((r) => setTimeout(r, 40));
    await flush();

    expect(branchSelect.value).toBe('XC-B02');
    expect(estimateCard()).toBeNull();
    expect(savePlanBtn()).toBeUndefined();
  });

  it('row 4: Calculate -> Save OK with no edit -> "plan saved <id>" shown (kept), while a subsequent in-flight Save across an edit is dropped', async () => {
    let resolveDraft!: (res: Response) => void;
    draftResponder = () =>
      new Promise<Response>((r) => {
        resolveDraft = r;
      });

    await mountScreen();
    clickCalculate();
    await flush();

    savePlanBtn()!.click();
    await flush();
    resolveDraft(json(200, DRAFT_ANSWER));
    await flush();
    expect(host.querySelector('.xc-home-alert-success')?.textContent).toContain('draft-2367-001');

    // Start a second save after editing and recalculating, then edit currency before it answers
    const currencySelect = host.querySelector<HTMLSelectElement>('#currency-select')!;
    currencySelect.value = 'EUR';
    currencySelect.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();

    clickCalculate();
    await flush();
    savePlanBtn()!.click();
    await flush();

    currencySelect.value = 'USD';
    currencySelect.dispatchEvent(new Event('change', { bubbles: true }));
    await flush();
    clickCalculate();
    await flush();

    resolveDraft(json(200, DRAFT_ANSWER));
    await flush();
    expect(estimateCard()).not.toBeNull();
    expect(host.querySelector('.xc-home-alert-success')).toBeNull();
  });

  it('row 5: Any input edit clears the estimate, savedPlanId and savePlanError (kept), and an automatic branch switch also clears savedPlanId and savePlanError when recalculated', async () => {
    await mountScreen(20);
    clickCalculate();
    await flush();

    savePlanBtn()!.click();
    await flush();
    expect(host.querySelector('.xc-home-alert-success')?.textContent).toContain('draft-2367-001');

    // Input edit clears estimate and savedPlanId (kept)
    const amountInput = host.querySelector<HTMLInputElement>('#amount-input')!;
    amountInput.value = '250';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();
    expect(estimateCard()).toBeNull();

    clickCalculate();
    await flush();
    expect(host.querySelector('.xc-home-alert-success')).toBeNull();

    // Save again on XC-B01, then let XC-B01 disappear from the branches poll
    savePlanBtn()!.click();
    await flush();
    expect(host.querySelector('.xc-home-alert-success')?.textContent).toContain('draft-2367-001');

    branchesResponder = () => json(200, BRANCHES_B02_ONLY);
    await new Promise((r) => setTimeout(r, 40));
    await flush();
    expect(estimateCard()).toBeNull();

    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();
    expect(host.querySelector('.xc-home-alert-success')).toBeNull();
  });

  it('edge case: The Save button busy state ends when the old save answers, even if its result is dropped', async () => {
    let resolveDraft!: (res: Response) => void;
    draftResponder = () =>
      new Promise<Response>((r) => {
        resolveDraft = r;
      });

    await mountScreen();
    clickCalculate();
    await flush();

    savePlanBtn()!.click();
    await flush();
    expect(savePlanBtn()!.disabled).toBe(true);

    const amountInput = host.querySelector<HTMLInputElement>('#amount-input')!;
    amountInput.value = '250';
    amountInput.dispatchEvent(new Event('input', { bubbles: true }));
    await flush();

    clickCalculate();
    await flush();
    expect(savePlanBtn()!.disabled).toBe(true);

    resolveDraft(json(200, DRAFT_ANSWER));
    await flush();

    expect(savePlanBtn()!.disabled).toBe(false);
    expect(host.querySelector('.xc-home-alert-success')).toBeNull();
  });

  it('edge case: The initial branch selection on first load (no estimate yet) behaves as today, and dropping that branch on a later poll after Calculate hides the estimate', async () => {
    await mountScreen(20);
    const branchSelect = host.querySelector<HTMLSelectElement>('#branch-select')!;
    expect(branchSelect.value).toBe('XC-B01');
    expect(estimateCard()).toBeNull();

    clickCalculate();
    await flush();
    expect(estimateCard()).not.toBeNull();

    branchesResponder = () => json(200, BRANCHES_B02_ONLY);
    await new Promise((r) => setTimeout(r, 40));
    await flush();

    expect(branchSelect.value).toBe('XC-B02');
    expect(estimateCard()).toBeNull();
  });
});
