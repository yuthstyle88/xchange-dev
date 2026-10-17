// Issue #2314: Admin Overview — after a period change, clear previous period figures
// so cards, activity and flow show — while loading and if the read fails.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { TransactionRow } from '../../../shared/api/generated/transaction_row';
import { errorText, setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { meOf } from '../../../test-support/me';
import { closeDialog, Dialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { Overview } from './Overview';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

const EXPORTER_ME = (() => {
  const m = meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner' }) as unknown as {
    permissions: string[];
    [key: string]: unknown;
  };
  return { ...m, permissions: [...(m.permissions ?? []), 'ReportsExport'].sort() };
})();

const VIEW_OVERVIEW = '/api/v3/views/admin_overview';
const VIEW_TRANSACTIONS = '/api/v3/views/admin_transactions';
const VIEW_BRANCHES = '/api/v3/views/admin_branches';

function row(id: string): TransactionRow {
  return {
    id,
    date: '2026-10-06',
    created: '2026-10-06T03:00:00.000Z',
    type: 'BUY',
    currency: 'USD',
    amount: '100.00',
    rate: '30.0000',
    unitSize: '1',
    principal: '3000.00',
    fee: '0.00',
    bankFee: '0.00',
    settlement: '3000.00',
    method: 'Cash',
    customer: 'Customer',
    customerId: 'CUS-1',
    staff: 'owner',
    status: 'COMPLETED',
    notes: null,
    cancelReason: null,
    shiftId: 'S-1',
    referenceRate: null,
    referenceValue: null,
    referenceSpread: null,
    referencePercent: null,
    paymentRef: null,
    slipUrl: null,
  };
}

const LOADED_DATA = {
  period: { from: '2026-10-05', to: '2026-10-06' },
  totals: {
    volume: '4400.00',
    buyPrincipal: '3300.00',
    sellPrincipal: '1100.00',
    buyCount: 2,
    sellCount: 1,
    buyShare: '75',
    sellShare: '25',
    received: '1120.00',
    paid: '3250.00',
    net: '-2130.00',
    fees: '70.00',
    completedCount: 3,
    cancelledCount: 4,
    pendingCount: 5,
  },
  latest: [row('XC-001')],
  status: { shiftOpen: true, pendingCount: 6, openedBy: 'Somchai' },
};

type HandlerReply = { status: number; body: unknown } | Promise<Response>;
let handlers: Record<string, (params: URLSearchParams) => HandlerReply>;
let overviewCalls: URLSearchParams[];
let dispose: (() => void) | undefined;
let root: HTMLElement;

function ok(data: unknown): { status: number; body: unknown } {
  return { status: 200, body: { version: 7, changed: true, data } };
}

function fakeFetch(input: string): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  if (url.pathname === VIEW_OVERVIEW) {
    overviewCalls.push(url.searchParams);
  }
  const handler = handlers[url.pathname];
  if (!handler) {
    return Promise.resolve(
      new Response(JSON.stringify({ version: 1, changed: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );
  }
  const res = handler(url.searchParams);
  if (res instanceof Promise) {
    return res;
  }
  return Promise.resolve(
    new Response(JSON.stringify(res.body), {
      status: res.status,
      headers: { 'content-type': 'application/json' },
    }),
  );
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 5; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

function mount(
  init: { branch?: string; period?: { from: string; to: string }; shown?: boolean } = {},
) {
  const [branch, setBranch] = createSignal(init.branch ?? 'MAIN');
  const [period, setPeriod] = createSignal(init.period ?? { from: '2026-10-05', to: '2026-10-06' });
  const [shown, setShown] = createSignal(init.shown ?? true);

  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Overview branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  return { el: root, setBranch, setPeriod, setShown };
}

function text(el: Element): string {
  return (el.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function cards(el: HTMLElement): HTMLElement[] {
  return [...el.querySelectorAll<HTMLElement>('.stat-card')];
}

function card(c: HTMLElement): { label: string; value: string; unit: string; note: string } {
  return {
    label: text(c.querySelector('.stat-label')!),
    value: text(c.querySelector('.stat-value .value')!),
    unit: text(c.querySelector('.stat-value .unit')!),
    note: text(c.querySelector('.stat-note')!),
  };
}

describe('Admin Overview period change figures (#2314)', () => {
  beforeEach(() => {
    overviewCalls = [];
    handlers = {
      [VIEW_OVERVIEW]: () => ok(LOADED_DATA),
      [VIEW_BRANCHES]: () => ok({ branches: [{ id: 'MAIN', name: 'Main Branch' }] }),
      [VIEW_TRANSACTIONS]: () => ok([row('XC-001')]),
      '/api/v3/views/me': () => ok(EXPORTER_ME),
      [QUEUE_CARD_PATH]: () => ({ status: 200, body: queueCardUnchanged() }),
    };
    sharedSession().updateMe?.(EXPORTER_ME as any);
    vi.stubGlobal('fetch', vi.fn(fakeFetch));
    setLang('th');
  });

  afterEach(async () => {
    closeDialog();
    dispose?.();
    dispose = undefined;
    root?.remove();
    vi.unstubAllGlobals();
    setLang('th');
    await settle();
  });

  it('row 1: Period change, before the answer -> every figure shows — (while an invalid period sends no read and keeps screen)', async () => {
    const m = mount({ branch: 'MAIN', period: { from: '2026-10-05', to: '2026-10-06' } });
    await settle();
    expect(overviewCalls.length).toBe(1);
    expect(cards(m.el).map((c) => card(c).value)).toEqual([
      formatMoney('4400.00'),
      formatMoney('3300.00'),
      formatMoney('1100.00'),
      '3',
    ]);

    // Edge case (§9): an invalid period (from after to) sends no read and keeps today's screen
    m.setPeriod({ from: '2026-10-07', to: '2026-10-06' });
    await settle();
    expect(overviewCalls.length).toBe(1);
    expect(cards(m.el).map((c) => card(c).value)).toEqual([
      formatMoney('4400.00'),
      formatMoney('3300.00'),
      formatMoney('1100.00'),
      '3',
    ]);

    // Valid period change: before the answer arrives, every figure shows —
    let resolveNext!: (r: Response) => void;
    handlers[VIEW_OVERVIEW] = () =>
      new Promise<Response>((resolve) => {
        resolveNext = resolve;
      });

    m.setPeriod({ from: '2026-10-01', to: '2026-10-04' });
    await settle();
    expect(overviewCalls.length).toBe(2);

    const shownCards = cards(m.el).map(card);
    expect(shownCards.map((c) => c.value)).toEqual(['—', '—', '—', '—']);
    expect(shownCards[1]!.note).toContain('— รายการ');
    expect(shownCards[2]!.note).toContain('— รายการ');
    expect(shownCards[3]!.note).toContain('ยกเลิก — · ค้าง — รายการ');

    const activity = m.el.querySelector('#activity')!;
    expect(text(activity)).toContain('/ — รายการ');
    expect([...activity.querySelectorAll('strong')].map(text)).toEqual(['—', '—']);

    expect(text(m.el.querySelector('.flow-value')!)).toBe('— THB');
    expect([...m.el.querySelectorAll('.flow-row')].map(text)).toEqual(['รับเข้า—', 'จ่ายออก—']);
    expect(text(m.el)).not.toContain('4,400.00');

    resolveNext(
      new Response(JSON.stringify(ok(LOADED_DATA).body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );
    await settle();
  });

  it('row 2: Period change, read fails -> — and the error banner', async () => {
    const m = mount({ branch: 'MAIN', period: { from: '2026-10-05', to: '2026-10-06' } });
    await settle();
    expect(cards(m.el).map((c) => card(c).value)).toEqual([
      formatMoney('4400.00'),
      formatMoney('3300.00'),
      formatMoney('1100.00'),
      '3',
    ]);

    handlers[VIEW_OVERVIEW] = () => ({
      status: 500,
      body: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } },
    });

    m.setPeriod({ from: '2026-10-01', to: '2026-10-04' });
    await settle();

    const banner = m.el.querySelector('.error-banner');
    expect(banner).not.toBeNull();
    expect(banner?.textContent).toBe(errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง'));

    const shownCards = cards(m.el).map(card);
    expect(shownCards.map((c) => c.value)).toEqual(['—', '—', '—', '—']);
    expect(shownCards[1]!.note).toContain('— รายการ');
    expect(shownCards[2]!.note).toContain('— รายการ');
    expect(shownCards[3]!.note).toContain('ยกเลิก — · ค้าง — รายการ');

    const activity = m.el.querySelector('#activity')!;
    expect(text(activity)).toContain('/ — รายการ');
    expect([...activity.querySelectorAll('strong')].map(text)).toEqual(['—', '—']);

    expect(text(m.el.querySelector('.flow-value')!)).toBe('— THB');
    expect([...m.el.querySelectorAll('.flow-row')].map(text)).toEqual(['รับเข้า—', 'จ่ายออก—']);
    expect(text(m.el)).not.toContain('4,400.00');
  });
});
