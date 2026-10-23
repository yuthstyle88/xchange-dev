// #4919: Admin reconciliation, payments, devices, kiosk settings and booking slots input bounds (A1–A6).
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ApiClient, PageParams, ViewParams } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { sharedSession } from '../../shared/session/session';
import {
  makeAdminBookingPolicyDto,
  makeAdminBookingSlotsData,
  makeAdminClosureDto,
  makeAdminSlotDto,
  makeAdminSlotRuleDto,
  makeDeviceBranch,
  makeDevicesOverview,
  makeKbankReference,
  makeKioskCurrencies,
  makePaymentMethodItem,
  makeRateFreshnessDto,
  makeViewAnswer,
} from '../../shared/testing/fixtures';
import { meOf } from '../../test-support/me';
import { DevicesScreen } from '../devices/DevicesScreen';
import { th as devicesTh } from '../devices/i18n';
import { KioskCurrenciesScreen } from '../kiosk-currencies/KioskCurrenciesScreen';
import { BookingSlots } from './booking-slots/BookingSlots';
import * as bookingSlotsTable from './booking-slots/i18n';
import { Payments } from './payments/Payments';
import { th as paymentsTh } from './payments/i18n';
import { Reconciliation } from './reconciliation/Reconciliation';
import * as reconTable from './reconciliation/i18n';

const B01 = 'XC-B01';

