import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { meOf } from '../../../test-support/me';
import type { AdminMenuProps } from '../menus';
import { BookingSlots } from './BookingSlots';
import * as table from './i18n';
import {
  makeAdminBookingPolicyDto,
  makeAdminBookingSlotsData,
  makeAdminClosureDto,
  makeAdminSlotDto,
  makeAdminSlotRuleDto,
} from '../../../shared/testing/fixtures';

interface SentRequest {
  method: string;
  path: string;
  search: string;
  body: unknown;
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

const B01 = 'XC-B01';
const B02 = 'XC-B02';

const SAMPLE_POLICY = makeAdminBookingPolicyDto({
  cancelCutoffMinutes: 45,
  noShowGraceMinutes: 20,
  confirmExpiryMinutes: 30,
});

const BRANCH_A_RULE = makeAdminSlotRuleDto({
  branchId: B01,
  slotMinutes: 30,
  capacity: 8,
  horizonDays: 21,
});

const BRANCH_A_CLOSURE = makeAdminClosureDto({
  branchId: B01,
  closedOn: '2026-12-25',
  reason: 'Christmas Day',
});

const BRANCH_A_SLOT = makeAdminSlotDto({
  id: 'slot-a-1',
  branchId: B01,
  serviceDate: '2026-10-15',
  startAt: '2026-10-15T09:00:00Z',
  endAt: '2026-10-15T09:30:00Z',
  capacity: 8,
  reserved: 2,
  status: 'OPEN',
});

const BRANCH_A_DATA = makeAdminBookingSlotsData({
  rule: BRANCH_A_RULE,
  closures: [BRANCH_A_CLOSURE],
  slots: [BRANCH_A_SLOT],
  policy: SAMPLE_POLICY,
});

const BRANCH_B_RULE = makeAdminSlotRuleDto({
  branchId: B02,
  slotMinutes: 60,
  capacity: 12,
  horizonDays: 14,
});

const BRANCH_B_CLOSURE = makeAdminClosureDto({
  branchId: B02,
  closedOn: '2026-04-13',
  reason: 'Songkran Festival',
});

const BRANCH_B_SLOT = makeAdminSlotDto({
  id: 'slot-b-1',
  branchId: B02,
  serviceDate: '2026-10-16',
  startAt: '2026-10-16T10:00:00Z',
  endAt: '2026-10-16T11:00:00Z',
  capacity: 12,
  reserved: 1,
  status: 'OPEN',
});

const BRANCH_B_DATA = makeAdminBookingSlotsData({
  rule: BRANCH_B_RULE,
  closures: [BRANCH_B_CLOSURE],
  slots: [BRANCH_B_SLOT],
  policy: SAMPLE_POLICY,
});

const NO_RULE_DATA = makeAdminBookingSlotsData({
  rule: null,
  closures: [],
  slots: [],
  policy: SAMPLE_POLICY,
});

describe('BookingSlots branch switching & stale-read guard (#2183)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];
  let viewHandler: (branch: string) => Promise<Response>;

  beforeEach(() => {
    setLang('th');
    sharedSession().updateMe(meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }));
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];

