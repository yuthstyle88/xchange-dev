// R3-4a (docs/design/web-silent-failures.md §7.2): `Cash.tsx` keeps one error per read (`cashError`,
// `movementsError`, `shiftsError`), each set and cleared only by its own read and shown above its own
// list; while a list's error is set its empty text is not shown. Up-front tests: rows (a), (b) and (d)
// are red until the single `cashError` is split; row (c) is green at 36abb596 (see its comment).
// Each per-read error keeps `class="error-banner" role="alert"` (Cash.recover.test.tsx and
// worded.test.tsx select `.error-banner`): the movements and shifts ones sit inside their own list's
// card or right above it; the cash one sits before the stock card and with neither history list.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeAdminCashData,
  makeAdminCashMovementRow,
  makeAdminShiftRow,
  makeMe,
} from '../../../shared/testing/fixtures';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { menu } from './menu';
import * as texts from './i18n';

const CASH = '/api/v3/views/admin_cash';
const MOVES = '/api/v3/views/admin_cash_movements';
const SHIFTS = '/api/v3/views/admin_shifts';

/** Each read fails with its own server sentence, so the screen shows which read failed. */
const CASH_MSG = 'อ่านยอดเงินสดไม่ได้';
const MOVES_MSG = 'อ่านรายการรับจ่ายไม่ได้';
const SHIFTS_MSG = 'อ่านประวัติกะไม่ได้';

const CASH_DATA = makeAdminCashData({
  openShifts: [],
  stock: [],
  stats: null,
  reference: { withReference: 0, withoutReference: 0, spread: '0.00', percent: '0.00' },
  currencies: ['THB'],
  lastCounted: null,
});

const MOVE = makeAdminCashMovementRow({
  id: 'MV-1',
  created: '2026-10-06T03:00:00.000Z',
  shiftId: 'S-0001',
  direction: 'OUT',
  currency: 'USD',
  amount: '10.25',
  reason: 'ส่งเซฟ-MV1',
});

const SHIFT = makeAdminShiftRow({ id: 'S-0042', status: 'CLOSED', openedAt: '2026-10-06T02:30:00.000Z', staff: 'EMP-042' });

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const refuse = (message: string) => json(400, { error: { code: 'unkeyedReadCode', message } });

class Server {
  cashFails = false;
  movesFails = false;
  shiftsFails = false;

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(raw, 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    const view = (data: unknown) => json(200, { version: 1, changed: true, data });
    if (method !== 'GET') return json(404, { error: { code: 'notFound', message: `unexpected ${method}` } });
    switch (u.pathname) {
      case '/api/v3/views/me':
        return view(makeMe({ id: 'u-owner', permissions: [], branch: 'MAIN' }));
      case QUEUE_CARD_PATH:
        return json(200, queueCardUnchanged());
      case CASH:
        return this.cashFails ? refuse(CASH_MSG) : view(CASH_DATA);
      case MOVES:
        return this.movesFails ? refuse(MOVES_MSG) : view([MOVE]);
      case SHIFTS:
        return this.shiftsFails ? refuse(SHIFTS_MSG) : view([SHIFT]);
      default:
        return view({ rows: [] });
    }
  };
}

let server: Server;
let dispose: (() => void) | undefined;
let root: HTMLElement;

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

async function mount(): Promise<HTMLElement> {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-06', to: '2026-10-06' });
  const [shown] = createSignal(true);
  const Component = (menu as AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element }).Component;
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
  return root.querySelector<HTMLElement>('.admin-cash-screen')!;
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
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
});

const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const banners = (s: ParentNode) => Array.from(s.querySelectorAll<HTMLElement>('.error-banner[role="alert"]'));
const bannerWith = (s: ParentNode, message: string) => banners(s).find((b) => text(b).includes(message)) ?? null;
const hasAlert = (s: ParentNode, message: string) => bannerWith(s, message) !== null;
const card = (s: ParentNode, title: string) =>
  Array.from(s.querySelectorAll<HTMLElement>('section.card')).find((c) => text(c.querySelector('.card-title')) === title)!;
