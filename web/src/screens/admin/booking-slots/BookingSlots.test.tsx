// Written before the implementation by the test writer gemini/warut-1 (ticket author claude/e92db6), against BK-6 provided tests 7f8e56d1. Do not edit, delete or weaken.
// Amended by gemini/warut-2 per reviewer claude/dell-claude-3 (#1410 issuecomment-6159215412).

import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { meOf } from '../../../test-support/me';
import { AdminShell } from '../AdminShell';
import type { AdminMenuProps } from '../menus';
import { menus } from '../menus';
import { BookingSlots } from './BookingSlots';
import * as table from './i18n';
import { menu } from './menu';
import type {
  AdminBookingPolicyDto,
  AdminBookingSlotsData,
  AdminClosureDto,
  AdminSlotDto,
  AdminSlotRuleDto,
} from '../../../shared/api/generated/admin_booking_slots';

interface SentRequest {
  method: string;
  path: string;
  search: string;
  body: unknown;
  headers: Record<string, string>;
}

const B01 = 'XC-B01';

const SAMPLE_SLOT_RULE = {
  branchId: B01,
  slotMinutes: 30,
  capacity: 8,
  horizonDays: 21,
} satisfies AdminSlotRuleDto;

const SAMPLE_CLOSURE = {
  branchId: B01,
  closedOn: '2026-12-25',
  reason: 'Christmas Day',
} satisfies AdminClosureDto;

const SAMPLE_SLOT = {
  id: 'slot-uuid-1',
  branchId: B01,
  serviceDate: '2026-10-15',
  startAt: '2026-10-15T09:00:00Z',
  endAt: '2026-10-15T09:30:00Z',
  capacity: 8,
  reserved: 2,
  status: 'OPEN',
} satisfies AdminSlotDto;

const SAMPLE_POLICY = {
  cancelCutoffMinutes: 45,
  noShowGraceMinutes: 20,
  confirmExpiryMinutes: 30,
} satisfies AdminBookingPolicyDto;

const SAMPLE_VIEW_DATA = {
  rule: SAMPLE_SLOT_RULE,
  closures: [SAMPLE_CLOSURE],
  slots: [SAMPLE_SLOT],
  policy: SAMPLE_POLICY,
} satisfies AdminBookingSlotsData;

const NO_RULE_VIEW_DATA = {
  rule: null,
  closures: [],
  slots: [],
  policy: SAMPLE_POLICY,
} satisfies AdminBookingSlotsData;