    viewHandler = async (branch: string) =>
      new Response(
        JSON.stringify({
          version: 1,
          changed: true,
          data: branch === B02 ? BRANCH_B_DATA : BRANCH_A_DATA,
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      let body: unknown = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }

      sentRequests.push({ method, path: u.pathname, search: u.search, body });

      if (u.pathname === '/api/v3/views/admin_booking_slots') {
        const branch = u.searchParams.get('branch') ?? '';
        return viewHandler(branch);
      }

      return new Response(JSON.stringify({}), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(() => {
    setLang('th');
    dispose?.();
    dispose = null;
    container.remove();
    vi.restoreAllMocks();
  });

  function mountWithBranch(initialBranch = B01) {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown] = createSignal(true);
    const props: AdminMenuProps = { branch, period, shown };
    dispose = render(() => <BookingSlots {...props} />, container);
    return { branch, setBranch };
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  function ruleInputs() {
    const slotMinutesInput = container.querySelector<HTMLInputElement>(
      'input[name="slotMinutes"], input[data-field="slotMinutes"]',
    );
    const capacityInput = container.querySelector<HTMLInputElement>(
      'input[name="capacity"], input[data-field="capacity"]',
    );
    const horizonDaysInput = container.querySelector<HTMLInputElement>(
      'input[name="horizonDays"], input[data-field="horizonDays"]',
    );
    const saveRuleBtn = [...container.querySelectorAll('button')].find((b) =>
      /save|บันทึก/i.test(b.textContent ?? ''),
    );
    expect(slotMinutesInput).not.toBeNull();
    expect(capacityInput).not.toBeNull();
    expect(horizonDaysInput).not.toBeNull();
    expect(saveRuleBtn).toBeDefined();
    return {
      slotMinutesInput: slotMinutesInput!,
      capacityInput: capacityInput!,
      horizonDaysInput: horizonDaysInput!,
      saveRuleBtn: saveRuleBtn!,
    };
  }

  it("row 1: Switch A->B, B's read fails -> Rule fields empty, no slots or closures listed; Save rule sends no PUT", async () => {
    viewHandler = async (branch: string) => {
      if (branch === B02) {
        return new Response(
          JSON.stringify({ error: { code: 'serverError', message: 'Failed to load' } }),
          { status: 500, headers: { 'content-type': 'application/json' } },
        );
      }
      return new Response(
        JSON.stringify({ version: 1, changed: true, data: BRANCH_A_DATA }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    };

    const { setBranch } = mountWithBranch(B01);
    await settle();

    const { slotMinutesInput, capacityInput, horizonDaysInput, saveRuleBtn } = ruleInputs();
    expect(slotMinutesInput.value).toBe('30');
    expect(capacityInput.value).toBe('8');
    expect(horizonDaysInput.value).toBe('21');
    expect(container.textContent).toContain('Christmas Day');

    setBranch(B02);
    await settle();

    expect(slotMinutesInput.value).toBe('');
    expect(capacityInput.value).toBe('');
    expect(horizonDaysInput.value).toBe('');
    expect(container.textContent).not.toContain('Christmas Day');
    expect(container.textContent).toContain(table.th.noClosures);
    expect(container.textContent).toContain(table.th.noSlots);

    saveRuleBtn.click();
    await settle();

    const rulePuts = sentRequests.filter((r) => r.method === 'PUT' && r.path.endsWith('/booking-slot-rule'));
    expect(rulePuts.length).toBe(0);
    expect(container.querySelector('[role="alert"], .error, .field-error')?.textContent).toBe(
      table.th.ruleValidationFailed,
    );
  });

  it("row 2: Switch A->B, A answers after B -> B's data shown; A's late answer is ignored", async () => {
    const slowA = deferred<Response>();
    viewHandler = (branch: string) => {
      if (branch === B01) {
        return slowA.promise;
      }
      return Promise.resolve(
        new Response(
          JSON.stringify({ version: 1, changed: true, data: BRANCH_B_DATA }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      );
    };

    const { setBranch } = mountWithBranch(B01);
    await settle();

    // Switch to B before A answers
    setBranch(B02);
    await settle();

    const { slotMinutesInput, capacityInput, horizonDaysInput } = ruleInputs();
    expect(slotMinutesInput.value).toBe('60');
    expect(capacityInput.value).toBe('12');
    expect(horizonDaysInput.value).toBe('14');
    expect(container.textContent).toContain('Songkran Festival');

    // Now A's slow response arrives after B's
    slowA.resolve(
      new Response(
        JSON.stringify({ version: 1, changed: true, data: BRANCH_A_DATA }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );
    await settle();

    expect(slotMinutesInput.value).toBe('60');
    expect(capacityInput.value).toBe('12');
    expect(horizonDaysInput.value).toBe('14');
    expect(container.textContent).toContain('Songkran Festival');
    expect(container.textContent).not.toContain('Christmas Day');
  });

  it('row 3: A branch with no rule shows empty fields and clears prior branch fields while loading', async () => {
    const pendingB = deferred<Response>();
    viewHandler = (branch: string) => {
      if (branch === B02) {
        return pendingB.promise;
      }
      return Promise.resolve(
        new Response(
          JSON.stringify({ version: 1, changed: true, data: BRANCH_A_DATA }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      );
    };

    const { setBranch } = mountWithBranch(B01);
    await settle();

    const { slotMinutesInput, capacityInput, horizonDaysInput } = ruleInputs();
    expect(slotMinutesInput.value).toBe('30');

    // Switching to B immediately clears A's branch-specific state while B is in flight
    setBranch(B02);
    expect(slotMinutesInput.value).toBe('');
    expect(capacityInput.value).toBe('');
    expect(horizonDaysInput.value).toBe('');
    expect(container.textContent).not.toContain('Christmas Day');

    // B resolves with rule: null -> fields stay empty
    pendingB.resolve(
      new Response(
        JSON.stringify({ version: 1, changed: true, data: NO_RULE_DATA }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );
    await settle();

    expect(slotMinutesInput.value).toBe('');
    expect(capacityInput.value).toBe('');
    expect(horizonDaysInput.value).toBe('');
  });

  it('row 4: Normal load shows rule, closures, slots and policy, and keeps org-wide policy on branch switch', async () => {
    const pendingB = deferred<Response>();
    viewHandler = (branch: string) => {
      if (branch === B02) {
        return pendingB.promise;
      }
      return Promise.resolve(
        new Response(
          JSON.stringify({ version: 1, changed: true, data: BRANCH_A_DATA }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      );
    };

    const { setBranch } = mountWithBranch(B01);
    await settle();

    const { slotMinutesInput, capacityInput, horizonDaysInput } = ruleInputs();
    expect(slotMinutesInput.value).toBe('30');
    expect(capacityInput.value).toBe('8');
    expect(horizonDaysInput.value).toBe('21');
    expect(container.textContent).toContain('2026-12-25');
    expect(container.textContent).toContain('Christmas Day');
    expect(container.textContent).toContain('OPEN');

    const cutoffInput = container.querySelector<HTMLInputElement>(
      'input[name="cancelCutoffMinutes"], input[data-field="cancelCutoffMinutes"]',
    );
    const graceInput = container.querySelector<HTMLInputElement>(
      'input[name="noShowGraceMinutes"], input[data-field="noShowGraceMinutes"]',
    );
    const expiryInput = container.querySelector<HTMLInputElement>(
      'input[name="confirmExpiryMinutes"], input[data-field="confirmExpiryMinutes"]',
    );
    expect(cutoffInput?.value).toBe('45');
    expect(graceInput?.value).toBe('20');
    expect(expiryInput?.value).toBe('30');

    // Switching branch clears branch-specific state but keeps org-wide policy
    setBranch(B02);
    expect(slotMinutesInput.value).toBe('');
    expect(cutoffInput?.value).toBe('45');
    expect(graceInput?.value).toBe('20');
    expect(expiryInput?.value).toBe('30');
  });

  it("edge case: Switching back to A while B is in flight -> A's newest answer wins; B's is dropped", async () => {
    let aCalls = 0;
    const pendingB = deferred<Response>();

    viewHandler = (branch: string) => {
      if (branch === B02) {
        return pendingB.promise;
      }
      aCalls++;
      const ruleMinutes = aCalls === 1 ? 30 : 45;
      return Promise.resolve(
        new Response(
          JSON.stringify({
            version: aCalls,
            changed: true,
            data: {
              ...BRANCH_A_DATA,
              rule: { ...BRANCH_A_RULE, slotMinutes: ruleMinutes },
            },
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      );
    };

    const { setBranch } = mountWithBranch(B01);
    await settle();

    const { slotMinutesInput, capacityInput, horizonDaysInput } = ruleInputs();
    expect(slotMinutesInput.value).toBe('30');

    // Switch to B (stays in flight)
    setBranch(B02);
    await settle();

    // Switch back to A while B is still in flight
    setBranch(B01);
    await settle();

    expect(slotMinutesInput.value).toBe('45');
    expect(capacityInput.value).toBe('8');
    expect(horizonDaysInput.value).toBe('21');
    expect(container.textContent).toContain('Christmas Day');

    // Now B's in-flight response resolves; it must be dropped
    pendingB.resolve(
      new Response(
        JSON.stringify({ version: 1, changed: true, data: BRANCH_B_DATA }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
    );
    await settle();

    expect(slotMinutesInput.value).toBe('45');
    expect(capacityInput.value).toBe('8');
    expect(horizonDaysInput.value).toBe('21');
    expect(container.textContent).toContain('Christmas Day');
    expect(container.textContent).not.toContain('Songkran Festival');
  });

  it("edge case: A -> B -> A where stale reads (B, then first A) fail after newest A answer -> A's newest values shown and no error element", async () => {
    let aCalls = 0;
    const firstA = deferred<Response>();
    const pendingB = deferred<Response>();

    viewHandler = (branch: string) => {
      if (branch === B02) {
        return pendingB.promise;
      }
      aCalls++;
      if (aCalls === 1) {
        return firstA.promise;
      }
      return Promise.resolve(
        new Response(
          JSON.stringify({
            version: 2,
            changed: true,
            data: {
              ...BRANCH_A_DATA,
              rule: { ...BRANCH_A_RULE, slotMinutes: 45 },
            },
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        ),
      );
    };

    const { setBranch } = mountWithBranch(B01);
    await settle();

    // Switch to B while first A is still in flight
    setBranch(B02);
    await settle();

    // Switch back to A; second A answers immediately with slotMinutes 45
    setBranch(B01);
    await settle();

    const { slotMinutesInput, capacityInput, horizonDaysInput } = ruleInputs();
    expect(slotMinutesInput.value).toBe('45');
    expect(capacityInput.value).toBe('8');
    expect(horizonDaysInput.value).toBe('21');
    expect(container.textContent).toContain('Christmas Day');
    expect(container.querySelector('[role="alert"], .error, .field-error')).toBeNull();

    // Now B's stale read fails 500, then first A's stale read fails 500
    pendingB.resolve(
      new Response(
        JSON.stringify({ error: { code: 'serverError', message: 'B failed' } }),
        { status: 500, headers: { 'content-type': 'application/json' } },
      ),
    );
    await settle();

    firstA.resolve(
      new Response(
        JSON.stringify({ error: { code: 'serverError', message: 'First A failed' } }),
        { status: 500, headers: { 'content-type': 'application/json' } },
      ),
    );
    await settle();

    expect(slotMinutesInput.value).toBe('45');
    expect(capacityInput.value).toBe('8');
    expect(horizonDaysInput.value).toBe('21');
    expect(container.textContent).toContain('Christmas Day');
    expect(container.querySelector('[role="alert"], .error, .field-error')).toBeNull();
  });

  it('edge case: same-branch read fails after a successful load (Save rule reload 500) -> rule fields, closures and slots are cleared', async () => {
    let viewCount = 0;
    viewHandler = async () => {
      viewCount++;
      if (viewCount === 1) {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: BRANCH_A_DATA }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      return new Response(
        JSON.stringify({ error: { code: 'serverError', message: 'Reload failed' } }),
        { status: 500, headers: { 'content-type': 'application/json' } },
      );
    };

    mountWithBranch(B01);
    await settle();

    const { slotMinutesInput, capacityInput, horizonDaysInput, saveRuleBtn } = ruleInputs();
    expect(slotMinutesInput.value).toBe('30');
    expect(capacityInput.value).toBe('8');
    expect(horizonDaysInput.value).toBe('21');
    expect(container.textContent).toContain('Christmas Day');
    expect(container.textContent).toContain('OPEN');

    // Save rule succeeds, triggering same-branch loadView(B01) which fails 500
    saveRuleBtn.click();
    await settle();

    expect(slotMinutesInput.value).toBe('');
    expect(capacityInput.value).toBe('');
    expect(horizonDaysInput.value).toBe('');
    expect(container.textContent).not.toContain('Christmas Day');
    expect(container.textContent).toContain(table.th.noClosures);
    expect(container.textContent).toContain(table.th.noSlots);
    expect(container.querySelector('[role="alert"], .error, .field-error')).not.toBeNull();
  });
});