async function settle(): Promise<void> {
  for (let i = 0; i < 12; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('Admin and device input bounds (#4919 §3 A1–A6)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  it('A1: Reconciliation with period 2026-10-01..2026-10-05 names the day in the sheet header and disables Certify with the "pick one day" line', async () => {
    const certifies: unknown[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const u = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (u.pathname === '/api/v3/views/finance_reconciliation' && method === 'GET') {
          const branch = u.searchParams.get('branch') ?? B01;
          const date = u.searchParams.get('date') ?? '2026-10-05';
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: {
                branch,
                date,
                transactions: { available: false, reason: 'TX', data: null },
                counts: { available: false, reason: 'CA-3', data: null },
                transfers: { available: false, reason: 'CA-5', data: null },
                exceptions: { available: false, reason: 'EX', data: null },
                integrity: { available: false, reason: 'LG-7', data: null },
                reversalsPending: { available: false, reason: 'RV-1', data: null },
                figuresHash: 'hash-1',
                certifications: [],
              },
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/v3/commands/reconciliation_certify' && method === 'POST') {
          certifies.push(init?.body);
        }
        return new Response(JSON.stringify({ version: 1, changed: false }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }),
    );

    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-01', to: '2026-10-05' });
    const [shown] = createSignal(true);
    dispose = render(() => <Reconciliation branch={branch} period={period} shown={shown} />, host);
    await settle();

    const header = host.querySelector('header');
    expect(header, 'sheet header exists').not.toBeNull();
    expect(header!.textContent).toContain('2026-10-05');
    expect(header!.textContent).toContain(reconTable.th.pickOneDay);

    const certifyBtn = [...host.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === reconTable.th.certify,
    );
    expect(certifyBtn, 'Certify button exists').toBeDefined();
    expect(certifyBtn!.disabled).toBe(true);
    certifyBtn!.click();
    await settle();
    expect(certifies).toEqual([]);
  });

  it('A2: Payments: enable a channel with neither buy nor sell, click Save -> worded line on that channel card; POST /api/payment-methods is not called', async () => {
    const posts: unknown[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const u = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (u.pathname === '/api/v3/views/admin_branches' && method === 'GET') {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: { branches: [{ id: B01, name: 'สาขาหลัก' }] },
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/v3/views/admin_payments' && method === 'GET') {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: {
                branchId: B01,
                canEdit: true,
                settingsVersion: 1,
                updatedAt: null,
                updatedBy: null,
                methods: [
                  makePaymentMethodItem({ id: 'Cash', enabled: true, buy: true, sell: true, account: '', instruction: '' }),
                  makePaymentMethodItem({ id: 'Credit Card', enabled: false, buy: false, sell: true, account: '', instruction: '' }),
                  makePaymentMethodItem({ id: 'Alipay', enabled: false, buy: false, sell: true, account: '', instruction: '' }),
                  makePaymentMethodItem({ id: 'PromptPay', enabled: false, buy: false, sell: true, account: '', instruction: '' }),
                  makePaymentMethodItem({ id: 'Bank Transfer', enabled: true, buy: true, sell: true, account: '', instruction: '' }),
                ],
                history: [],
              },
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        if (u.pathname === '/api/payment-methods' && method === 'POST') {
          posts.push(init?.body);
        }
        return new Response(JSON.stringify({ saved: true }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }),
    );

    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown] = createSignal(true);
    dispose = render(() => <Payments branch={branch} period={period} shown={shown} />, host);
    await settle();

    const cashCard = host.querySelector<HTMLElement>('[data-payment-id="Cash"]')!;
    expect(cashCard).not.toBeNull();
    const checkboxes = [...cashCard.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')];
    // Untick sell and buy on Cash while leaving Cash enabled
    for (const cb of checkboxes.slice(1)) {
      if (cb.checked && !cb.disabled) {
        cb.click();
      }
    }
    await settle();

    const saveBtn = [...host.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === paymentsTh.save,
    )!;
    saveBtn.click();
    await settle();

    expect(posts).toEqual([]);
    expect(cashCard.textContent).toContain(paymentsTh.channelDirectionRequired);
  });

  it('A3: Devices: no transaction picked; blank drawer reason -> Print is disabled with a title; Drawer shows a worded line at the reason field and sends nothing', async () => {
    const posts: unknown[] = [];
    const api: Pick<ApiClient, 'view' | 'post' | 'page'> = {
      async view(name: string) {
        if (name === 'devices') {
          return {
            version: 1,
            changed: true,
            data: makeDevicesOverview({
              branchId: B01,
              branches: [makeDeviceBranch({ id: B01, name: 'สาขาหลัก' })],
              printerOnline: true,
              printerKind: 'SIMULATED',
              printerModel: 'NEOCAL NTP-8220BT',
              hasDrawer: null,
              drawerAgentActive: false,
              balanceThb: '1000.00',
              balanceUsd: '100.00',
              completedCount: 1,
            }) as any,
          };
        }
        return { version: 1, changed: true, data: [] as any };
      },
      async page(_name: string, _q?: PageParams) {
        return [{ id: 'TX-1', type: 'BUY', amount: '100.00', currency: 'USD' }] as any;
      },
      async post(path: string, body: unknown) {
        posts.push({ path, body });
        return { saved: true } as any;
      },
    };
    vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('offline'))));
    vi.spyOn(window.history, 'replaceState').mockImplementation(() => {});

    dispose = render(() => <DevicesScreen api={api} branch={B01} />, host);
    await settle();

    const printBtn = [...host.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === devicesTh.print,
    )!;
    const drawerBtn = [...host.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === devicesTh.drawer,
    )!;

    expect(printBtn.disabled).toBe(true);
    expect(printBtn.title).toBe(devicesTh.printTxRequired);

    const reasonInput = host.querySelector<HTMLInputElement>('.reason-group input[type="text"]')!;
    reasonInput.value = '   ';
    reasonInput.dispatchEvent(new Event('input', { bubbles: true }));
    drawerBtn.click();
    await settle();

    const reasonGroup = host.querySelector('.reason-group')!;
    expect(reasonGroup.querySelector('[role="alert"]')?.textContent).toBe(devicesTh.reasonRequired);
    expect(posts).toEqual([]);
  });

  it('A4: Kiosk result seconds: type "0x10", save -> a worded line naming 3–120; POST /api/kiosk is not called', async () => {
    const saves: unknown[] = [];
    const api: Pick<ApiClient, 'view' | 'post'> = {
      view(_name: string, _q?: ViewParams) {
        return Promise.resolve(
          makeViewAnswer(
            makeKioskCurrencies({
              currencies: [],
              kbank: makeKbankReference({ source: 'kbank', publishedAt: '2026-10-06T05:00:00Z', round: 1 }),
              freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
            }),
            { version: 1, changed: true },
          ) as any,
        );
      },
      async post(_path: string, body: unknown) {
        const b = body as { action: string };
        if (b.action === 'settings') {
          return { resultSeconds: 10, version: 1 } as any;
        }
        saves.push(body);
        return { resultSeconds: 10, version: 2 } as any;
      },
    };

    dispose = render(() => <KioskCurrenciesScreen api={api} branch="B9" />, host);
    await settle();

    host.querySelector<HTMLButtonElement>('button[name="load_result_seconds"]')!.click();
    await settle();

    const input = host.querySelector<HTMLInputElement>('input#result_seconds')!;
    const saveBtn = host.querySelector<HTMLButtonElement>('button[name="save_result_seconds"]')!;

    input.value = '0x10';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();
    await settle();

    const alert = host.querySelector('[role="alert"]');
    expect(alert, 'alert shown for "0x10"').not.toBeNull();
    expect(alert!.textContent).toContain('3–120');
    expect(saves).toEqual([]);
  });

  it('A5: Kiosk result seconds: type "45", save -> saved as today', async () => {
    const saves: unknown[] = [];
    const api: Pick<ApiClient, 'view' | 'post'> = {
      view(_name: string, _q?: ViewParams) {
        return Promise.resolve(
          makeViewAnswer(
            makeKioskCurrencies({
              currencies: [],
              kbank: makeKbankReference({ source: 'kbank', publishedAt: '2026-10-06T05:00:00Z', round: 1 }),
              freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
            }),
            { version: 1, changed: true },
          ) as any,
        );
      },
      async post(_path: string, body: unknown) {
        const b = body as { action: string };
        if (b.action === 'settings') {
          return { resultSeconds: 10, version: 1 } as any;
        }
        saves.push(body);
        return { resultSeconds: 45, version: 2 } as any;
      },
    };

    dispose = render(() => <KioskCurrenciesScreen api={api} branch="B9" />, host);
    await settle();

    host.querySelector<HTMLButtonElement>('button[name="load_result_seconds"]')!.click();
    await settle();

    const input = host.querySelector<HTMLInputElement>('input#result_seconds')!;
    const saveBtn = host.querySelector<HTMLButtonElement>('button[name="save_result_seconds"]')!;

    input.value = '45';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();
    await settle();

    expect(host.querySelector('[role="alert"]')).toBeNull();
    expect(saves).toEqual([
      {
        action: 'configureSettings',
        payload: { branchId: 'B9', resultSeconds: 45 },
      },
    ]);
  });

  it('A6: Booking slots: a server 400 refusal with field "horizonDays" is shown at horizon days, not only in the top banner', async () => {
    sharedSession().updateMe(
      meOf({ id: 'admin-1', role: 'ADMIN', branch: B01, name: 'Admin', orgWide: true }),
    );
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit) => {
        const u = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (u.pathname === '/api/v3/views/admin_booking_slots' && method === 'GET') {
          return new Response(
            JSON.stringify({
              version: 1,
              changed: true,
              data: makeAdminBookingSlotsData({
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
                    reason: 'Holiday',
                  }),
                ],
                slots: [
                  makeAdminSlotDto({
                    id: 'slot-1',
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
              }),
            }),
            { status: 200, headers: { 'content-type': 'application/json' } },
          );
        }
        return new Response(
          JSON.stringify({
            error: {
              code: 'validationFailed',
              field: 'horizonDays',
              message: 'horizonDays invalid',
            },
          }),
          { status: 400, headers: { 'content-type': 'application/json' } },
        );
      }),
    );

    const [branch] = createSignal(B01);
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
    const [shown] = createSignal(true);
    dispose = render(() => <BookingSlots branch={branch} period={period} shown={shown} />, host);
    await settle();

    const saveRuleBtn = [...host.querySelectorAll('button')].find(
      (b) => b.textContent?.trim() === bookingSlotsTable.th.saveRule,
    )!;
    saveRuleBtn.click();
    await settle();

    const horizonGroup = host.querySelector('input[name="horizonDays"]')?.closest('.form-group');
    expect(horizonGroup?.querySelector('[role="alert"]')?.textContent).toBe(
      bookingSlotsTable.th['error.validationFailed'],
    );
    expect(host.querySelector('.booking-slots-screen > [role="alert"]')).toBeNull();
  });
});
