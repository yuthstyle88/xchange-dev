// #5425: Desk new-booking dialog Thai field labels and slot date prefix (§7, §8, §9)
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import {
  makeDeskBookingItemDto,
  makeDeskBookingsData,
  makeMe,
  makeMeRole,
} from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { th } from './i18n';

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const STAFF_TELLER = makeMe({
  id: '00000000-0000-4000-8000-000000000001',
  name: 'Somchai Teller',
  roles: [makeMeRole({ role: 'teller', branchId: 'XC-B01' })],
  branch: 'XC-B01',
  branchName: 'สาขาหลัก',
  permissions: ['BookingsDesk', 'RatesRead'],
});

const BOOKINGS_WITH_SLOTS = makeDeskBookingsData({
  items: [
    makeDeskBookingItemDto({
      id: 'bk-1',
      bookingNo: 'B261030-0001',
      status: 'CONFIRMED',
      customerDisplayName: 'Customer A',
      phoneLast4: '1234',
      readiness: 'ready',
      ticketNumber: null,
      slotId: 'slot-oct30',
      startAt: '2026-10-30T03:00:00Z',
      endAt: '2026-10-30T03:30:00Z',
    }),
    makeDeskBookingItemDto({
      id: 'bk-2',
      bookingNo: 'B261030-0002',
      status: 'CONFIRMED',
      customerDisplayName: 'Customer B',
      phoneLast4: '5678',
      readiness: 'ready',
      ticketNumber: null,
      slotId: 'slot-no-date',
      startAt: undefined,
      endAt: undefined,
    }),
  ],
});

function stubBookingsFetch() {
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const rawUrl = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(rawUrl, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();

    if (method === 'GET' && u.pathname === '/api/v3/views/me') {
      return jsonResponse(200, { version: 1, changed: true, data: STAFF_TELLER });
    }
    if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings') {
      return jsonResponse(200, { version: 1, changed: true, data: BOOKINGS_WITH_SLOTS });
    }
    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  vi.stubGlobal('fetch', fetchFn);
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('DeskBookingsScreen — New-booking dialog labels and slot date (#5425)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('staff');
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setAudience('staff');
    setLang('th');
  });

  async function openDialog(): Promise<void> {
    const btn = host.querySelector<HTMLButtonElement>('button[data-action="new-booking"]');
    expect(btn).not.toBeNull();
    btn!.click();
    await flush();
  }

  it('dialog open, th: shows the 8 Thai labels and no English labels (§8 row 1)', async () => {
    setLang('th');
    stubBookingsFetch();
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    await openDialog();

    const branchLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-branch"]');
    const slotLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-slot"]');
    const sideLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-side"]');
    const currencyLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-currency"]');
    const quantityLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-quantity"]');
    const displayNameLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-display-name"]');
    const phoneLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-phone"]');
    const referralLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-referral"]');

    expect(branchLabel?.textContent?.trim()).toBe('สาขา');
    expect(slotLabel?.textContent?.trim()).toBe('ช่วงเวลา');
    expect(sideLabel?.textContent?.trim()).toBe('ประเภทรายการ');
    expect(currencyLabel?.textContent?.trim()).toBe('สกุลเงิน');
    expect(quantityLabel?.textContent?.trim()).toBe('จำนวน');
    expect(displayNameLabel?.textContent?.trim()).toBe('ชื่อลูกค้า');
    expect(phoneLabel?.textContent?.trim()).toBe('เบอร์โทรลูกค้า');
    expect(referralLabel?.textContent?.trim()).toBe('รหัสแนะนำ');

    // Confirm keys in i18n match the required values
    expect(th['dialog.branch']).toBe('สาขา');
    expect(th['dialog.slot']).toBe('ช่วงเวลา');
    expect(th['dialog.side']).toBe('ประเภทรายการ');
    expect(th['dialog.currency']).toBe('สกุลเงิน');
    expect(th['dialog.quantity']).toBe('จำนวน');
    expect(th['dialog.displayName']).toBe('ชื่อลูกค้า');
    expect(th['dialog.phone']).toBe('เบอร์โทรลูกค้า');
    expect(th['dialog.referralCode']).toBe('รหัสแนะนำ');

    // Verify no hardcoded English labels are present in the new-booking dialog form
    const formLabels = Array.from(host.querySelectorAll('.xc-modal form label')).map((l) =>
      l.textContent?.trim(),
    );
    const englishLabels = [
      'Branch',
      'Slot',
      'Side',
      'Currency',
      'Quantity',
      'Customer Name',
      'Customer Phone',
      'Referral Code',
    ];
    for (const enLabel of englishLabels) {
      expect(formLabels).not.toContain(enLabel);
    }
  });

  it('slot startAt 2026-10-30T03:00:00...: option text starts with the Thai slot date (§8 row 2)', async () => {
    stubBookingsFetch();
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    await openDialog();

    const select = host.querySelector<HTMLSelectElement>('#new-booking-slot');
    expect(select).not.toBeNull();

    const slotOption = host.querySelector<HTMLOptionElement>(
      '#new-booking-slot option[value="slot-oct30"]',
    );
    expect(slotOption).not.toBeNull();
    const text = slotOption?.textContent?.trim() ?? '';
    expect(text.startsWith('30 ต.ค.')).toBe(true);
    expect(text).toBe('30 ต.ค. 10:00 - 10:30');
  });

  it('slot with no startAt keeps fallback s.slotId (§9 edge case)', async () => {
    stubBookingsFetch();
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    await openDialog();

    const slotOption = host.querySelector<HTMLOptionElement>(
      '#new-booking-slot option[value="slot-no-date"]',
    );
    expect(slotOption).not.toBeNull();
    expect(slotOption?.textContent?.trim()).toBe('slot-no-date');
  });

  it('en carries the English dialog labels (#5332)', async () => {
    setLang('en');
    stubBookingsFetch();
    dispose = render(() => <DeskBookingsScreen />, host);
    await flush();
    await openDialog();

    const branchLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-branch"]');
    const slotLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-slot"]');
    const sideLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-side"]');
    const currencyLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-currency"]');
    const quantityLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-quantity"]');
    const displayNameLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-display-name"]');
    const phoneLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-phone"]');
    const referralLabel = host.querySelector<HTMLLabelElement>('label[for="new-booking-referral"]');

    expect(branchLabel?.textContent?.trim()).toBe('Branch');
    expect(slotLabel?.textContent?.trim()).toBe('Slot');
    expect(sideLabel?.textContent?.trim()).toBe('Side');
    expect(currencyLabel?.textContent?.trim()).toBe('Currency');
    expect(quantityLabel?.textContent?.trim()).toBe('Quantity');
    expect(displayNameLabel?.textContent?.trim()).toBe('Customer Name');
    expect(phoneLabel?.textContent?.trim()).toBe('Customer Phone');
    expect(referralLabel?.textContent?.trim()).toBe('Referral Code');
  });
});
