import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { meOf } from '../../../test-support/me';
import type { AdminMenuProps } from '../menus';
import { BookingSlots } from './BookingSlots';
import * as table from './i18n';
import type {
  AdminBookingPolicyDto,
  AdminBookingSlotsData,
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

const SAMPLE_POLICY = {
  cancelCutoffMinutes: 45,
  noShowGraceMinutes: 20,
  confirmExpiryMinutes: 30,
} satisfies AdminBookingPolicyDto;

const SAMPLE_VIEW_DATA = {
  rule: SAMPLE_SLOT_RULE,
  closures: [],
  slots: [],
  policy: SAMPLE_POLICY,
} satisfies AdminBookingSlotsData;

const NO_POLICY_VIEW_DATA = {
  rule: SAMPLE_SLOT_RULE,
  closures: [],
  slots: [],
  policy: null,
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

describe('BookingSlots policy empty/whitespace validation (#2181)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let sentRequests: SentRequest[] = [];
  let viewData: AdminBookingSlotsData = SAMPLE_VIEW_DATA;

  beforeEach(() => {
    setLang('th');
    sharedSession().updateMe(meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }));
    container = document.createElement('div');
    document.body.appendChild(container);
    sentRequests = [];
    viewData = SAMPLE_VIEW_DATA;

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
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: viewData,
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (u.pathname === '/api/booking-policy' && method === 'PUT') {
        return new Response(JSON.stringify({}), {
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

  function policyInputs() {
    const cutoffInput = container.querySelector<HTMLInputElement>(
      'input[name="cancelCutoffMinutes"], input[data-field="cancelCutoffMinutes"]',
    );
    const graceInput = container.querySelector<HTMLInputElement>(
      'input[name="noShowGraceMinutes"], input[data-field="noShowGraceMinutes"]',
    );
    const expiryInput = container.querySelector<HTMLInputElement>(
      'input[name="confirmExpiryMinutes"], input[data-field="confirmExpiryMinutes"]',
    );
    const savePolicyBtn = [...container.querySelectorAll('button')].find((b) =>
      /policy|นโยบาย/i.test(b.textContent ?? ''),
    );
    expect(cutoffInput, 'cancelCutoffMinutes input exists').not.toBeNull();
    expect(graceInput, 'noShowGraceMinutes input exists').not.toBeNull();
    expect(expiryInput, 'confirmExpiryMinutes input exists').not.toBeNull();
    expect(savePolicyBtn, 'save policy button exists').toBeDefined();
    return {
      cutoffInput: cutoffInput!,
      graceInput: graceInput!,
      expiryInput: expiryInput!,
      savePolicyBtn: savePolicyBtn!,
    };
  }

  function setInput(el: HTMLInputElement, val: string) {
    el.value = val;
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }

  it("row 1: Cutoff '', grace 15, expiry 40 -> No PUT; policyValidationFailed shown", async () => {
    viewData = NO_POLICY_VIEW_DATA;
    mountComponent();
    await settle();

    const { cutoffInput, graceInput, expiryInput, savePolicyBtn } = policyInputs();
    expect(cutoffInput.value).toBe('');
    setInput(graceInput, '15');
    setInput(expiryInput, '40');

    savePolicyBtn.click();
    await settle();

    const polCalls = sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy');
    expect(polCalls.length).toBe(0);
    const alert = container.querySelector('[role="alert"], .error, .field-error');
    expect(alert?.textContent).toBe(table.th.policyValidationFailed);
  });

  it("row 2: Grace '  ' (spaces) -> No PUT; policyValidationFailed shown", async () => {
    mountComponent();
    await settle();

    const { cutoffInput, graceInput, expiryInput, savePolicyBtn } = policyInputs();
    setInput(cutoffInput, '50');
    setInput(graceInput, '  ');
    setInput(expiryInput, '40');

    savePolicyBtn.click();
    await settle();

    const polCalls = sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy');
    expect(polCalls.length).toBe(0);
    const alert = container.querySelector('[role="alert"], .error, .field-error');
    expect(alert?.textContent).toBe(table.th.policyValidationFailed);
  });

  it("row 3: Cutoff '0' typed explicitly -> PUT with cancelCutoffMinutes: 0 (an explicit zero stays valid)", async () => {
    viewData = NO_POLICY_VIEW_DATA;
    mountComponent();
    await settle();

    const { cutoffInput, graceInput, expiryInput, savePolicyBtn } = policyInputs();
    setInput(graceInput, '15');
    setInput(expiryInput, '40');

    // Empty cutoff is not treated as 0
    savePolicyBtn.click();
    await settle();
    expect(sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy').length).toBe(0);

    // Explicit '0' is valid and sends cancelCutoffMinutes: 0
    setInput(cutoffInput, '0');
    savePolicyBtn.click();
    await settle();

    const polCalls = sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy');
    expect(polCalls.length).toBe(1);
    expect(polCalls[0]!.body).toEqual({
      cancelCutoffMinutes: 0,
      noShowGraceMinutes: 15,
      confirmExpiryMinutes: 40,
      branch: B01,
    });
  });

  it('row 4: 50 / 15 / 40 -> PUT with exactly these values', async () => {
    viewData = NO_POLICY_VIEW_DATA;
    mountComponent();
    await settle();

    const { cutoffInput, graceInput, expiryInput, savePolicyBtn } = policyInputs();
    setInput(cutoffInput, '50');
    setInput(expiryInput, '40');

    // With grace still empty, no PUT is sent
    savePolicyBtn.click();
    await settle();
    expect(sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy').length).toBe(0);

    // Filling grace with 15 sends 50 / 15 / 40
    setInput(graceInput, '15');
    savePolicyBtn.click();
    await settle();

    const polCalls = sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy');
    expect(polCalls.length).toBe(1);
    expect(polCalls[0]!.body).toEqual({
      cancelCutoffMinutes: 50,
      noShowGraceMinutes: 15,
      confirmExpiryMinutes: 40,
      branch: B01,
    });
  });

  it('row 5: Cutoff -1 -> No PUT; policyValidationFailed shown', async () => {
    mountComponent();
    await settle();

    const { cutoffInput, graceInput, expiryInput, savePolicyBtn } = policyInputs();
    setInput(cutoffInput, '-1');
    setInput(graceInput, '15');
    setInput(expiryInput, '40');

    savePolicyBtn.click();
    await settle();

    expect(sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy').length).toBe(0);
    expect(container.querySelector('[role="alert"], .error, .field-error')?.textContent).toBe(
      table.th.policyValidationFailed,
    );

    // Clearing cutoff to empty also stays refused
    setInput(cutoffInput, '');
    savePolicyBtn.click();
    await settle();

    expect(sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy').length).toBe(0);
    expect(container.querySelector('[role="alert"], .error, .field-error')?.textContent).toBe(
      table.th.policyValidationFailed,
    );
  });

  it("edge case: '1.5' is refused with policyValidationFailed and no PUT", async () => {
    mountComponent();
    await settle();

    const { cutoffInput, graceInput, expiryInput, savePolicyBtn } = policyInputs();
    setInput(cutoffInput, '1.5');
    setInput(graceInput, '15');
    setInput(expiryInput, '40');

    savePolicyBtn.click();
    await settle();

    expect(sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy').length).toBe(0);
    expect(container.querySelector('[role="alert"], .error, .field-error')?.textContent).toBe(
      table.th.policyValidationFailed,
    );

    setInput(cutoffInput, ' ');
    savePolicyBtn.click();
    await settle();

    expect(sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy').length).toBe(0);
  });

  it("edge case: '007' is accepted as 7", async () => {
    viewData = NO_POLICY_VIEW_DATA;
    mountComponent();
    await settle();

    const { cutoffInput, graceInput, expiryInput, savePolicyBtn } = policyInputs();
    setInput(graceInput, '007');
    setInput(expiryInput, '007');

    // Empty cutoff refused first
    savePolicyBtn.click();
    await settle();
    expect(sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy').length).toBe(0);

    setInput(cutoffInput, '007');
    savePolicyBtn.click();
    await settle();

    const polCalls = sentRequests.filter((r) => r.method === 'PUT' && r.path === '/api/booking-policy');
    expect(polCalls.length).toBe(1);
    expect(polCalls[0]!.body).toEqual({
      cancelCutoffMinutes: 7,
      noShowGraceMinutes: 7,
      confirmExpiryMinutes: 7,
      branch: B01,
    });
  });
});
