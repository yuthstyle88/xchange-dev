// OL-6 B admin-booking-slots (audit #564 issuecomment-6702970101, shot 047): every 30-minute slot of the horizon was
// one flat list (~50,000 px). Slots are grouped by serviceDate, one <details> per day, the first day open.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import {
  type AdminBookingSlotsData,
  type AdminSlotDto,
  makeAdminBookingPolicyDto,
  makeAdminBookingSlotsData,
  makeAdminSlotDto,
  makeAdminSlotRuleDto,
} from '../../../shared/testing/fixtures';
import { meOf } from '../../../test-support/me';
import { BookingSlots } from './BookingSlots';
import * as table from './i18n';
import { groupSlotsByDay } from './slot-days';

const B01 = 'XC-B01';
const B02 = 'XC-B02';
const VIEW = '/api/v3/views/admin_booking_slots';

const slot = (id: string, date: string, status = 'OPEN', branchId = B01): AdminSlotDto =>
  makeAdminSlotDto({
    id,
    branchId,
    serviceDate: date,
    startAt: `${date}T02:00:00+00:00`,
    endAt: `${date}T02:30:00+00:00`,
    capacity: 5,
    reserved: 0,
    status,
  });

const viewOf = (branchId: string, slots: AdminSlotDto[]): AdminBookingSlotsData =>
  makeAdminBookingSlotsData({
    rule: makeAdminSlotRuleDto({ branchId, slotMinutes: 30, capacity: 5, horizonDays: 14 }),
    closures: [],
    slots,
    policy: makeAdminBookingPolicyDto({ cancelCutoffMinutes: 60, noShowGraceMinutes: 15, confirmExpiryMinutes: 30 }),
  });

let container: HTMLDivElement;
let dispose: (() => void) | undefined;
let data: Record<string, AdminBookingSlotsData>;

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(): Promise<(b: string) => void> {
  const [branch, setBranch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-11-07', to: '2026-11-07' });
  const [shown] = createSignal(true);
  dispose = render(() => <BookingSlots branch={branch} period={period} shown={shown} />, container);
  await settle();
  return setBranch;
}

const days = () => [...container.querySelectorAll<HTMLDetailsElement>('details.bs-day')];
const openDays = () => days().filter((d) => d.open).map((d) => d.dataset.day);

function toggle(d: HTMLDetailsElement, open: boolean): void {
  d.open = open;
  d.dispatchEvent(new Event('toggle'));
}

beforeEach(() => {
  vi.stubEnv('TZ', 'UTC');
  setLang('th');
  sharedSession().updateMe(meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }));
  container = document.createElement('div');
  document.body.appendChild(container);
  data = {
    [B01]: viewOf(B01, [
      slot('a1', '2026-11-07'),
      slot('a2', '2026-11-07', 'CLOSED'),
      slot('b1', '2026-11-08'),
      slot('c1', '2026-11-09', 'HELD'),
    ]),
    [B02]: viewOf(B02, [slot('z1', '2026-11-10', 'OPEN', B02), slot('z2', '2026-11-11', 'OPEN', B02)]),
  };
  vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(raw, 'http://localhost');
    if ((init?.method ?? 'GET').toUpperCase() === 'GET' && u.pathname === VIEW) {
      const d = data[u.searchParams.get('branch') ?? B01]!;
      return new Response(JSON.stringify({ version: 1, changed: true, data: structuredClone(d) }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }
    return new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } });
  });
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  container.remove();
  sharedSession().updateMe(undefined as never);
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
});

describe('groupSlotsByDay', () => {
  it('BD-1 one group per serviceDate, first-seen order, slots in server order, OPEN counted', () => {
    const g = groupSlotsByDay([slot('x', '2026-11-08'), slot('y', '2026-11-07', 'CLOSED'), slot('z', '2026-11-08')]);
    expect(g.map((d) => d.date)).toEqual(['2026-11-08', '2026-11-07']);
    expect(g[0]!.slots.map((s) => s.id)).toEqual(['x', 'z']);
    expect(g.map((d) => d.open)).toEqual([2, 0]);
    expect(groupSlotsByDay([])).toEqual([]);
  });
});

describe('BookingSlots day groups', () => {
  it('BD-2 the count text is in both tables', () => {
    expect(table.th.slotsDayCount).toBe('เปิด {open} จาก {total} ช่วง');
    expect(table.en.slotsDayCount).toBe('{open} of {total} slots open');
  });

  it('BD-3 one <details> per day; only the first is open; every slot row is still in the DOM', async () => {
    await mount();
    expect(days().map((d) => d.dataset.day)).toEqual(['2026-11-07', '2026-11-08', '2026-11-09']);
    expect(openDays()).toEqual(['2026-11-07']);
    expect(container.querySelectorAll('.slot-item')).toHaveLength(4);
    expect(days()[1]!.querySelector('.slot-item[data-slot-id="b1"]')).not.toBeNull();
  });

  it('BD-4 the summary shows the date and "open of total"', async () => {
    await mount();
    const s = days()[0]!.querySelector('summary')!;
    expect(s.querySelector('.bs-day-date')?.textContent).toBe('2026-11-07');
    expect(s.querySelector('.bs-day-count')?.textContent).toBe('เปิด 1 จาก 2 ช่วง');
  });

  it('BD-5 each slot row carries its status for the pill style', async () => {
    await mount();
    expect(container.querySelector('.slot-item[data-slot-id="a2"] .slot-status')?.getAttribute('data-status')).toBe('CLOSED');
  });

  it('BD-6 a day the user opened stays open, and a closed first day stays closed, across a reload after a write', async () => {
    await mount();
    toggle(days()[1]!, true);
    toggle(days()[0]!, false);
    await settle();
    expect(openDays()).toEqual(['2026-11-08']);
    const close = [...container.querySelectorAll<HTMLButtonElement>('.slot-item[data-slot-id="b1"] button')].find(
      (b) => b.textContent?.trim() === table.th.close,
    )!;
    close.click();
    await settle();
    expect(openDays()).toEqual(['2026-11-08']);
  });

  it('BD-7 a branch switch goes back to "first day open"', async () => {
    const setBranch = await mount();
    toggle(days()[2]!, true);
    await settle();
    setBranch(B02);
    await settle();
    expect(days().map((d) => d.dataset.day)).toEqual(['2026-11-10', '2026-11-11']);
    expect(openDays()).toEqual(['2026-11-10']);
  });

  it('BD-8 no slots: the empty text, no day groups', async () => {
    data[B01] = viewOf(B01, []);
    await mount();
    expect(days()).toHaveLength(0);
    expect(container.querySelector('.bs-days .bs-empty')?.textContent).toContain(table.th.noSlots);
  });
});
