// CA-10p-FE (#2914): the status strip's "opened by" line reads status.openShifts (design
// docs/design/blueprint/cash-shift-transfer.md §4.7, CA-10p #2916). openedBy is gone.
// Up-front tests written by the ticket's writer (claude/177aa7ef). The implementer makes them pass and must not
// edit, delete or weaken them. Row 1 ("Decision 2", one teller, th and en) lives in Overview.extra.test.tsx.
// Harness copied from Overview.nodata.test.tsx.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { makeOpenShiftDto, makeTransactionRow } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { sharedSession } from '../../../shared/session/session';
import { meOf } from '../../../test-support/me';
import { closeDialog, Dialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { Overview } from './Overview';
import * as table from './i18n';
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

function row(id: string) {
  return makeTransactionRow({
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
  });
}

type HandlerReply = { status: number; body: unknown } | Promise<Response>;
let handlers: Record<string, (params: URLSearchParams) => HandlerReply>;
let dispose: (() => void) | undefined;
let root: HTMLElement;

function ok(data: unknown): { status: number; body: unknown } {
  return { status: 200, body: { version: 7, changed: true, data } };
}

function fakeFetch(input: string): Promise<Response> {
  const url = new URL(input, 'http://localhost');
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


function shift(tellerName: string, openedAt: string, n: number) {
  return makeOpenShiftDto({ shiftId: `SH-${n}`, tellerId: `U-${n}`, tellerName, openedAt });
}

function answer(status: Record<string, unknown>) {
  return {
    period: { from: '2026-10-05', to: '2026-10-06' },
    totals: {
      volume: '0.00',
      buyPrincipal: '0.00',
      sellPrincipal: '0.00',
      buyCount: 0,
      sellCount: 0,
      buyShare: '0',
      sellShare: '0',
      received: '0.00',
      paid: '0.00',
      net: '0.00',
      fees: '0.00',
      completedCount: 0,
      cancelledCount: 0,
      pendingCount: 0,
    },
    latest: [],
    status,
  };
}

function strip(el: HTMLElement): string {
  const s = el.querySelector('.status-strip');
  expect(s, 'the status strip renders').not.toBeNull();
  return text(s!);
}

const OPENED_BY = table.th.openedByLabel.replace(' {staff}', '');

describe('CA-10p-FE (#2914 §9): the opened-by line lists each open shift from openShifts', () => {
  beforeEach(() => {
    handlers = {
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

  it('row 2: several open shifts show every teller name, joined by ", ", in the order the server sends them (by openedAt)', async () => {
    handlers[VIEW_OVERVIEW] = () =>
      ok(
        answer({
          shiftOpen: true,
          pendingCount: 0,
          openShifts: [shift('Somchai', '2026-10-06T01:00:00Z', 1), shift('Anan', '2026-10-06T02:30:00Z', 2)],
        }),
      );
    const m = mount();
    await settle();
    expect(strip(m.el)).toContain(`${OPENED_BY} Somchai, Anan`);
  });

  it('row 3: no shift open (shiftOpen false, openShifts []) shows "not open" and the open-shift hint, and no opened-by line', async () => {
    handlers[VIEW_OVERVIEW] = () => ok(answer({ shiftOpen: false, pendingCount: 0, openShifts: [] }));
    const m = mount();
    await settle();
    const s = strip(m.el);
    expect(s).toContain(table.th.shiftClosed);
    expect(s).toContain(table.th.openShiftHint);
    expect(s).not.toContain(OPENED_BY);
  });

  it('row 4: shiftOpen true with an empty openShifts shows "open" and no opened-by line', async () => {
    handlers[VIEW_OVERVIEW] = () => ok(answer({ shiftOpen: true, pendingCount: 0, openShifts: [] }));
    const m = mount();
    await settle();
    const s = strip(m.el);
    expect(s).toContain(table.th.shiftOpen);
    expect(s).not.toContain(OPENED_BY);
  });

  it('row 5 (kept): the overview read fails: the strip shows statusError and refreshHint', async () => {
    handlers[VIEW_OVERVIEW] = () => ({ status: 500, body: { error: { code: 'serverError', message: 'boom' } } });
    const m = mount();
    await settle();
    const s = strip(m.el);
    expect(s).toContain(table.th.statusError);
    expect(s).toContain(table.th.refreshHint);
  });

  it('row 6: shiftOpen true with openShifts missing from the answer counts as an empty list (no crash, no opened-by line)', async () => {
    handlers[VIEW_OVERVIEW] = () => ok(answer({ shiftOpen: true, pendingCount: 0 }));
    const m = mount();
    await settle();
    const s = strip(m.el);
    expect(s).toContain(table.th.shiftOpen);
    expect(s).not.toContain(OPENED_BY);
  });
});
