// OL-7 admin-booking-slots (verifier row 92, VERIFY.md: "still raw OPEN/CLOSED badges, both ปิดรับจอง and เปิดรับจอง on every
// row, and the button still reads Save กฎการจอง"). A slot's status is OPEN or CLOSED only
// (api/migrations/20261205131000_booking_slots.sql:65 CHECK). Each row shows its status in words (statusOpen / statusClosed)
// and offers only the action that status allows: an OPEN slot can be closed, a CLOSED slot can be opened. The Thai labels
// "Save" and "Save กฎการจอง" become Thai. Written before the implementation by claude/94389a6b for the sub-controller
// claude/1cc51615 (with it, BookingSlots.test.tsx's close/open request test and branch-switch.test.tsx's two OPEN checks were
// moved to an OPEN + a CLOSED slot and to data-status). Do not edit, delete or weaken; add tests in a file of your own.
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { BookingSlots } from './BookingSlots';
import * as table from './i18n';
import {
  makeAdminBookingPolicyDto,
  makeAdminBookingSlotsData,
  makeAdminSlotDto,
  makeAdminSlotRuleDto,
} from '../../../shared/testing/fixtures';

const B01 = 'XC-B01';

const OPEN_SLOT = makeAdminSlotDto({
  id: 'slot-open',
  branchId: B01,
  serviceDate: '2026-10-15',
  startAt: '2026-10-15T09:00:00Z',
  endAt: '2026-10-15T09:30:00Z',
  capacity: 8,
  reserved: 2,
  status: 'OPEN',
});

const CLOSED_SLOT = makeAdminSlotDto({
  ...OPEN_SLOT,
  id: 'slot-closed',
  startAt: '2026-10-15T09:30:00Z',
  endAt: '2026-10-15T10:00:00Z',
  status: 'CLOSED',
});

const VIEW = makeAdminBookingSlotsData({
  rule: makeAdminSlotRuleDto({ branchId: B01, slotMinutes: 30, capacity: 8, horizonDays: 21 }),
  closures: [],
  slots: [OPEN_SLOT, CLOSED_SLOT],
  policy: makeAdminBookingPolicyDto({ cancelCutoffMinutes: 45, noShowGraceMinutes: 20, confirmExpiryMinutes: 30 }),
});

function props(): AdminMenuProps {
  const [branch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
  const [shown] = createSignal(true);
  return { branch, period, shown };
}

let container: HTMLDivElement;
let dispose: (() => void) | null = null;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(url, 'http://localhost');
    if (u.pathname === '/api/v3/views/admin_booking_slots') {
      return new Response(JSON.stringify({ version: 1, changed: true, data: VIEW }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }
    return new Response(JSON.stringify({}), { status: 200, headers: { 'content-type': 'application/json' } });
  });
});

afterEach(() => {
  setLang('th');
  dispose?.();
  dispose = null;
  container.remove();
  vi.restoreAllMocks();
});

async function mount(lang: 'th' | 'en'): Promise<void> {
  setLang(lang);
  dispose = render(() => <BookingSlots {...props()} />, container);
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
}

function row(id: string): HTMLElement {
  const el = container.querySelector<HTMLElement>(`.slot-item[data-slot-id="${id}"]`);
  expect(el, `row ${id}`).not.toBeNull();
  return el!;
}

function buttonTexts(el: HTMLElement): string[] {
  return [...el.querySelectorAll('button')].map((b) => (b.textContent ?? '').trim());
}

describe('booking slot rows', () => {
  it('BS-1 Thai: the status is in words, and data-status keeps the server value', async () => {
    await mount('th');
    const open = row('slot-open').querySelector('.slot-status')!;
    const closed = row('slot-closed').querySelector('.slot-status')!;
    expect(open.textContent?.trim()).toBe(table.th.statusOpen);
    expect(closed.textContent?.trim()).toBe(table.th.statusClosed);
    expect(open.getAttribute('data-status')).toBe('OPEN');
    expect(closed.getAttribute('data-status')).toBe('CLOSED');
  });

  it('BS-2 the Thai and English words', () => {
    expect(table.th.statusOpen).toBe('เปิดรับ');
    expect(table.th.statusClosed).toBe('ปิดรับ');
    expect(table.en.statusOpen).toBe('Open');
    expect(table.en.statusClosed).toBe('Closed');
  });

  it('BS-3 English: the status is in English words', async () => {
    await mount('en');
    expect(row('slot-open').querySelector('.slot-status')?.textContent?.trim()).toBe('Open');
    expect(row('slot-closed').querySelector('.slot-status')?.textContent?.trim()).toBe('Closed');
  });

  it('BS-4 an OPEN slot offers close only; a CLOSED slot offers open only', async () => {
    await mount('th');
    const open = buttonTexts(row('slot-open'));
    const closed = buttonTexts(row('slot-closed'));
    expect(open).toContain(table.th.close);
    expect(open).not.toContain(table.th.open);
    expect(closed).toContain(table.th.open);
    expect(closed).not.toContain(table.th.close);
  });

  it('BS-5 unchanged: every row keeps its capacity field and its update button', async () => {
    await mount('th');
    for (const id of ['slot-open', 'slot-closed']) {
      expect(row(id).querySelector('input[name="slotCapacity"]'), `${id} capacity`).not.toBeNull();
      expect(buttonTexts(row(id))).toContain(table.th.updateCapacity);
    }
  });

  it('BS-6 the Thai save labels are Thai', () => {
    expect(table.th.saveRule).toBe('บันทึกกฎการจอง');
    expect(table.th.save).toBe('บันทึก');
    expect(Object.values(table.th).filter((v) => /^Save\b/.test(v))).toEqual([]);
  });
});
