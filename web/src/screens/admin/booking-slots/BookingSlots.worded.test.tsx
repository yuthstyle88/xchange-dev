import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang, t as sharedT } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { meOf } from '../../../test-support/me';
import type { AdminMenuProps } from '../menus';
import { BookingSlots } from './BookingSlots';
import * as table from './i18n';
import type {
  AdminBookingPolicyDto,
  AdminBookingSlotsData,
  AdminClosureDto,
  AdminSlotDto,
  AdminSlotRuleDto,
} from '../../../shared/api/generated/admin_booking_slots';

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

describe('BookingSlots wordedError presentation (#2182)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;

  let viewStatus = 200;
  let viewErrorCode: string | undefined = undefined;

  let commandStatus = 200;
  let commandErrorCode: string | undefined = undefined;
  let commandNetworkError: Error | null = null;

  beforeEach(() => {
    setLang('th');
    sharedSession().updateMe(meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }));
    container = document.createElement('div');
    document.body.appendChild(container);

    viewStatus = 200;
    viewErrorCode = undefined;

    commandStatus = 200;
    commandErrorCode = undefined;
    commandNetworkError = null;

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (u.pathname === '/api/v3/views/admin_booking_slots') {
        if (viewStatus !== 200) {
          return new Response(
            JSON.stringify({
              error: { code: viewErrorCode ?? 'serverError', message: 'Failed to load' },
            }),
            { status: viewStatus, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: SAMPLE_VIEW_DATA,
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (commandNetworkError && method !== 'GET') {
        throw commandNetworkError;
      }

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

  function mountComponent(props: Partial<AdminMenuProps> = {}) {
    dispose = render(() => <BookingSlots {...dummyProps(props)} />, container);
  }

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  function clickSaveRule() {
    const saveRuleBtn = [...container.querySelectorAll('button')].find((b) =>
      /save|บันทึก/i.test(b.textContent ?? ''),
    );
    expect(saveRuleBtn, 'save rule button exists').toBeDefined();
    saveRuleBtn!.click();
  }

  it('row 1: Save -> 500 shows the wordedError server-error text, not validationFailed', async () => {
    commandStatus = 500;
    commandErrorCode = 'internalError';
    mountComponent();
    await settle();

    clickSaveRule();
    await settle();

    const alert = container.querySelector('[role="alert"], .error, .field-error');
    expect(alert?.textContent).toBe(sharedT('error.serverError'));
    expect(alert?.textContent).not.toBe(table.th['error.validationFailed']);
  });

  it('row 2: Save -> network TypeError shows the wordedError network text', async () => {
    commandNetworkError = new TypeError('Failed to fetch');
    mountComponent();
    await settle();

    clickSaveRule();
    await settle();

    const alert = container.querySelector('[role="alert"], .error, .field-error');
    expect(alert?.textContent).toBe(sharedT('error.network'));
    expect(alert?.textContent).not.toBe(table.th['error.validationFailed']);
  });

  it('row 3: Opening the screen -> admin_booking_slots 500 shows the wordedError server-error text', async () => {
    viewStatus = 500;
    viewErrorCode = 'internalError';
    mountComponent();
    await settle();

    const alert = container.querySelector('[role="alert"], .error, .field-error');
    expect(alert?.textContent).toBe(sharedT('error.serverError'));
    expect(alert?.textContent).not.toBe(table.th['error.validationFailed']);
  });

  it('row 4: 400 with a mapped code (capacityBelowReserved) shows the mapped text', async () => {
    commandStatus = 400;
    commandErrorCode = 'capacityBelowReserved';
    mountComponent();
    await settle();

    clickSaveRule();
    await settle();

    const alert = container.querySelector('[role="alert"], .error, .field-error');
    expect(alert?.textContent).toBe(table.th['error.capacityBelowReserved']);

    // Subsequent 500 shows serverError, not validationFailed
    commandStatus = 500;
    commandErrorCode = 'internalError';
    clickSaveRule();
    await settle();

    expect(container.querySelector('[role="alert"], .error, .field-error')?.textContent).toBe(
      sharedT('error.serverError'),
    );
  });

  it('row 5: 400 with an unknown code shows validationFailed', async () => {
    commandStatus = 400;
    commandErrorCode = 'someUnknownBackendError';
    mountComponent();
    await settle();

    clickSaveRule();
    await settle();

    const alert = container.querySelector('[role="alert"], .error, .field-error');
    expect(alert?.textContent).toBe(table.th['error.validationFailed']);

    // Subsequent 500 shows serverError, distinguishing 5xx from unmapped 4xx
    commandStatus = 500;
    commandErrorCode = 'someUnknownBackendError';
    clickSaveRule();
    await settle();

    expect(container.querySelector('[role="alert"], .error, .field-error')?.textContent).toBe(
      sharedT('error.serverError'),
    );
  });

  it('edge case: 409 with a mapped 4xx code (slotHasBookings) shows mapped text first', async () => {
    commandStatus = 409;
    commandErrorCode = 'slotHasBookings';
    mountComponent();
    await settle();

    const closeBtn = [...container.querySelectorAll('button')].find((b) =>
      /close|ปิดรับจอง/i.test(b.textContent ?? ''),
    );
    expect(closeBtn, 'slot close button exists').toBeDefined();
    closeBtn!.click();
    await settle();

    const alert = container.querySelector('[role="alert"], .error, .field-error');
    expect(alert?.textContent).toBe(table.th['error.slotHasBookings']);

    // Network failure on retry shows network text
    commandNetworkError = new TypeError('Network dropped');
    closeBtn!.click();
    await settle();

    expect(container.querySelector('[role="alert"], .error, .field-error')?.textContent).toBe(
      sharedT('error.network'),
    );
  });
});
