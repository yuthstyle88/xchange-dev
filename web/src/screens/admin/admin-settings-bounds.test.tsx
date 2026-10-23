// #4984 (P7): Admin reconciliation, booking slots, campaigns and rate policy input bounds (rows 1–7).
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang, t as sharedT } from '../../shared/i18n';
import { sharedSession } from '../../shared/session/session';
import {
  makeAdminBookingPolicyDto,
  makeAdminBookingSlotsData,
  makeAdminClosureDto,
  makeAdminSlotRuleDto,
  makeCampaignData,
  makeCampaignDetail,
  makeCampaignListItem,
  makeCampaignPeriodTotals,
  makeCampaignRuleDetail,
  makeCampaignVersionDetail,
  makeCampaignsData,
  makeFinanceSheet,
  makePart,
  makePublishedLineDto,
  makePublishedRatesData,
  makeRatePolicyData,
  makeRatePolicyVersionDto,
} from '../../shared/testing/fixtures';
import { meOf } from '../../test-support/me';
import { BookingSlots } from './booking-slots/BookingSlots';
import * as bookingTable from './booking-slots/i18n';
import { Campaigns } from './campaigns/Campaigns';
import * as campaignsTable from './campaigns/i18n';
import { emptyValueTiers, RatesPolicyCard } from './rates/RatesPolicyCard';
import * as ratesTable from './rates/i18n';
import { Reconciliation } from './reconciliation/Reconciliation';
import * as reconTable from './reconciliation/i18n';

const B01 = 'XC-B01';

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

