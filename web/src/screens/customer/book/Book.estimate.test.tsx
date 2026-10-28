// XC-BOOKING-ESTIMATE FE-1: C02 /app/book shows planner_preview's indicative benefit, warns when the chosen slot is at
// or after validUntil, and refreshes the estimate only on a new rates ETag or once at validUntil, never on a timer.
// Design docs/design/blueprint/marketing-campaigns.md E-6, E-7, E-9, §6.4.5 rows 22 and 23, §6.4.6 FE-1.
// Written before the implementation by the FE lane controller claude/e92db6. Do not edit, delete or weaken these
// tests; add your own elsewhere.
//
// [required] by these tests:
// - The answer's `benefit` is rendered through the shared `IndicativeBenefit` (`screens/customer/benefit-estimate/`)
//   inside the preview, with `slotStartAt` = the chosen slot's `start`.
// - With a branch chosen, the screen polls `/api/public/orgs/<org>/branches/<code>/rates` with `If-None-Match` every
//   `ratesIntervalMs` (a `BookScreen` prop, default 10 000 ms). A 200 with a new ETag sends one more planner_preview;
//   a 304 sends none.
// - When the kept answer is ESTIMATED, the screen sends one more planner_preview at its `validUntil`, once.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeBookingCreateOutput,
  makePlannerBenefitItemDto,
  makePlannerPreviewAnswer,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import { BookScreen } from './Book';

const BRANCH_ID = 'XC-B01';
const SLOT_ID = '00000000-0000-4000-8000-000000000001';
const SLOT_START = '2026-11-05T03:00:00+00:00';

const BRANCHES = {
  branches: [{ id: BRANCH_ID, code: BRANCH_ID, name: 'สาขาสยามพารากอน', timezone: 'Asia/Bangkok', bookingEnabled: true }],
};
const SLOTS = {
  version: 1,
  changed: true,
  data: { slots: [{ id: SLOT_ID, start: SLOT_START, end: '2026-11-05T03:30:00+00:00', available: 3 }] },
};

const estimated = (validUntil: string) =>
  makePlannerPreviewAnswer({
    thb: '3542.00',
    rate: '35.42000000',
    unitSize: '1',
    benefit: {
      state: 'ESTIMATED',
      basis: 'INDICATIVE',
      channel: 'APP',
      evaluatedAt: '2026-11-01T00:00:00Z',
      validUntil,
      publicationId: 'pub-1',
      finalRate: '35.52000000',
      thbAfter: '3552.00',
      benefitThb: '10.00',
      items: [makePlannerBenefitItemDto({ label: 'Gold week', benefitClass: 'PRICE', rateBefore: '35.42', rateAfter: '35.52' })],
    },
  });

const NONE = makePlannerPreviewAnswer({
  thb: '3542.00',
  rate: '35.42000000',
  unitSize: '1',
  benefit: { state: 'NONE', basis: 'INDICATIVE', channel: 'APP', evaluatedAt: '2026-11-01T00:00:00Z', reason: 'noActiveRule' },
});

function json(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  return new Response(status === 304 ? null : JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', ...headers },
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 15; i++) await new Promise((r) => setTimeout(r, 0));
}

