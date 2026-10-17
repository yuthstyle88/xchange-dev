// CA-10q-FE #3023: the admin cash screen reads `openShifts` (every OPEN shift of the branch, one per teller,
// in the server's order by openedAt) instead of the single `shift` of v1. Rows of #3023 §8, rulings (a)–(c) of
// the FE controller: hasOpenShift = openShifts.length > 0 (a missing key reads as []), one banner line per open
// shift in server order with its own id / openedAt / tellerName, and the reference gate uses the same test.
// Written before the implementation by claude/studio-claude-3. Do not edit, delete or weaken; add tests in a
// file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminCashData } from '../../../shared/api/generated/admin_cash';
import type { OpenShiftDto } from '../../../shared/api/generated/common';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/cash/menu.ts exists').toBeDefined();
  return m!.menu;
}

const CASH = '/api/v3/views/admin_cash';
const MOVES = '/api/v3/views/admin_cash_movements';
const SHIFTS = '/api/v3/views/admin_shifts';

// i18n.ts (th) — the texts the screen already has; #3023 adds none.
const NO_OPEN_SHIFT = 'ยังไม่มีกะเปิด';
const STOCK_NOTE_1 = 'เริ่มจากยอดเงินที่ตรวจนับจริง';
const REF_NO_RATES = 'ส่วนต่างเทียบเรทอ้างอิงในกะ: ยังไม่มีเรทอ้างอิง';
const bannerLine = (id: string): string => `กะกำลังเปิด · ${id}`;
const bannerSub = (iso: string, staff: string): string =>
  `เปิด ${when(iso)} · ${staff} · ยอดกะไม่ขึ้นกับตัวกรองวันที่ของรายงาน`;
const refWithRates = (spread: string, percent: string): string =>
  `ส่วนต่างเทียบเรทอ้างอิงในกะ: ${formatMoney(spread)} THB (${percent}%)`;

/** The screen's own formatting of an instant (Cash.tsx `when`). */
function when(iso: string): string {
  return new Date(iso).toLocaleString('th-TH', { timeZone: 'Asia/Bangkok' }).replace(/\s+/g, ' ').trim();
}

const SHIFT_A: OpenShiftDto = { shiftId: 'S-0001', tellerId: 'u-7', tellerName: 'EMP-007', openedAt: '2026-10-06T02:30:00.000Z' };
// Two OPEN shifts as the server orders them; deliberately not sorted by id, and the later teller's name
// sorts first, so a screen that re-sorts by id, by name or by time-descending shows another order.
const SHIFT_EARLY: OpenShiftDto = { shiftId: 'S-0009', tellerId: 'u-9', tellerName: 'Zed', openedAt: '2026-10-06T01:00:00.000Z' };
const SHIFT_LATE: OpenShiftDto = { shiftId: 'S-0002', tellerId: 'u-2', tellerName: 'Ann', openedAt: '2026-10-06T04:15:00.000Z' };

function cashWith(openShifts: OpenShiftDto[], withReference = 2): AdminCashData {
  return {
    openShifts,
    stock: [{ code: 'THB', opening: '100000.00', net: '-2230.00', expected: '97770.00' }],
    stats: openShifts.length > 0 ? { opening: '100000.00', incoming: '920.00', outgoing: '3150.00', remaining: '12345.67' } : null,
    reference: { withReference, withoutReference: 1, spread: '240.00', percent: '6.74' },
    currencies: ['THB', 'USD'],
    lastCounted: null,
  };
}

type Reply = { status: number; body: unknown };

class Server {
  version = 7;
  /** The `data` admin_cash answers; `unknown` so a test can drop the `openShifts` key. */
  cash: unknown = cashWith([SHIFT_A]);

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input), 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    const json = (reply: Reply): Response =>
      new Response(JSON.stringify(reply.body), { status: reply.status, headers: { 'content-type': 'application/json' } });
    if (url.pathname === '/api/v3/views/me') {
      return json({
        status: 200,
        body: { version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' } },
      });
    }
    if (url.pathname === QUEUE_CARD_PATH) return json({ status: 200, body: queueCardUnchanged() });
    if (method === 'GET' && url.pathname === CASH) {
      return json({ status: 200, body: { version: this.version, changed: true, data: this.cash } });
    }
    if (method === 'GET' && (url.pathname === MOVES || url.pathname === SHIFTS)) {
      return json({ status: 200, body: { version: this.version, changed: true, data: [] } });
    }
    return json({ status: 500, body: { error: { code: 'serverError', message: `unexpected ${method} ${url.pathname}` } } });
  };
}