function setInput(el: HTMLInputElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('Admin settings bounds (#4984 rows 1–7)', () => {
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
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  it('row 1: Reconciliation certify note bound 0..500 (maxLength, live n / 500 counter, over-max and server refusal via fieldError)', async () => {
    const writes: unknown[] = [];
    let failServer = false;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const u = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (u.pathname === '/api/v3/views/finance_reconciliation' && method === 'GET') {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: makeFinanceSheet({
                branch: B01,
                date: '2026-10-15',
                transactions: makePart<never>(null, { available: false, reason: 'TX' }),
                counts: makePart<never>(null, { available: false, reason: 'CA-3' }),
                transfers: makePart<never>(null, { available: false, reason: 'CA-5' }),
                exceptions: makePart([], { available: false, reason: null }),
                integrity: makePart([], { available: false, reason: null }),
                reversalsPending: makePart([], { available: false, reason: null }),
                figuresHash: 'hash-1',
                certifications: [],
              }),
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/v3/commands/reconciliation_certify' && method === 'POST') {
          writes.push(init?.body);
          if (failServer) {
            return new Response(
              JSON.stringify({
                error: { code: 'validationFailed', field: 'note', message: 'note too long' },
              }),
              { status: 400, headers: { 'content-type': 'application/json' } },
            );
          }
        }
        return new Response(JSON.stringify({ version: 1, changed: false }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }),
    );

    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown] = createSignal(true);
    dispose = render(() => <Reconciliation branch={branch} period={period} shown={shown} />, host);
    await settle();

    const section = host.querySelector('.reconciliation-certify')!;
    const noteInput = section.querySelector<HTMLInputElement>('input[type="text"]')!;
    expect(noteInput.maxLength).toBe(500);
    expect(section.textContent).toContain('0 / 500');

    setInput(noteInput, 'hello');
    await settle();
    expect(section.textContent).toContain('5 / 500');

    const certifyBtn = [...section.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === reconTable.th.certify,
    )!;

    setInput(noteInput, 'x'.repeat(501));
    certifyBtn.click();
    await settle();
    expect(writes).toHaveLength(0);
    expect(host.querySelector('[data-field-error="note"]')?.textContent).toBe(
      sharedT('input.invalid', { field: reconTable.th.note }),
    );

    failServer = true;
    setInput(noteInput, 'valid note');
    certifyBtn.click();
    await settle();
    expect(writes).toHaveLength(1);
    expect(host.querySelector('[data-field-error="note"]')?.textContent).toBe(
      sharedT('input.invalid', { field: reconTable.th.note }),
    );
  });

  it('row 2: BookingSlots closure reason bound 1..200 (maxLength, live n / 200 counter, over-max and server refusal via fieldError)', async () => {
    sharedSession().updateMe(
      meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }),
    );
    const writes: unknown[] = [];
    let failServer = false;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const u = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (u.pathname === '/api/v3/views/admin_booking_slots' && method === 'GET') {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: makeAdminBookingSlotsData({
                rule: makeAdminSlotRuleDto({ branchId: B01, slotMinutes: 30, capacity: 8, horizonDays: 21 }),
                closures: [makeAdminClosureDto({ branchId: B01, closedOn: '2026-12-25', reason: 'Holiday' })],
                slots: [],
                policy: makeAdminBookingPolicyDto({
                  cancelCutoffMinutes: 45,
                  noShowGraceMinutes: 20,
                  confirmExpiryMinutes: 30,
                }),
              }),
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname.endsWith('/closures') && method === 'POST') {
          writes.push(init?.body);
          if (failServer) {
            return new Response(
              JSON.stringify({
                error: { code: 'validationFailed', field: 'reason', message: 'reason invalid' },
              }),
              { status: 400, headers: { 'content-type': 'application/json' } },
            );
          }
        }
        return new Response(JSON.stringify({ saved: true }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }),
    );

    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown] = createSignal(true);
    dispose = render(() => <BookingSlots branch={branch} period={period} shown={shown} />, host);
    await settle();

    const closedOnInput = host.querySelector<HTMLInputElement>('input[name="closedOn"]')!;
    const reasonInput = host.querySelector<HTMLInputElement>('input[name="reason"]')!;
    const reasonGroup = reasonInput.closest('.form-group')!;
    expect(reasonInput.maxLength).toBe(200);
    expect(reasonGroup.textContent).toContain('0 / 200');

    setInput(closedOnInput, '2026-12-31');
    setInput(reasonInput, 'Holiday');
    await settle();
    expect(reasonGroup.textContent).toContain('7 / 200');

    const addBtn = [...host.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === bookingTable.th.addClosure,
    )!;

    setInput(reasonInput, 'r'.repeat(201));
    addBtn.click();
    await settle();
    expect(writes).toHaveLength(0);
    expect(host.querySelector('[data-field-error="reason"]')?.textContent).toBe(
      sharedT('input.invalid', { field: bookingTable.th.reason }),
    );

    failServer = true;
    setInput(reasonInput, 'New Year Eve');
    addBtn.click();
    await settle();
    expect(writes).toHaveLength(1);
    expect(host.querySelector('[data-field-error="reason"]')?.textContent).toBe(
      sharedT('input.invalid', { field: bookingTable.th.reason }),
    );
  });

  it('row 3: BookingSlots cancel cutoff and no-show grace bound 0..129,600 (max="129600", over-max and server refusal at field)', async () => {
    sharedSession().updateMe(
      meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }),
    );
    const writes: unknown[] = [];
    let failField: string | null = null;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const u = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (u.pathname === '/api/v3/views/admin_booking_slots' && method === 'GET') {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: makeAdminBookingSlotsData({
                rule: makeAdminSlotRuleDto({ branchId: B01, slotMinutes: 30, capacity: 8, horizonDays: 21 }),
                closures: [],
                slots: [],
                policy: makeAdminBookingPolicyDto({
                  cancelCutoffMinutes: 45,
                  noShowGraceMinutes: 20,
                  confirmExpiryMinutes: 30,
                }),
              }),
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/booking-policy' && method === 'PUT') {
          writes.push(init?.body);
          if (failField) {
            return new Response(
              JSON.stringify({
                error: { code: 'validationFailed', field: failField, message: 'invalid' },
              }),
              { status: 400, headers: { 'content-type': 'application/json' } },
            );
          }
        }
        return new Response(JSON.stringify({ saved: true }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }),
    );

    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown] = createSignal(true);
    dispose = render(() => <BookingSlots branch={branch} period={period} shown={shown} />, host);
    await settle();

    const cutoffInput = host.querySelector<HTMLInputElement>('input[name="cancelCutoffMinutes"]')!;
    const graceInput = host.querySelector<HTMLInputElement>('input[name="noShowGraceMinutes"]')!;
    expect(cutoffInput.getAttribute('min')).toBe('0');
    expect(cutoffInput.getAttribute('max')).toBe('129600');
    expect(graceInput.getAttribute('min')).toBe('0');
    expect(graceInput.getAttribute('max')).toBe('129600');

    const savePolicyBtn = [...host.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === bookingTable.th.savePolicy,
    )!;

    setInput(cutoffInput, '129601');
    savePolicyBtn.click();
    await settle();
    expect(writes).toHaveLength(0);
    expect(host.querySelector('[data-field-error="cancelCutoffMinutes"]')?.textContent).toBe(
      sharedT('input.invalid', { field: bookingTable.th.cancelCutoffMinutes }),
    );

    setInput(cutoffInput, '0');
    setInput(graceInput, '129601');
    savePolicyBtn.click();
    await settle();
    expect(writes).toHaveLength(0);
    expect(host.querySelector('[data-field-error="noShowGraceMinutes"]')?.textContent).toBe(
      sharedT('input.invalid', { field: bookingTable.th.noShowGraceMinutes }),
    );

    failField = 'cancelCutoffMinutes';
    setInput(graceInput, '129600');
    savePolicyBtn.click();
    await settle();
    expect(writes).toHaveLength(1);
    expect(host.querySelector('[data-field-error="cancelCutoffMinutes"]')?.textContent).toBe(
      sharedT('input.invalid', { field: bookingTable.th.cancelCutoffMinutes }),
    );
  });

  it('row 4: BookingSlots confirm expiry bound 1..129,600 (min="1", max="129600", over-max and server refusal at field)', async () => {
    sharedSession().updateMe(
      meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }),
    );
    const writes: unknown[] = [];
    let failServer = false;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const u = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (u.pathname === '/api/v3/views/admin_booking_slots' && method === 'GET') {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: makeAdminBookingSlotsData({
                rule: makeAdminSlotRuleDto({ branchId: B01, slotMinutes: 30, capacity: 8, horizonDays: 21 }),
                closures: [],
                slots: [],
                policy: makeAdminBookingPolicyDto({
                  cancelCutoffMinutes: 45,
                  noShowGraceMinutes: 20,
                  confirmExpiryMinutes: 30,
                }),
              }),
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/booking-policy' && method === 'PUT') {
          writes.push(init?.body);
          if (failServer) {
            return new Response(
              JSON.stringify({
                error: { code: 'validationFailed', field: 'confirmExpiryMinutes', message: 'invalid' },
              }),
              { status: 400, headers: { 'content-type': 'application/json' } },
            );
          }
        }
        return new Response(JSON.stringify({ saved: true }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }),
    );

    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown] = createSignal(true);
    dispose = render(() => <BookingSlots branch={branch} period={period} shown={shown} />, host);
    await settle();

    const expiryInput = host.querySelector<HTMLInputElement>('input[name="confirmExpiryMinutes"]')!;
    expect(expiryInput.getAttribute('min')).toBe('1');
    expect(expiryInput.getAttribute('max')).toBe('129600');

    const savePolicyBtn = [...host.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === bookingTable.th.savePolicy,
    )!;

    setInput(expiryInput, '129601');
    savePolicyBtn.click();
    await settle();
    expect(writes).toHaveLength(0);
    expect(host.querySelector('[data-field-error="confirmExpiryMinutes"]')?.textContent).toBe(
      sharedT('input.invalid', { field: bookingTable.th.confirmExpiryMinutes }),
    );

    failServer = true;
    setInput(expiryInput, '129600');
    savePolicyBtn.click();
    await settle();
    expect(writes).toHaveLength(1);
    expect(host.querySelector('[data-field-error="confirmExpiryMinutes"]')?.textContent).toBe(
      sharedT('input.invalid', { field: bookingTable.th.confirmExpiryMinutes }),
    );
  });

  it('row 5: Campaigns coupon count per issue bound 1..1,000 (max="1000", over-max and server refusal at field)', async () => {
    const writes: unknown[] = [];
    let failServer = false;

    const listItem = makeCampaignListItem({
      id: 'cmp-1',
      name: 'Golden October',
      versionId: 'ver-1',
      version: 1,
      status: 'ACTIVE',
      startsAt: '2026-10-01T00:00:00Z',
      endsAt: '2026-10-31T23:59:59Z',
      budgetThb: '100000.00',
      budgetUsedThb: '0.00',
      budgetLeftThb: '100000.00',
      uses: 0,
      maxUses: 100,
      period: makeCampaignPeriodTotals({ redemptions: 0, priceCostThb: '0.00', rewardExpenseThb: '0.00' }),
    });
    const detail = makeCampaignDetail({
      id: 'cmp-1',
      name: 'Golden October',
      currentVersionId: 'ver-1',
      versions: [
        makeCampaignVersionDetail({
          id: 'ver-1',
          version: 1,
          status: 'ACTIVE',
          startsAt: '2026-10-01T00:00:00Z',
          endsAt: '2026-10-31T23:59:59Z',
          budgetThb: '100000.00',
          maxUses: 100,
          stackGroup: 'DEFAULT',
          priority: 10,
          branches: ['MAIN'],
          rules: [
            makeCampaignRuleDetail({
              id: '00000000-0000-0000-0000-000000000001',
              ruleNo: 1,
              benefitType: 'RATE_IMPROVEMENT',
              benefitClass: 'PRICE',
              requiresCoupon: true,
              eligibility: {},
              calculation: { spreadShare: '0.20' },
              caps: {},
            }),
          ],
          simulations: [],
          approval: null,
        }),
      ],
    });

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const u = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (u.pathname === '/api/v3/views/campaigns') {
          return new Response(
            JSON.stringify({ version: 1, changed: true, data: makeCampaignsData({ campaigns: [listItem] }) }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/v3/views/campaign') {
          return new Response(
            JSON.stringify({ version: 1, changed: true, data: makeCampaignData({ campaign: detail }) }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/campaigns/cmp-1/coupons' && method === 'POST') {
          writes.push(init?.body);
          if (failServer) {
            return new Response(
              JSON.stringify({
                error: { code: 'validationFailed', field: 'count', message: 'count invalid' },
              }),
              { status: 400, headers: { 'content-type': 'application/json' } },
            );
          }
        }
        return new Response(JSON.stringify({ coupons: [] }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }),
    );

    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-14' });
    const [shown] = createSignal(true);
    dispose = render(() => <Campaigns branch={branch} period={period} shown={shown} />, host);
    await settle();

    host.querySelector<HTMLElement>('[data-campaign-id="cmp-1"]')!.click();
    await settle();

    const countInput = host.querySelector<HTMLInputElement>('input[name="count"]')!;
    expect(countInput.getAttribute('min')).toBe('1');
    expect(countInput.getAttribute('max')).toBe('1000');

    const issueBtn = [...host.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === campaignsTable.th.issueCoupons,
    )!;

    setInput(countInput, '1001');
    issueBtn.click();
    await settle();
    expect(writes).toHaveLength(0);
    expect(host.querySelector('[data-field-error="count"]')?.textContent).toBe(
      sharedT('input.invalid', { field: campaignsTable.th.couponCount }),
    );

    failServer = true;
    setInput(countInput, '1000');
    issueBtn.click();
    await settle();
    expect(writes).toHaveLength(1);
    expect(host.querySelector('[data-field-error="count"]')?.textContent).toBe(
      sharedT('input.invalid', { field: campaignsTable.th.couponCount }),
    );
  });

  it('row 6: Campaigns priority bound 0..2,147,483,647 (min="0", max="2147483647", over-max and server refusal at field)', async () => {
    const writes: unknown[] = [];
    let failServer = false;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const u = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (u.pathname === '/api/v3/views/campaigns') {
          return new Response(
            JSON.stringify({ version: 1, changed: true, data: makeCampaignsData({ campaigns: [] }) }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/campaigns' && method === 'POST') {
          writes.push(init?.body);
          if (failServer) {
            return new Response(
              JSON.stringify({
                error: { code: 'validationFailed', field: 'priority', message: 'priority invalid' },
              }),
              { status: 400, headers: { 'content-type': 'application/json' } },
            );
          }
        }
        return new Response(JSON.stringify({ id: 'cmp-new', name: 'New', status: 'DRAFT' }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }),
    );

    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-14' });
    const [shown] = createSignal(true);
    dispose = render(() => <Campaigns branch={branch} period={period} shown={shown} />, host);
    await settle();

    const openCreateBtn = [...host.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === campaignsTable.th.create,
    )!;
    openCreateBtn.click();
    await settle();

    setInput(host.querySelector<HTMLInputElement>('input[name="name"]')!, 'New Campaign');
    setInput(host.querySelector<HTMLInputElement>('input[name="startsAt"]')!, '2026-11-01');
    setInput(host.querySelector<HTMLInputElement>('input[name="endsAt"]')!, '2026-11-30');
    setInput(host.querySelector<HTMLInputElement>('input[name="budgetThb"]')!, '50000.00');

    const priorityInput = host.querySelector<HTMLInputElement>('input[name="priority"]')!;
    expect(priorityInput.getAttribute('min')).toBe('0');
    expect(priorityInput.getAttribute('max')).toBe('2147483647');

    const submitCreateBtn = [...host.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === campaignsTable.th.create,
    )!;

    setInput(priorityInput, '2147483648');
    submitCreateBtn.click();
    await settle();
    expect(writes).toHaveLength(0);
    expect(host.querySelector('[data-field-error="priority"]')?.textContent).toBe(
      sharedT('input.invalid', { field: campaignsTable.th.fieldPriority }),
    );

    failServer = true;
    setInput(priorityInput, '2147483647');
    submitCreateBtn.click();
    await settle();
    expect(writes).toHaveLength(1);
    expect(host.querySelector('[data-field-error="priority"]')?.textContent).toBe(
      sharedT('input.invalid', { field: campaignsTable.th.fieldPriority }),
    );
  });

  it('row 7: RatesPolicyCard max reference age bound 1..168 (min="1", max="168", over-max and server refusal at field)', async () => {
    const writes: unknown[] = [];
    let failServer = false;

    const [branch] = createSignal(B01);
    const [busy] = createSignal(false);
    const [canSavePolicy] = createSignal(true);
    const [policyData] = createSignal(
      makeRatePolicyData({
        policy: makeRatePolicyVersionDto({
          id: 'pol-1',
          branchId: B01,
          version: 1,
          buyAdjustPct: '0.0000',
          sellAdjustPct: '0.0000',
          guards: {},
          maxReferenceAgeHours: 24,
          onStale: 'STOP',
          staleLimitThb: null,
          reason: 'Initial',
          valueTiersOn: false,
          valueTiers: [],
          sharpMoveAlertPct: null,
        }),
      }),
    );
    const [policyError] = createSignal<string | null>(null);
    const [pubData] = createSignal(
      makePublishedRatesData({
        publication: null,
        lines: [
          makePublishedLineDto({
            currency: 'USD',
            group: 'ALL',
            unitSize: '100',
            buyRate: '33.00',
            sellRate: '33.50',
            lineId: 'line-USD',
            bands: [],
          }),
        ],
      }),
    );
    const [valueTiers, setValueTiers] = createSignal(emptyValueTiers());
    const [valueTiersOn, setValueTiersOn] = createSignal(false);
    const [sharpMoveAlertPct, setSharpMoveAlertPct] = createSignal('');
    const [adjustBuyPct] = createSignal('0.0000');
    const [adjustSellPct] = createSignal('0.0000');
    const [readGen] = createSignal(1);

    const savePolicyWrite = {
      async send(body: unknown) {
        writes.push(body);
        if (failServer) {
          const { ApiError } = await import('../../shared/api/client');
          throw new ApiError({
            status: 400,
            code: 'validationFailed',
            field: 'maxReferenceAgeHours',
            message: 'maxReferenceAgeHours invalid',
          });
        }
        return { id: 'pol-2', version: 2 };
      },
      reset() {},
    };

    dispose = render(
      () => (
        <RatesPolicyCard
          branch={branch}
          busy={busy}
          canSavePolicy={canSavePolicy}
          policyData={policyData}
          policyError={policyError}
          pubData={pubData}
          valueTiers={valueTiers}
          setValueTiers={setValueTiers}
          valueTiersOn={valueTiersOn}
          setValueTiersOn={setValueTiersOn}
          sharpMoveAlertPct={sharpMoveAlertPct}
          setSharpMoveAlertPct={setSharpMoveAlertPct}
          adjustBuyPct={adjustBuyPct}
          adjustSellPct={adjustSellPct}
          savePolicyWrite={savePolicyWrite}
          readGen={readGen}
          setSimulateResult={() => {}}
          guarded={(fn) => fn}
          toast={() => {}}
          t={(k) => ratesTable.th[k]}
        />
      ),
      host,
    );
    await settle();

    const ageInput = host.querySelector<HTMLInputElement>('input[name="maxReferenceAgeHours"]')!;
    expect(ageInput.getAttribute('min')).toBe('1');
    expect(ageInput.getAttribute('max')).toBe('168');

    const saveBtn = host.querySelector<HTMLButtonElement>('button[data-testid="save-policy"]')!;

    setInput(ageInput, '169');
    saveBtn.click();
    await settle();
    expect(writes).toHaveLength(0);
    expect(host.querySelector('[data-field-error="maxReferenceAgeHours"]')?.textContent).toBe(
      ratesTable.th.maxReferenceAgeInvalid,
    );

    failServer = true;
    setInput(ageInput, '168');
    saveBtn.click();
    await settle();
    expect(writes).toHaveLength(1);
    expect(host.querySelector('[data-field-error="maxReferenceAgeHours"]')?.textContent).toBe(
      sharedT('input.invalid', { field: ratesTable.th.maxReferenceAgeHoursLabel }),
    );
  });
});
