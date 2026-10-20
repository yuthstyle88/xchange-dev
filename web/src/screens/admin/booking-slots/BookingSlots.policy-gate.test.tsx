// BK-9b (#2090): show the A04 booking-policy section only to an org-wide ADMIN
// (design `docs/design/blueprint/booking-slots.md` §9 row BK-9b).
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
} from '../../../shared/testing/fixtures';
import { warmModules } from '../../../shared/testing/warm-modules';
import { meOf } from '../../../test-support/me';
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

interface SentRequest {
  method: string;
  path: string;
  body: unknown;
}

function menuProps(): AdminMenuProps {
  const [branch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
  const [shown] = createSignal(true);
  return { branch, period, shown };
}

describe('BK-9b: A04 BookingSlots policy section org-wide ADMIN gate (#2090 §9)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: SentRequest[];

  beforeAll(() => warmModules(() => import('./BookingSlots')), 60_000);

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];
    sharedSession().updateMe(undefined as never);

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : null;
      sentRequests.push({ method, path: u.pathname, body });

      if (method === 'GET' && u.pathname === '/api/v3/views/admin_booking_slots') {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: structuredClone(SAMPLE_VIEW_DATA) }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (method === 'PUT' && u.pathname === '/api/booking-policy') {
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

  function hasSections1To3(): boolean {
    const rule = container.querySelector('input[name="slotMinutes"]') !== null;
    const closures = container.querySelector('input[name="closedOn"]') !== null;
    const slots = container.querySelector('[data-slot-id="slot-uuid-1"]') !== null;
    return rule && closures && slots;
  }

  it('row 1: ADMIN with branchId: null (org-wide) — policy section shown and save sends PUT /api/booking-policy', async () => {
    sharedSession().updateMe(
      meOf({ id: 'admin-org', role: 'ADMIN', branch: B01, name: 'Org Admin', orgWide: true }),
    );
    await mount();

    expect(hasPolicySection(), 'policy section is shown for org-wide ADMIN').toBe(true);

    const cutoffInput = container.querySelector<HTMLInputElement>('input[name="cancelCutoffMinutes"]')!;
    const graceInput = container.querySelector<HTMLInputElement>('input[name="noShowGraceMinutes"]')!;
    const expiryInput = container.querySelector<HTMLInputElement>('input[name="confirmExpiryMinutes"]')!;
    expect(cutoffInput.value).toBe('45');
    expect(graceInput.value).toBe('20');
    expect(expiryInput.value).toBe('30');

    cutoffInput.value = '60';
    cutoffInput.dispatchEvent(new Event('input', { bubbles: true }));
    graceInput.value = '15';
    graceInput.dispatchEvent(new Event('input', { bubbles: true }));
    expiryInput.value = '25';
    expiryInput.dispatchEvent(new Event('input', { bubbles: true }));

    const savePolicyBtn = Array.from(container.querySelectorAll('button')).find(
      (b) => b.textContent?.trim() === table.th.savePolicy,
    );
    expect(savePolicyBtn, 'savePolicy button exists').toBeDefined();
    savePolicyBtn!.click();
    await settle();

    const policyPuts = sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy');
    expect(policyPuts).toHaveLength(1);
    expect(policyPuts[0]!.body).toEqual({
      cancelCutoffMinutes: 60,
      noShowGraceMinutes: 15,
      confirmExpiryMinutes: 25,
      branch: B01,
    });
  });

  it('row 2: ADMIN at XC-B01 only (branch-scoped) — policy section not in the DOM; rule, closures and slots sections are shown', async () => {
    sharedSession().updateMe(
      meOf({ id: 'admin-branch', role: 'ADMIN', branch: B01, name: 'Branch Admin', orgWide: false }),
    );
    await mount();

    expect(hasSections1To3(), 'rule, closures and slots sections are shown').toBe(true);
    expect(hasPolicySection(), 'policy section is not in the DOM for branch-scoped ADMIN').toBe(false);
  });

  it('row 3: ADMIN at XC-B01 plus a second ADMIN row with branchId: null — policy section is shown', async () => {
    const base = meOf({ id: 'admin-both', role: 'ADMIN', branch: B01, name: 'Dual Admin', orgWide: false });
    sharedSession().updateMe({
      ...base,
      roles: [
        { role: 'ADMIN', branchId: B01 },
        { role: 'ADMIN', branchId: null },
      ],
    });
    await mount();

    expect(hasSections1To3(), 'rule, closures and slots sections are shown').toBe(true);
    expect(hasPolicySection(), 'policy section is shown when any ADMIN row has branchId: null').toBe(true);
  });

  it('row 4: me not loaded yet (undefined) — policy section is hidden, with no error and no policy request', async () => {
    sharedSession().updateMe(undefined as never);
    await mount();

    expect(hasSections1To3(), 'rule, closures and slots sections are shown').toBe(true);
    expect(hasPolicySection(), 'policy section is hidden when me is undefined').toBe(false);
    expect(container.querySelector('[role="alert"], .error'), 'no error alert rendered').toBeNull();
    expect(
      sentRequests.filter((r) => r.path === '/api/booking-policy'),
      'no policy request sent',
    ).toHaveLength(0);
  });

  it('row 5: an org-wide row of another role (e.g. OWNER / EXECUTIVE, branchId: null) with no ADMIN row — policy section is hidden', async () => {
    const base = meOf({ id: 'exec-org', role: 'EXECUTIVE', branch: B01, name: 'Exec Only', orgWide: true });
    sharedSession().updateMe({
      ...base,
      roles: [
        { role: 'OWNER', branchId: null },
        { role: 'EXECUTIVE', branchId: null },
      ],
    });
    await mount();

    expect(hasSections1To3(), 'rule, closures and slots sections are shown').toBe(true);
    expect(hasPolicySection(), 'policy section is hidden without an org-wide ADMIN row').toBe(false);
  });
});