/** Inside the list's card, or the element right above it. */
const sitsWith = (banner: HTMLElement | null, list: HTMLElement) =>
  banner !== null && (list.contains(banner) || banner.nextElementSibling === list);
const movementsCard = (s: ParentNode) => card(s, texts.th.historyMovementsTitle);
const shiftsCard = (s: ParentNode) => card(s, texts.th.historyShiftsTitle);
const stockCard = (s: ParentNode) => s.querySelector<HTMLElement>('section.card.stock-card')!;

async function toggleShowAll(s: HTMLElement): Promise<void> {
  const box = s.querySelector<HTMLInputElement>('.stock-filter input[type="checkbox"]')!;
  box.checked = !box.checked;
  box.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
}

describe('Admin Cash keeps one error per read (R3-4a)', () => {
  it('(a) a movements failure stays shown after "show all" re-reads cash successfully, and "no movements" is not shown', async () => {
    server.movesFails = true;
    const s = await mount();
    expect(hasAlert(s, MOVES_MSG), 'the movements error shows after the first read').toBe(true);

    await toggleShowAll(s);

    expect(hasAlert(s, MOVES_MSG), 'the movements error is still shown').toBe(true);
    expect(sitsWith(bannerWith(s, MOVES_MSG), movementsCard(s)), 'the movements error sits with the movements list').toBe(true);
    expect(text(s)).not.toContain(texts.th.emptyMovements);
  });

  it('(b) a shifts failure stays shown after "show all" re-reads cash successfully, and "no shifts" is not shown', async () => {
    server.shiftsFails = true;
    const s = await mount();
    expect(hasAlert(s, SHIFTS_MSG), 'the shifts error shows after the first read').toBe(true);

    await toggleShowAll(s);

    expect(hasAlert(s, SHIFTS_MSG), 'the shifts error is still shown').toBe(true);
    expect(sitsWith(bannerWith(s, SHIFTS_MSG), shiftsCard(s)), 'the shifts error sits with the shifts list').toBe(true);
    expect(text(s)).not.toContain(texts.th.emptyShifts);
  });

  // Green at 36abb596: readCash's catch sets cashError and clears only `cash`; the movements and shifts
  // signals are never touched, so their rows stay. Kept as the brief's row (c), a guard for the split.
  it('(c) a cash read failure shows its own error, while movements and shifts that loaded keep their rows', async () => {
    server.cashFails = true;
    const s = await mount();

    expect(hasAlert(s, CASH_MSG), 'the cash error shows').toBe(true);
    const cashBanner = bannerWith(s, CASH_MSG)!;
    expect(cashBanner.compareDocumentPosition(stockCard(s)) & Node.DOCUMENT_POSITION_FOLLOWING, 'before the stock card').toBeTruthy();
    expect(sitsWith(cashBanner, movementsCard(s))).toBe(false);
    expect(sitsWith(cashBanner, shiftsCard(s))).toBe(false);
    expect(hasAlert(s, MOVES_MSG)).toBe(false);
    expect(hasAlert(s, SHIFTS_MSG)).toBe(false);
    expect(text(s)).toContain('ส่งเซฟ-MV1');
    expect(text(s)).toContain('S-0042');
    expect(text(s)).not.toContain(texts.th.emptyMovements);
    expect(text(s)).not.toContain(texts.th.emptyShifts);
  });

  it('(d) a movements failure and a later cash failure (on "show all") are both shown, each its own', async () => {
    server.movesFails = true;
    const s = await mount();
    expect(hasAlert(s, MOVES_MSG)).toBe(true);

    server.cashFails = true;
    await toggleShowAll(s);

    expect(hasAlert(s, CASH_MSG), 'the cash error shows').toBe(true);
    expect(hasAlert(s, MOVES_MSG), 'the movements error is still shown').toBe(true);
    expect(bannerWith(s, CASH_MSG)).not.toBe(bannerWith(s, MOVES_MSG));
    expect(sitsWith(bannerWith(s, MOVES_MSG), movementsCard(s))).toBe(true);
  });
});