function dummyProps(overrides: Partial<AdminMenuProps> = {}): AdminMenuProps {
  const [branch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
  const [shown] = createSignal(true);
  return {
    branch,
    period,
    shown,
    ...overrides,
  };
}

describe('BK-9: Admin Booking Slots Screen (A04 #1410 §2, §7, §8, §9)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];

  let viewStatus = 200;
  let viewData: unknown = SAMPLE_VIEW_DATA;
  let viewErrorCode: string | undefined = undefined;

  let commandStatus = 200;
  let commandOutput: unknown = {};
  let commandErrorCode: string | undefined = undefined;

  beforeEach(() => {
    setLang('th');
    sharedSession().updateMe(meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }));
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];

    viewStatus = 200;
    viewData = SAMPLE_VIEW_DATA;
    viewErrorCode = undefined;

    commandStatus = 200;
    commandOutput = {};
    commandErrorCode = undefined;

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers: Record<string, string> = {};
      if (init?.headers) {
        new Headers(init.headers).forEach((v, k) => {
          headers[k.toLowerCase()] = v;
        });
      }
      let body: unknown = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }

      sentRequests.push({ method, path: u.pathname, search: u.search, body, headers });

      if (u.pathname === '/api/v3/views/admin_booking_slots') {
        if (viewStatus !== 200) {
          return new Response(
            JSON.stringify({ error: { code: viewErrorCode ?? 'serverError', message: 'Failed to load' } }),
            { status: viewStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: viewData,
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      // Commands
      const isSlotRuleSave = /^\/api\/branches\/[^/]+\/booking-slot-rule$/.test(u.pathname) && method === 'PUT';
      const isClosureAdd = /^\/api\/branches\/[^/]+\/closures$/.test(u.pathname) && method === 'POST';
      const isClosureRemove = /^\/api\/branches\/[^/]+\/closures\/[^/]+\/remove$/.test(u.pathname) && method === 'POST';
      const isSlotCapacitySet = /^\/api\/booking-slots\/[^/]+\/capacity$/.test(u.pathname) && method === 'PUT';
      const isSlotClose = /^\/api\/booking-slots\/[^/]+\/close$/.test(u.pathname) && method === 'POST';
      const isSlotOpen = /^\/api\/booking-slots\/[^/]+\/open$/.test(u.pathname) && method === 'POST';
      const isPolicySet = u.pathname === '/api/booking-policy' && method === 'PUT';

      if (
        isSlotRuleSave ||
        isClosureAdd ||
        isClosureRemove ||
        isSlotCapacitySet ||
        isSlotClose ||
        isSlotOpen ||
        isPolicySet
      ) {
        if (commandStatus !== 200) {
          return new Response(
            JSON.stringify({
              error: {
                code: commandErrorCode ?? 'error',
                message: 'Command failed',
              },
            }),
            { status: commandStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(JSON.stringify(commandOutput), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      return new Response(JSON.stringify({ error: 'not found' }), { status: 404 });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = null;
    container.remove();
    vi.restoreAllMocks();
  });

  function mountComponent(props: Partial<AdminMenuProps> = {}) {
    dispose = render(() => <BookingSlots {...dummyProps(props)} />, container);
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  // ── 1. Role and Permission Gate (§2, §4, §9 row 1) ──────────────────────────
  describe('1. Role and Permission Gate (§2, §4, §9 row 1)', () => {
    it("pins menu metadata: id 'booking-slots', needs 'BranchesManage', group 'desk', order 25", () => {
      expect(menu, 'menu is defined').toBeDefined();
      expect(menu?.id).toBe('booking-slots');
      expect(menu?.needs).toBe('BranchesManage');
      expect(menu?.needs).not.toBe('all');
      expect(menu?.group).toBe('desk');
      expect(menu?.order).toBe(25);
      expect(menu?.dated).toBe(false);
      expect(typeof menu?.label?.()).toBe('string');
      expect(typeof menu?.heading?.title?.()).toBe('string');
    });

    it('visibility via Permission: menu.needs is BranchesManage, not a role string (§2.3)', () => {
      // Must not match role names directly
      expect(menu?.needs, 'menu is defined with needs').toBeDefined();
      expect(menu?.needs).not.toMatch(/^(ADMIN|TELLER|OWNER|MANAGER|BRANCH_MANAGER)$/i);
      expect(menu?.needs).toBe('BranchesManage');
    });

    it('ADMIN (holding BranchesManage) sees booking-slots in AdminShell; TELLER does not (§2.1, §9 row 1)', async () => {
      const branches = [{ id: B01, name: 'Silom Branch' }];

      const adminApi = {
        async view<T>(name: string): Promise<{ version: number; changed: boolean; data?: T }> {
          if (name === 'me') {
            return {
              version: 1,
              changed: true,
              data: {
                ...meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }),
                branchName: 'Silom Branch',
                branches,
              } as unknown as T,
            };
          }
          if (name === 'admin_branches') {
            return { version: 1, changed: true, data: { branches } as unknown as T };
          }
          return { version: 1, changed: false };
        },
        async page<R>(): Promise<R[]> {
          return [];
        },
        async act<R>(): Promise<R> {
          return {} as R;
        },
      } as unknown as ApiClient;

      const tellerApi = {
        async view<T>(name: string): Promise<{ version: number; changed: boolean; data?: T }> {
          if (name === 'me') {
            return {
              version: 1,
              changed: true,
              data: {
                ...meOf({ id: 'teller-1', role: 'TELLER', branch: B01, name: 'Teller' }),
                branchName: 'Silom Branch',
                branches,
              } as unknown as T,
            };
          }
          if (name === 'admin_branches') {
            return { version: 1, changed: true, data: { branches } as unknown as T };
          }
          return { version: 1, changed: false };
        },
        async page<R>(): Promise<R[]> {
          return [];
        },
        async act<R>(): Promise<R> {
          return {} as R;
        },
      } as unknown as ApiClient;

      // 1. ADMIN sees booking-slots
      const adminContainer = document.createElement('div');
      document.body.appendChild(adminContainer);
      const adminDispose = render(() => <AdminShell api={adminApi} menus={menus} />, adminContainer);
      await settle();

      const adminNavLinks = Array.from(adminContainer.querySelectorAll('nav a, aside a'));
      const adminHasBookingSlots = adminNavLinks.some((a) => a.getAttribute('href') === '#booking-slots');
      expect(adminHasBookingSlots, 'ADMIN with BranchesManage sees #booking-slots').toBe(true);
      adminDispose();
      adminContainer.remove();

      // 2. TELLER does NOT see booking-slots
      const tellerContainer = document.createElement('div');
      document.body.appendChild(tellerContainer);
      const tellerDispose = render(() => <AdminShell api={tellerApi} menus={menus} />, tellerContainer);
      await settle();

      const tellerNavLinks = Array.from(tellerContainer.querySelectorAll('nav a, aside a'));
      const tellerHasBookingSlots = tellerNavLinks.some((a) => a.getAttribute('href') === '#booking-slots');
      expect(tellerHasBookingSlots, 'TELLER without BranchesManage does NOT see #booking-slots').toBe(false);
      tellerDispose();
      tellerContainer.remove();
    });
  });

  // ── 2. Edge Case: Branch with No Rule (§9 row 3, §6) ───────────────────────
  describe('2. Branch with No Rule (§9 row 3, §6)', () => {
    it('a branch with no rule shows empty fields, not a default (Q-B1, §9 row 3)', async () => {
      viewData = NO_RULE_VIEW_DATA;
      mountComponent();
      await settle();

      const slotMinutesInput = container.querySelector<HTMLInputElement>(
        'input[name="slotMinutes"], input[data-field="slotMinutes"]',
      );
      const capacityInput = container.querySelector<HTMLInputElement>(
        'input[name="capacity"], input[data-field="capacity"]',
      );
      const horizonDaysInput = container.querySelector<HTMLInputElement>(
        'input[name="horizonDays"], input[data-field="horizonDays"]',
      );

      expect(slotMinutesInput, 'slotMinutes input exists').not.toBeNull();
      expect(capacityInput, 'capacity input exists').not.toBeNull();
      expect(horizonDaysInput, 'horizonDays input exists').not.toBeNull();

      expect(slotMinutesInput?.value, 'slotMinutes is empty string when no rule set').toBe('');
      expect(capacityInput?.value, 'capacity is empty string when no rule set').toBe('');
      expect(horizonDaysInput?.value, 'horizonDays is empty string when no rule set').toBe('');
    });
  });

  // ── 3. View Loading and Field Presentation (§7 [required], §8) ─────────────
  describe('3. View Loading and Field Presentation (§7 [required], §8)', () => {
    it('loads and displays rule, closures, slots, and policy from admin_booking_slots view', async () => {
      viewData = SAMPLE_VIEW_DATA;
      mountComponent();
      await settle();

      expect(sentRequests.some((r) => r.path === '/api/v3/views/admin_booking_slots'), 'view requested').toBe(true);

      // Rule fields
      const slotMinutesInput = container.querySelector<HTMLInputElement>(
        'input[name="slotMinutes"], input[data-field="slotMinutes"]',
      );
      const capacityInput = container.querySelector<HTMLInputElement>(
        'input[name="capacity"], input[data-field="capacity"]',
      );
      const horizonDaysInput = container.querySelector<HTMLInputElement>(
        'input[name="horizonDays"], input[data-field="horizonDays"]',
      );

      expect(slotMinutesInput?.value).toBe('30');
      expect(capacityInput?.value).toBe('8');
      expect(horizonDaysInput?.value).toBe('21');

      // Closure entry
      expect(container.textContent).toContain('2026-12-25');
      expect(container.textContent).toContain('Christmas Day');

      // Slot entry
      expect(container.textContent).toContain('OPEN');

      // Policy fields
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
    });
  });

  // ── 4. slotRuleSet Command (§4.4, §7 [required], §9 row 2) ────────────────
  describe('4. slotRuleSet Command (§4.4, §7 [required], §9 row 2)', () => {
    it('submitting rule sends PUT /api/branches/{id}/booking-slot-rule with Idempotency-Key', async () => {
      mountComponent();
      await settle();

      const slotMinutesInput = container.querySelector<HTMLInputElement>(
        'input[name="slotMinutes"], input[data-field="slotMinutes"]',
      );
      const capacityInput = container.querySelector<HTMLInputElement>(
        'input[name="capacity"], input[data-field="capacity"]',
      );
      const horizonDaysInput = container.querySelector<HTMLInputElement>(
        'input[name="horizonDays"], input[data-field="horizonDays"]',
      );

      expect(slotMinutesInput, 'slotMinutes input exists').not.toBeNull();
      expect(capacityInput, 'capacity input exists').not.toBeNull();
      expect(horizonDaysInput, 'horizonDays input exists').not.toBeNull();

      slotMinutesInput!.value = '60';
      slotMinutesInput!.dispatchEvent(new Event('input', { bubbles: true }));
      capacityInput!.value = '10';
      capacityInput!.dispatchEvent(new Event('input', { bubbles: true }));
      horizonDaysInput!.value = '30';
      horizonDaysInput!.dispatchEvent(new Event('input', { bubbles: true }));

      const saveRuleBtn = [...container.querySelectorAll('button')].find((b) =>
        /save|บันทึก|rule|กฎ/i.test(b.textContent ?? ''),
      );
      expect(saveRuleBtn, 'save rule button exists').toBeDefined();
      saveRuleBtn?.click();
      await settle();

      const ruleRequests = sentRequests.filter(
        (r) => r.method === 'PUT' && r.path === `/api/branches/${B01}/booking-slot-rule`,
      );
      expect(ruleRequests.length, 'slotRuleSet sent to /api/branches/{id}/booking-slot-rule').toBe(1);

      const req = ruleRequests[0]!;
      expect(req.headers['idempotency-key'], 'carries Idempotency-Key header').toBeTruthy();
      expect(req.body).toEqual({ slotMinutes: 60, capacity: 10, horizonDays: 30, branch: B01 });
    });

    it('enforces rule field boundaries: slot_minutes 5–240, capacity > 0, horizon_days 1–90 (§7 [required])', async () => {
      mountComponent();
      await settle();

      const slotMinutesInput = container.querySelector<HTMLInputElement>(
        'input[name="slotMinutes"], input[data-field="slotMinutes"]',
      );
      const capacityInput = container.querySelector<HTMLInputElement>(
        'input[name="capacity"], input[data-field="capacity"]',
      );
      const horizonDaysInput = container.querySelector<HTMLInputElement>(
        'input[name="horizonDays"], input[data-field="horizonDays"]',
      );

      expect(slotMinutesInput, 'slotMinutes input exists').not.toBeNull();
      expect(capacityInput, 'capacity input exists').not.toBeNull();
      expect(horizonDaysInput, 'horizonDays input exists').not.toBeNull();

      // Test out-of-bound values: 4 (< 5), 0 (capacity <= 0), 91 (> 90)
      slotMinutesInput!.value = '4';
      slotMinutesInput!.dispatchEvent(new Event('input', { bubbles: true }));
      capacityInput!.value = '0';
      capacityInput!.dispatchEvent(new Event('input', { bubbles: true }));
      horizonDaysInput!.value = '91';
      horizonDaysInput!.dispatchEvent(new Event('input', { bubbles: true }));

      const saveRuleBtn = [...container.querySelectorAll('button')].find((b) =>
        /save|บันทึก|rule|กฎ/i.test(b.textContent ?? ''),
      );
      saveRuleBtn?.click();
      await settle();

      const invalidRequests = sentRequests.filter(
        (r) => r.method === 'PUT' && r.path.includes('/booking-slot-rule'),
      );
      expect(invalidRequests.length, 'client validation blocks out-of-range rule send').toBe(0);
      expect(container.querySelector('[role="alert"], .error, .field-error'), 'validation error displayed').not.toBeNull();
    });
  });

  // ── 5. closureAdd and closureRemove Commands (§4.4, §7, §9 row 2) ───────────
  describe('5. closureAdd and closureRemove Commands (§4.4, §7, §9 row 2)', () => {
    it('adding a closure sends POST /api/branches/{id}/closures with Idempotency-Key', async () => {
      mountComponent();
      await settle();

      const dateInput = container.querySelector<HTMLInputElement>(
        'input[name="closedOn"], input[type="date"], input[data-field="closedOn"]',
      );
      const reasonInput = container.querySelector<HTMLInputElement>(
        'input[name="reason"], input[data-field="reason"]',
      );
      expect(dateInput, 'closure date input exists').not.toBeNull();
      expect(reasonInput, 'closure reason input exists').not.toBeNull();

      dateInput!.value = '2026-11-01';
      dateInput!.dispatchEvent(new Event('input', { bubbles: true }));
      reasonInput!.value = 'Renovation';
      reasonInput!.dispatchEvent(new Event('input', { bubbles: true }));

      const addBtn = [...container.querySelectorAll('button')].find((b) =>
        /add|เพิ่ม|closure|วันหยุด/i.test(b.textContent ?? ''),
      );
      expect(addBtn, 'add closure button exists').toBeDefined();
      addBtn?.click();
      await settle();

      const closureCalls = sentRequests.filter(
        (r) => r.method === 'POST' && r.path === `/api/branches/${B01}/closures`,
      );
      expect(closureCalls.length, 'closureAdd request sent').toBe(1);

      const req = closureCalls[0]!;
      expect(req.headers['idempotency-key'], 'carries Idempotency-Key').toBeTruthy();
      expect((req.body as any)?.closedOn).toBe('2026-11-01');
      expect((req.body as any)?.reason).toBe('Renovation');
      expect((req.body as any)?.branch, '#1913: every booking command names its branch').toBe(B01);
    });

    it('removing a closure sends POST /api/branches/{id}/closures/{date}/remove with Idempotency-Key', async () => {
      mountComponent();
      await settle();

      const removeBtn = [...container.querySelectorAll('button')].find((b) =>
        /remove|delete|ลบ/i.test(b.textContent ?? ''),
      );
      expect(removeBtn, 'remove closure button exists').toBeDefined();
      removeBtn?.click();
      await settle();

      const removeCalls = sentRequests.filter(
        (r) => r.method === 'POST' && r.path === `/api/branches/${B01}/closures/2026-12-25/remove`,
      );
      expect(removeCalls.length, 'closureRemove request sent to correct path').toBe(1);
      expect(removeCalls[0]!.headers['idempotency-key'], 'closureRemove carries Idempotency-Key').toBeTruthy();
      expect((removeCalls[0]!.body as any)?.branch, '#1913: every booking command names its branch').toBe(B01);
    });
  });

  // ── 6. Slot Capacity, Close, and Open Commands (§4.4, §7, §9 row 2) ────────
  describe('6. Slot Capacity, Close, and Open Commands (§4.4, §7, §9 row 2)', () => {
    it('slotCapacitySet sends PUT /api/booking-slots/{id}/capacity with Idempotency-Key', async () => {
      mountComponent();
      await settle();

      const slotCapInput = container.querySelector<HTMLInputElement>(
        'input[name="slotCapacity"], input[data-slot-id="slot-uuid-1"]',
      );
      expect(slotCapInput, 'slot capacity input exists').not.toBeNull();
      slotCapInput!.value = '10';
      slotCapInput!.dispatchEvent(new Event('input', { bubbles: true }));

      const updateBtn = [...container.querySelectorAll('button')].find((b) =>
        /update|บันทึก|ความจุ/i.test(b.textContent ?? ''),
      );
      updateBtn?.click();
      await settle();

      const capCalls = sentRequests.filter(
        (r) => r.method === 'PUT' && r.path === '/api/booking-slots/slot-uuid-1/capacity',
      );
      expect(capCalls.length, 'slotCapacitySet request sent').toBe(1);
      expect(capCalls[0]!.headers['idempotency-key'], 'carries Idempotency-Key').toBeTruthy();
      expect(capCalls[0]!.body).toEqual({ capacity: 10, branch: B01 });
    });

    it('slotClose and slotOpen send POST to /api/booking-slots/{id}/close and /open with Idempotency-Key', async () => {
      mountComponent();
      await settle();

      const closeBtn = [...container.querySelectorAll('button')].find((b) =>
        /close|ปิดรับจอง/i.test(b.textContent ?? ''),
      );
      expect(closeBtn, 'slot close button exists').toBeDefined();
      closeBtn?.click();
      await settle();

      const closeCalls = sentRequests.filter(
        (r) => r.method === 'POST' && r.path === '/api/booking-slots/slot-uuid-1/close',
      );
      expect(closeCalls.length, 'slotClose sent to /api/booking-slots/{id}/close').toBe(1);
      expect(closeCalls[0]!.headers['idempotency-key'], 'slotClose carries Idempotency-Key').toBeTruthy();
      expect((closeCalls[0]!.body as any)?.branch, '#1913: every booking command names its branch').toBe(B01);

      const openBtn = [...container.querySelectorAll('button')].find((b) =>
        /open|เปิดรับจอง/i.test(b.textContent ?? ''),
      );
      expect(openBtn, 'slot open button exists').toBeDefined();
      openBtn?.click();
      await settle();

      const openCalls = sentRequests.filter(
        (r) => r.method === 'POST' && r.path === '/api/booking-slots/slot-uuid-1/open',
      );
      expect(openCalls.length, 'slotOpen sent to /api/booking-slots/{id}/open').toBe(1);
      expect(openCalls[0]!.headers['idempotency-key'], 'slotOpen carries Idempotency-Key').toBeTruthy();
      expect((openCalls[0]!.body as any)?.branch, '#1913: every booking command names its branch').toBe(B01);
    });
  });

  // ── 7. bookingPolicySet Command (§4.4, §7 [required], §9 row 2) ───────────
  describe('7. bookingPolicySet Command (§4.4, §7 [required], §9 row 2)', () => {
    it('submitting policy sends PUT /api/booking-policy with Idempotency-Key', async () => {
      mountComponent();
      await settle();

      const cutoffInput = container.querySelector<HTMLInputElement>(
        'input[name="cancelCutoffMinutes"], input[data-field="cancelCutoffMinutes"]',
      );
      const graceInput = container.querySelector<HTMLInputElement>(
        'input[name="noShowGraceMinutes"], input[data-field="noShowGraceMinutes"]',
      );
      const expiryInput = container.querySelector<HTMLInputElement>(
        'input[name="confirmExpiryMinutes"], input[data-field="confirmExpiryMinutes"]',
      );

      expect(cutoffInput, 'cancelCutoffMinutes input exists').not.toBeNull();
      expect(graceInput, 'noShowGraceMinutes input exists').not.toBeNull();
      expect(expiryInput, 'confirmExpiryMinutes input exists').not.toBeNull();

      cutoffInput!.value = '50';
      cutoffInput!.dispatchEvent(new Event('input', { bubbles: true }));
      graceInput!.value = '15';
      graceInput!.dispatchEvent(new Event('input', { bubbles: true }));
      expiryInput!.value = '40';
      expiryInput!.dispatchEvent(new Event('input', { bubbles: true }));

      const savePolicyBtn = [...container.querySelectorAll('button')].find((b) =>
        /policy|นโยบาย/i.test(b.textContent ?? ''),
      );
      expect(savePolicyBtn, 'save policy button exists').toBeDefined();
      savePolicyBtn?.click();
      await settle();

      const polCalls = sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy');
      expect(polCalls.length, 'bookingPolicySet sent to /api/booking-policy').toBe(1);

      const req = polCalls[0]!;
      expect(req.headers['idempotency-key'], 'carries Idempotency-Key').toBeTruthy();
      expect(req.body).toEqual({ cancelCutoffMinutes: 50, noShowGraceMinutes: 15, confirmExpiryMinutes: 40, branch: B01 });
    });

    it('enforces policy boundary checks: cutoff >= 0, grace >= 0, expiry > 0 (§7 [required])', async () => {
      mountComponent();
      await settle();

      const cutoffInput = container.querySelector<HTMLInputElement>(
        'input[name="cancelCutoffMinutes"], input[data-field="cancelCutoffMinutes"]',
      );
      const graceInput = container.querySelector<HTMLInputElement>(
        'input[name="noShowGraceMinutes"], input[data-field="noShowGraceMinutes"]',
      );
      const expiryInput = container.querySelector<HTMLInputElement>(
        'input[name="confirmExpiryMinutes"], input[data-field="confirmExpiryMinutes"]',
      );

      expect(cutoffInput, 'cancelCutoffMinutes input exists').not.toBeNull();
      expect(graceInput, 'noShowGraceMinutes input exists').not.toBeNull();
      expect(expiryInput, 'confirmExpiryMinutes input exists').not.toBeNull();

      cutoffInput!.value = '-1';
      cutoffInput!.dispatchEvent(new Event('input', { bubbles: true }));
      graceInput!.value = '-5';
      graceInput!.dispatchEvent(new Event('input', { bubbles: true }));
      expiryInput!.value = '0';
      expiryInput!.dispatchEvent(new Event('input', { bubbles: true }));

      const savePolicyBtn = [...container.querySelectorAll('button')].find((b) =>
        /policy|นโยบาย/i.test(b.textContent ?? ''),
      );
      savePolicyBtn?.click();
      await settle();

      const invalidCalls = sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy');
      expect(invalidCalls.length, 'client validation blocks invalid policy values').toBe(0);
      expect(container.querySelector('[role="alert"], .error, .field-error')).not.toBeNull();
    });
  });

  // ── 8. Idempotency-Key Retention Across Retries (§9 row 2, §7 [required]) ───
  describe('8. Idempotency-Key Retention Across Retries (§9 row 2, §7 [required])', () => {
    it('commands reuse the exact same Idempotency-Key across retries', async () => {
      commandStatus = 500;
      mountComponent();
      await settle();

      const saveRuleBtn = [...container.querySelectorAll('button')].find((b) =>
        /save|บันทึก/i.test(b.textContent ?? ''),
      );
      expect(saveRuleBtn, 'action button exists').toBeDefined();
      saveRuleBtn?.click();
      await settle();

      const firstCall = sentRequests[sentRequests.length - 1];
      const firstKey = firstCall?.headers['idempotency-key'];
      expect(firstKey, 'first request carries Idempotency-Key').toBeTruthy();

      // Retry same action
      saveRuleBtn?.click();
      await settle();

      const secondCall = sentRequests[sentRequests.length - 1];
      const secondKey = secondCall?.headers['idempotency-key'];
      expect(secondKey, 'retry reuses identical Idempotency-Key header').toBe(firstKey);
    });
  });

  // ── 9. Worded Error Presentation for Refusal Codes (§8) ────────────────────
  describe('9. Worded Error Presentation for Refusal Codes (§8)', () => {
    const errorCodes = [
      'capacityBelowReserved',
      'slotHasBookings',
      'validationFailed',
      'roleNotAllowed',
      'roleNotAllowedHere',
    ];

    for (const code of errorCodes) {
      it(`shows worded error message and never raw code for ${code} (§8)`, async () => {
        commandStatus = 400;
        commandErrorCode = code;
        mountComponent();
        await settle();

        const actionBtn = [...container.querySelectorAll('button')].find((b) =>
          /save|บันทึก|update|แก้ไข/i.test(b.textContent ?? ''),
        );
        expect(actionBtn, 'action button exists').toBeDefined();
        actionBtn?.click();
        await settle();

        const expectedText = table.th[`error.${code}`] ?? table.th[`err.${code}`];
        expect(expectedText, `table.th has key error.${code} or err.${code}`).toBeDefined();

        // The error alert must be present
        const alert = container.querySelector('[role="alert"], .error, .toast-error');
        expect(alert, `alert element rendered for ${code}`).not.toBeNull();
        expect(container.textContent).toContain(expectedText);
        expect(container.textContent).not.toContain(code); // must not show raw error code
      });
    }
  });

  // ── 10. Numbers and Times Sent Exactly as Typed ─────────────────────────────
  describe('10. Numbers and Times Sent Exactly as Typed', () => {
    it('sends numbers and dates exactly as typed without conversion artifacts', async () => {
      mountComponent();
      await settle();

      const dateInput = container.querySelector<HTMLInputElement>(
        'input[name="closedOn"], input[type="date"], input[data-field="closedOn"]',
      );
      const reasonInput = container.querySelector<HTMLInputElement>(
        'input[name="reason"], input[data-field="reason"]',
      );

      expect(dateInput, 'dateInput exists').not.toBeNull();
      expect(reasonInput, 'reasonInput exists').not.toBeNull();

      const typedDate = '2026-11-20';
      const typedReason = 'Staff Training';
      dateInput!.value = typedDate;
      dateInput!.dispatchEvent(new Event('input', { bubbles: true }));
      reasonInput!.value = typedReason;
      reasonInput!.dispatchEvent(new Event('input', { bubbles: true }));

      const addBtn = [...container.querySelectorAll('button')].find((b) =>
        /add|เพิ่ม/i.test(b.textContent ?? ''),
      );
      addBtn?.click();
      await settle();

      const closureReq = sentRequests.find((r) => r.path === `/api/branches/${B01}/closures`);
      expect((closureReq?.body as any)?.closedOn, 'date sent exactly as typed').toBe(typedDate);
      expect((closureReq?.body as any)?.reason, 'reason sent exactly as typed').toBe(typedReason);
    });
  });

  // ── 11. i18n Key Parity ────────────────────────────────────────────────────
  describe('11. i18n Key Parity', () => {
    it('th and en tables have identical, non-empty key sets', () => {
      const thKeys = Object.keys(table.th).sort();
      const enKeys = Object.keys(table.en).sort();
      expect(thKeys).toEqual(enKeys);
      expect(thKeys.length, 'keys are non-empty').toBeGreaterThan(0);
    });
  });
});
