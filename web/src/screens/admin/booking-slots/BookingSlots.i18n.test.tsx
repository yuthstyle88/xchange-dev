import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
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

function dummyProps(): AdminMenuProps {
  const [branch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
  const [shown] = createSignal(true);
  return { branch, period, shown };
}

describe('BookingSlots - i18n English Refusal Presentation', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;
  let commandStatus = 200;
  let commandErrorCode: string | undefined = undefined;

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);

    commandStatus = 200;
    commandErrorCode = undefined;

    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');

      if (u.pathname === '/api/v3/views/admin_booking_slots') {
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: SAMPLE_VIEW_DATA,
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
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

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  it('shows worded error message in English and not Thai when language is set to EN', async () => {
    setLang('en');
    commandStatus = 400;
    commandErrorCode = 'capacityBelowReserved';

    dispose = render(() => <BookingSlots {...dummyProps()} />, container);
    await settle();

    const actionBtn = [...container.querySelectorAll('button')].find((b) =>
      /save|update|edit/i.test(b.textContent ?? ''),
    );
    expect(actionBtn).toBeDefined();
    actionBtn?.click();
    await settle();

    const expectedEn = table.en['error.capacityBelowReserved'];
    const expectedTh = table.th['error.capacityBelowReserved'];
    expect(expectedEn).toBeDefined();
    expect(expectedTh).toBeDefined();

    const alert = container.querySelector('[role="alert"], .error');
    expect(alert).not.toBeNull();
    expect(container.textContent).toContain(expectedEn);
    expect(container.textContent).not.toContain(expectedTh);
    expect(container.textContent).not.toContain('capacityBelowReserved');
  });

  it('falls back to worded validation failed message in English when error code is unknown', async () => {
    setLang('en');
    commandStatus = 400;
    commandErrorCode = 'someUnknownBackendError';

    dispose = render(() => <BookingSlots {...dummyProps()} />, container);
    await settle();

    const actionBtn = [...container.querySelectorAll('button')].find((b) =>
      /save|update|edit/i.test(b.textContent ?? ''),
    );
    expect(actionBtn).toBeDefined();
    actionBtn?.click();
    await settle();

    const expectedEn = table.en['error.validationFailed'];
    const expectedTh = table.th['error.validationFailed'];
    expect(expectedEn).toBeDefined();

    const alert = container.querySelector('[role="alert"], .error');
    expect(alert).not.toBeNull();
    expect(container.textContent).toContain(expectedEn);
    expect(container.textContent).not.toContain(expectedTh);
    expect(container.textContent).not.toContain('someUnknownBackendError');
  });
});
