// #2495: Admin Booking Slots — render slot startAt / endAt as Bangkok wall-clock times (HH:mm)
// instead of raw UTC timestamps; keep serviceDate as sent.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AdminBookingSlotsData, AdminSlotDto } from '../../../shared/api/generated/admin_booking_slots';
import type { Me } from '../../../shared/api/generated/common';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { meOf } from '../../../test-support/me';
import type { AdminMenuProps } from '../menus';
import { BookingSlots } from './BookingSlots';
import * as table from './i18n';

const B01 = 'XC-B01';
const B02 = 'XC-B02';
const VIEW = '/api/v3/views/admin_booking_slots';

interface SentWrite {
  method: string;
  path: string;
  body: unknown;
}

function makeViewData(branchId: string, slots: AdminSlotDto[]): AdminBookingSlotsData {
  return {
    rule: { branchId, slotMinutes: 30, capacity: 8, horizonDays: 21 },
    closures: [],
    slots,
    policy: { cancelCutoffMinutes: 45, noShowGraceMinutes: 20, confirmExpiryMinutes: 30 },
  };
}

describe('#2495: BookingSlots renders slot startAt and endAt as Bangkok wall-clock HH:mm times', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let writes: SentWrite[];
  let branchData: Record<string, AdminBookingSlotsData>;

  beforeEach(() => {
    vi.stubEnv('TZ', 'UTC');
    setLang('th');
    sharedSession().updateMe(meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }));
    container = document.createElement('div');
    document.body.appendChild(container);
    writes = [];
    branchData = {
      [B01]: makeViewData(B01, [
        {
          id: 'slot-0900',
          branchId: B01,
          serviceDate: '2026-10-17',
          startAt: '2026-10-17T02:00:00+00:00',
          endAt: '2026-10-17T02:30:00+00:00',
          capacity: 8,
          reserved: 2,
          status: 'OPEN',
        },
      ]),
      [B02]: makeViewData(B02, [
        {
          id: 'slot-b02-1330',
          branchId: B02,
          serviceDate: '2026-10-17',
          startAt: '2026-10-17T06:30:00+00:00',
          endAt: '2026-10-17T07:30:00+00:00',
          capacity: 12,
          reserved: 1,
          status: 'OPEN',
        },
      ]),
    };

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();

      if (method === 'GET' && u.pathname === VIEW) {
        const b = u.searchParams.get('branch') ?? B01;
        const data = branchData[b] ?? branchData[B01]!;
        return new Response(JSON.stringify({ version: 1, changed: true, data: structuredClone(data) }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
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
      writes.push({ method, path: u.pathname, body });
      return new Response(JSON.stringify({}), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(() => {
    setLang('th');
    dispose?.();
    dispose = undefined;
    container.remove();
    sharedSession().updateMe(undefined as unknown as Me);
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
  });

  async function settle(): Promise<void> {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }

  async function mount(initialBranch = B01): Promise<{ setBranch: (b: string) => void }> {
    const [branch, setBranch] = createSignal(initialBranch);
    const [period] = createSignal({ from: '2026-10-17', to: '2026-10-17' });
    const [shown] = createSignal(true);
    const props: AdminMenuProps = { branch, period, shown };
    dispose = render(() => <BookingSlots {...props} />, container);
    await settle();
    return { setBranch };
  }

  function slotItem(slotId: string): HTMLElement {
    const el = container.querySelector<HTMLElement>(`.slot-item[data-slot-id="${slotId}"]`);
    expect(el, `slot-item ${slotId} exists`).not.toBeNull();
    return el!;
  }

  it('row 1: slot startAt 2026-10-17T02:00:00+00:00, endAt 2026-10-17T02:30:00+00:00 shows 09:00 - 09:30 in Bangkok time and keeps serviceDate', async () => {
    await mount();

    const item = slotItem('slot-0900');
    const dateEl = item.querySelector('.slot-date');
    const timeEl = item.querySelector('.slot-time');

    expect(dateEl?.textContent?.trim()).toBe('2026-10-17');
    expect(timeEl?.textContent?.trim()).toBe('09:00 - 09:30');
    expect(timeEl?.textContent).not.toContain('02:00');
    expect(timeEl?.textContent).not.toContain('02:30');
    expect(timeEl?.textContent).not.toContain('+00:00');

    setLang('en');
    await settle();
    expect(timeEl?.textContent?.trim()).toBe('09:00 - 09:30');
  });

  it('row 2: slot startAt 2026-10-16T23:30:00+00:00 (06:30 Bangkok on the 17th) shows 06:30, not a time on the 16th', async () => {
    branchData[B01] = makeViewData(B01, [
      {
        id: 'slot-0630',
        branchId: B01,
        serviceDate: '2026-10-17',
        startAt: '2026-10-16T23:30:00+00:00',
        endAt: '2026-10-17T00:00:00+00:00',
        capacity: 8,
        reserved: 0,
        status: 'OPEN',
      },
    ]);

    await mount();

    const item = slotItem('slot-0630');
    const dateEl = item.querySelector('.slot-date');
    const timeEl = item.querySelector('.slot-time');

    expect(dateEl?.textContent?.trim()).toBe('2026-10-17');
    expect(timeEl?.textContent?.trim()).toBe('06:30 - 07:00');
    expect(timeEl?.textContent).not.toContain('2026-10-16');
    expect(timeEl?.textContent).not.toContain('23:30');
  });

  it('row 3: capacity, close, and open writes still send the slot id unchanged on a Bangkok-formatted slot row', async () => {
    await mount();

    const item = slotItem('slot-0900');
    expect(item.querySelector('.slot-time')?.textContent?.trim()).toBe('09:00 - 09:30');

    const capInput = item.querySelector<HTMLInputElement>('input[name="slotCapacity"]');
    expect(capInput).not.toBeNull();
    capInput!.value = '10';
    capInput!.dispatchEvent(new Event('input', { bubbles: true }));

    const findSlotButton = (label: string): HTMLButtonElement => {
      const btn = Array.from(slotItem('slot-0900').querySelectorAll('button')).find(
        (b) => b.textContent?.trim() === label,
      );
      expect(btn, `slot button ${label} exists`).toBeDefined();
      return btn!;
    };

    findSlotButton(table.th.updateCapacity).click();
    await settle();
    findSlotButton(table.th.close).click();
    await settle();
    findSlotButton(table.th.open).click();
    await settle();

    expect(writes).toEqual([
      {
        method: 'PUT',
        path: '/api/booking-slots/slot-0900/capacity',
        body: { capacity: 10, branch: B01 },
      },
      {
        method: 'POST',
        path: '/api/booking-slots/slot-0900/close',
        body: { branch: B01 },
      },
      {
        method: 'POST',
        path: '/api/booking-slots/slot-0900/open',
        body: { branch: B01 },
      },
    ]);
  });

  it('row 4: branch switch reads load and format the new branch slots in Bangkok time', async () => {
    const { setBranch } = await mount(B01);

    expect(slotItem('slot-0900').querySelector('.slot-time')?.textContent?.trim()).toBe('09:00 - 09:30');

    setBranch(B02);
    await settle();

    expect(container.querySelector('.slot-item[data-slot-id="slot-0900"]')).toBeNull();
    const b02Item = slotItem('slot-b02-1330');
    expect(b02Item.querySelector('.slot-date')?.textContent?.trim()).toBe('2026-10-17');
    expect(b02Item.querySelector('.slot-time')?.textContent?.trim()).toBe('13:30 - 14:30');
  });

  it('edge case: a missing or unparsable time shows the raw string rather than Invalid Date while valid times format as Bangkok HH:mm', async () => {
    branchData[B01] = makeViewData(B01, [
      {
        id: 'slot-bad-start',
        branchId: B01,
        serviceDate: '2026-10-17',
        startAt: 'not-a-date',
        endAt: '2026-10-17T02:30:00+00:00',
        capacity: 8,
        reserved: 0,
        status: 'OPEN',
      },
      {
        id: 'slot-empty-end',
        branchId: B01,
        serviceDate: '2026-10-17',
        startAt: '2026-10-17T02:00:00+00:00',
        endAt: '',
        capacity: 8,
        reserved: 0,
        status: 'OPEN',
      },
    ]);

    await mount();

    const badStartTime = slotItem('slot-bad-start').querySelector('.slot-time')?.textContent?.trim();
    expect(badStartTime).toBe('not-a-date - 09:30');
    expect(badStartTime).not.toContain('Invalid Date');

    const emptyEndTime = slotItem('slot-empty-end').querySelector('.slot-time')?.textContent?.trim();
    expect(emptyEndTime).toBe('09:00 -');
    expect(emptyEndTime).not.toContain('Invalid Date');
  });
});