let server: Server;
let dispose: (() => void) | undefined;
let root: HTMLElement;

async function settle(): Promise<void> {
  for (let round = 0; round < 3; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

async function mounted(): Promise<HTMLElement> {
  const Component = theMenu().Component;
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-06', to: '2026-10-06' });
  const [shown] = createSignal(true);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  await settle();
  return root;
}

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

/** The banner lines naming an open shift, in document order. */
function bannerLines(el: HTMLElement): string[] {
  return [...el.querySelectorAll('.stock-banner strong')].map(text).filter((s) => s.startsWith('กะกำลังเปิด'));
}

/** True when the stock table (gated by hasOpenShift) is shown rather than its empty note. */
function stockTableShown(el: HTMLElement): boolean {
  return el.querySelector('.table-scroll') !== null && !text(el).includes(STOCK_NOTE_1);
}

beforeEach(() => {
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
  location.hash = '#cash';
  setLang('th');
});

afterEach(async () => {
  await settle();
  await pendingWrite.abandon();
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
  await settle();
});

describe('CA-10q-FE #3023: the admin cash screen reads openShifts', () => {
  it('0 open shifts: the no-open-shift banner, no shift line, the stock note instead of the table, no-rates reference', async () => {
    server.cash = cashWith([], 2);
    const el = await mounted();
    expect(text(el)).toContain(NO_OPEN_SHIFT);
    expect(bannerLines(el)).toEqual([]);
    expect(stockTableShown(el)).toBe(false);
    expect(text(el)).toContain(REF_NO_RATES);
    expect(text(el)).not.toContain(refWithRates('240.00', '6.74'));
  });

  it('openShifts key missing: read as [] (ruling a), the same as 0 open shifts', async () => {
    const data = cashWith([SHIFT_A], 2) as Partial<AdminCashData>;
    delete data.openShifts;
    server.cash = data;
    const el = await mounted();
    expect(el.querySelector('.error-banner')).toBeNull();
    expect(text(el)).toContain(NO_OPEN_SHIFT);
    expect(bannerLines(el)).toEqual([]);
    expect(stockTableShown(el)).toBe(false);
    expect(text(el)).toContain(REF_NO_RATES);
  });

  it('1 open shift: one banner line with its shiftId, then its openedAt and tellerName; the stock table is shown', async () => {
    server.cash = cashWith([SHIFT_A], 0);
    const el = await mounted();
    expect(bannerLines(el)).toEqual([bannerLine('S-0001')]);
    expect(text(el)).toContain(bannerSub(SHIFT_A.openedAt, 'EMP-007'));
    expect(text(el)).not.toContain(NO_OPEN_SHIFT);
    expect(stockTableShown(el)).toBe(true);
  });

  it('2 open shifts: two banner lines in the order the server sends, each with its own openedAt and tellerName', async () => {
    server.cash = cashWith([SHIFT_EARLY, SHIFT_LATE], 0);
    const el = await mounted();
    expect(bannerLines(el)).toEqual([bannerLine('S-0009'), bannerLine('S-0002')]);
    const all = text(el);
    const subEarly = bannerSub(SHIFT_EARLY.openedAt, 'Zed');
    const subLate = bannerSub(SHIFT_LATE.openedAt, 'Ann');
    expect(all).toContain(subEarly);
    expect(all).toContain(subLate);
    // each line is followed by its own shift's details, in server order
    const at = (s: string): number => all.indexOf(s);
    expect(at(bannerLine('S-0009'))).toBeLessThan(at(subEarly));
    expect(at(subEarly)).toBeLessThan(at(bannerLine('S-0002')));
    expect(at(bannerLine('S-0002'))).toBeLessThan(at(subLate));
    expect(stockTableShown(el)).toBe(true);
  });

  it('reference gate (ruling c): an open shift and withReference > 0 -> the with-rates summary', async () => {
    server.cash = cashWith([SHIFT_A], 2);
    const el = await mounted();
    expect(text(el)).toContain(refWithRates('240.00', '6.74'));
    expect(text(el)).not.toContain(REF_NO_RATES);
  });

  it('reference gate (ruling c): two open shifts and withReference > 0 -> the with-rates summary', async () => {
    server.cash = cashWith([SHIFT_EARLY, SHIFT_LATE], 2);
    const el = await mounted();
    expect(text(el)).toContain(refWithRates('240.00', '6.74'));
  });
});