describe('Customer Book: the indicative benefit of the plan (FE-1, rows 22 and 23, E-9)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let previews: number;
  let answer: () => unknown;
  let ratesEtag: string;
  let ratesCalls: Array<string | null>;
  let fetchStub: (input: string, init?: RequestInit) => Promise<Response>;

  beforeAll(() => warmModules(() => import('./Book')), 60_000);

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    window.history.replaceState({}, '', '/app/book');
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    previews = 0;
    answer = () => estimated('2026-11-30T00:00:00Z');
    ratesEtag = '"e1"';
    ratesCalls = [];
    fetchStub = vi.fn(async (input: string, init?: RequestInit): Promise<Response> => {
      const u = new URL(input, 'http://xc.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      if (u.pathname === '/api/public/host') return json(200, { org: 'XC', env: 'test' });
      if (u.pathname === `/api/public/orgs/XC/branches/${BRANCH_ID}/rates`) {
        const inm = new Headers(init?.headers).get('if-none-match');
        ratesCalls.push(inm);
        if (inm === ratesEtag) return json(304, null, { etag: ratesEtag });
        return json(200, { asOf: ratesEtag, rates: [] }, { etag: ratesEtag });
      }
      if (u.pathname.startsWith('/api/public/orgs/')) return json(200, BRANCHES);
      if (u.pathname === '/api/v3/views/booking_slots') return json(200, SLOTS);
      if (u.pathname === '/api/v3/compute/planner_preview' && method === 'POST') {
        previews++;
        return json(200, answer());
      }
      if (u.pathname === '/api/bookings' && method === 'POST') return json(200, makeBookingCreateOutput({ branchId: BRANCH_ID }));
      return json(404, { error: { code: 'notFound', message: u.pathname } });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    window.history.replaceState({}, '', '/app/book');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAudience('customer');
    setLang('th');
  });

  async function mountPlan(): Promise<void> {
    dispose = render(() => <BookScreen fetch={fetchStub} ratesIntervalMs={20} />, host);
    await flush();
    await vi.waitFor(() => expect(host.querySelector(`option[value="${BRANCH_ID}"]`)).not.toBeNull());
    const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = BRANCH_ID;
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(host.querySelector(`option[value="${SLOT_ID}"]`)).not.toBeNull());
    const slotSel = host.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = SLOT_ID;
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));
    const qty = host.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    qty.value = '100';
    qty.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() => expect(host.querySelector('[data-testid="planner-preview"]')).not.toBeNull());
  }

  it('F1 (row 22) an ESTIMATED answer shows the benefit block with the indicative note inside the preview', async () => {
    await mountPlan();
    const preview = host.querySelector('[data-testid="planner-preview"]')!;
    await vi.waitFor(() => expect(preview.querySelector('[data-row="benefit-estimate"]')).not.toBeNull());
    expect(preview.querySelector('[data-row="benefit-item"]')?.textContent).toContain('Gold week: 35.42 → 35.52');
    expect(preview.querySelector('[data-row="benefit-indicative-note"]')?.textContent).toBe('เรทสุดท้ายยืนยันเมื่อทำรายการ');
  });

  it('F2 (row 22) a NONE answer shows the preview without a benefit block or a zero amount', async () => {
    answer = () => NONE;
    await mountPlan();
    await flush();
    expect(host.querySelector('[data-row="benefit-estimate"]')).toBeNull();
    expect(host.querySelector('[data-testid="planner-preview"]')).not.toBeNull();
  });

  it('F3 (row 23) the chosen slot starts at validUntil: the promotion-ends warning shows', async () => {
    answer = () => estimated(SLOT_START);
    await mountPlan();
    await vi.waitFor(() => expect(host.querySelector('[data-row="benefit-ends-before-slot"]')).not.toBeNull());
  });

  it('F4 (E-9) 304s from the rates route send no planner_preview; a new ETag sends exactly one', async () => {
    await mountPlan();
    await vi.waitFor(() => expect(ratesCalls.length).toBeGreaterThanOrEqual(3), { timeout: 2000 });
    expect(ratesCalls.slice(1).every((c) => c === '"e1"'), 'polled with If-None-Match').toBe(true);
    const before = previews;
    await new Promise((r) => setTimeout(r, 150));
    expect(previews, 'no planner_preview on a timer').toBe(before);
    ratesEtag = '"e2"';
    await vi.waitFor(() => expect(previews).toBe(before + 1), { timeout: 2000 });
    await new Promise((r) => setTimeout(r, 150));
    expect(previews, 'exactly one more after the new ETag').toBe(before + 1);
  });

  it('F5 (E-9) one more planner_preview at validUntil, once', async () => {
    // Every answer before T says the estimate holds until T; after T it holds for weeks.
    const T = Date.now() + 1000;
    answer = () => estimated(Date.now() < T ? new Date(T).toISOString() : '2026-11-30T00:00:00Z');
    await mountPlan();
    const before = previews;
    expect(Date.now(), 'the plan is ready before T').toBeLessThan(T);
    await vi.waitFor(() => expect(previews).toBe(before + 1), { timeout: 3000 });
    await new Promise((r) => setTimeout(r, 300));
    expect(previews, 'only once at validUntil').toBe(before + 1);
  });
});
