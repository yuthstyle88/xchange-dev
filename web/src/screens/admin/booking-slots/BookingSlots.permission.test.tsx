import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import {
  makeAdminBookingPolicyDto,
  makeAdminBookingSlotsData,
  makeAdminClosureDto,
  makeAdminSlotDto,
  makeAdminSlotRuleDto,
  makeMe,
  makeMeRole,
} from '../../../shared/testing/fixtures';
import { warmModules } from '../../../shared/testing/warm-modules';
import type { AdminMenuProps } from '../menus';
import { BookingSlots } from './BookingSlots';
import * as table from './i18n';

const B01 = 'XC-B01';

const SAMPLE_VIEW_DATA = makeAdminBookingSlotsData({
  rule: makeAdminSlotRuleDto({
    branchId: B01,
    slotMinutes: 30,
    capacity: 8,
    horizonDays: 21,
  }),
  closures: [
    makeAdminClosureDto({
      branchId: B01,
      closedOn: '2026-12-25',
      reason: 'Christmas Day',
    }),
  ],
  slots: [
    makeAdminSlotDto({
      id: 'slot-uuid-1',
      branchId: B01,
      serviceDate: '2026-10-15',
      startAt: '2026-10-15T09:00:00Z',
      endAt: '2026-10-15T09:30:00Z',
      capacity: 8,
      reserved: 2,
      status: 'OPEN',
    }),
  ],
  policy: makeAdminBookingPolicyDto({
    cancelCutoffMinutes: 45,
    noShowGraceMinutes: 20,
    confirmExpiryMinutes: 30,
  }),
});

function menuProps(): AdminMenuProps {
  const [branch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
  const [shown] = createSignal(true);
  return { branch, period, shown };
}

describe('RB-W1 (#4731): BookingSlots policy section permission gate', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let putPolicyStatus = 200;
  let putPolicyBody: unknown = {};

  beforeAll(() => warmModules(() => import('./BookingSlots')), 60_000);

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    putPolicyStatus = 200;
    putPolicyBody = {};
    sharedSession().updateMe(undefined as never);

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && u.pathname === '/api/v3/views/admin_booking_slots') {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: structuredClone(SAMPLE_VIEW_DATA) }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (method === 'PUT' && u.pathname === '/api/booking-policy') {
        if (putPolicyStatus !== 200) {
          return new Response(JSON.stringify(putPolicyBody), {
            status: putPolicyStatus,
            headers: { 'content-type': 'application/json' },
          });
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      return new Response(JSON.stringify({ version: 1, changed: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    container.remove();
    sharedSession().updateMe(undefined as never);
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mount(): Promise<void> {
    dispose = render(() => <BookingSlots {...menuProps()} />, container);
    await settle();
  }

  function hasPolicySection(): boolean {
    const cutoff = container.querySelector('input[name="cancelCutoffMinutes"]');
    const grace = container.querySelector('input[name="noShowGraceMinutes"]');
    const expiry = container.querySelector('input[name="confirmExpiryMinutes"]');
    const heading = Array.from(container.querySelectorAll('h3')).some(
      (h) => h.textContent?.trim() === table.th.policySection,
    );
    return Boolean(cutoff || grace || expiry || heading);
  }

  it('R1: roles [ADMIN@org], permissions include BranchesManage — shown (today\'s case, kept)', async () => {
    sharedSession().updateMe(
      makeMe({
        id: 'user-r1',
        roles: [makeMeRole({ role: 'ADMIN', branchId: null })],
        permissions: ['BranchesManage'],
      }),
    );
    await mount();

    expect(hasPolicySection(), 'policy section is shown for org-wide ADMIN with BranchesManage').toBe(true);
  });

  it('R2: roles [EXECUTIVE@org, ADMIN@org] (the owner), permissions include BranchesManage — shown', async () => {
    sharedSession().updateMe(
      makeMe({
        id: 'user-r2',
        roles: [
          makeMeRole({ role: 'EXECUTIVE', branchId: null }),
          makeMeRole({ role: 'ADMIN', branchId: null }),
        ],
        permissions: ['BranchesManage'],
      }),
    );
    await mount();

    expect(hasPolicySection(), 'policy section is shown for owner with BranchesManage').toBe(true);
  });

  it('R3: roles [CUSTOM@org] whose permissions include BranchesManage (the role name is not ADMIN) — shown', async () => {
    sharedSession().updateMe(
      makeMe({
        id: 'user-r3',
        roles: [makeMeRole({ role: 'CUSTOM', branchId: null })],
        permissions: ['BranchesManage'],
      }),
    );
    await mount();

    expect(hasPolicySection(), 'policy section is shown for custom org-wide role with BranchesManage').toBe(true);
  });

  it('R4: roles [ADMIN@org], permissions without BranchesManage — hidden', async () => {
    sharedSession().updateMe(
      makeMe({
        id: 'user-r4',
        roles: [makeMeRole({ role: 'ADMIN', branchId: null })],
        permissions: ['UsersManage'],
      }),
    );
    await mount();

    expect(hasPolicySection(), 'policy section is hidden when permissions lack BranchesManage').toBe(false);
  });

  it('R5: roles [ADMIN@B1] (branch-scoped), permissions include BranchesManage — hidden', async () => {
    sharedSession().updateMe(
      makeMe({
        id: 'user-r5',
        roles: [makeMeRole({ role: 'ADMIN', branchId: B01 })],
        permissions: ['BranchesManage'],
      }),
    );
    await mount();

    expect(hasPolicySection(), 'policy section is hidden for branch-scoped ADMIN').toBe(false);
  });

  it('R6: R1, the save answers 403 outOfScope — the worded error.outOfScope text shows (role="alert"), not validationFailed', async () => {
    putPolicyStatus = 403;
    putPolicyBody = {
      error: {
        code: 'outOfScope',
        message: 'No permission for the whole organisation',
      },
    };

    sharedSession().updateMe(
      makeMe({
        id: 'user-r6',
        roles: [makeMeRole({ role: 'ADMIN', branchId: null })],
        permissions: ['BranchesManage'],
      }),
    );
    await mount();

    expect(hasPolicySection()).toBe(true);

    const savePolicyBtn = Array.from(container.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === table.th.savePolicy,
    );
    expect(savePolicyBtn, 'savePolicy button exists').toBeDefined();
    savePolicyBtn!.click();
    await settle();

    const alert = container.querySelector('[role="alert"], .error');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(table.th['error.outOfScope']);
    expect(alert!.textContent).not.toContain(table.th['error.validationFailed']);

    // Also verify English presentation
    setLang('en');
    savePolicyBtn!.click();
    await settle();
    const alertEn = container.querySelector('[role="alert"], .error');
    expect(alertEn).not.toBeNull();
    expect(alertEn!.textContent).toContain(table.en['error.outOfScope']);
    expect(alertEn!.textContent).not.toContain(table.en['error.validationFailed']);
  });
});
