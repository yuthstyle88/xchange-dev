import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { Cash } from './Cash';

const CASH = '/api/v3/views/admin_cash';
const MOVES = '/api/v3/views/admin_cash_movements';
const SHIFTS = '/api/v3/views/admin_shifts';

const ISO_MOVEMENT = '2026-10-15T18:30:00Z';
const ISO_SHIFT_OPENED = '2026-10-15T17:15:00Z';
const ISO_HISTORY_SHIFT = '2026-10-15T19:45:00Z';

const origToLocaleString = Date.prototype.toLocaleString;
const nodeEnv = (globalThis as { process?: { env: Record<string, string | undefined> } }).process
  ?.env;
const origTz = nodeEnv?.TZ;

function formatInBangkok(iso: string): string {
  return origToLocaleString
    .call(new Date(iso), 'th-TH', { timeZone: 'Asia/Bangkok' })
    .replace(/\s+/g, ' ')
    .trim();
}

function setDeviceTimeZone(tz: string): void {
  if (nodeEnv) {
    nodeEnv.TZ = tz;
  }
  vi.spyOn(Date.prototype, 'toLocaleString').mockImplementation(function (
    this: Date,
    locales?: Intl.LocalesArgument,
    options?: Intl.DateTimeFormatOptions,
  ): string {
    const effectiveOptions: Intl.DateTimeFormatOptions = {
      ...options,
      timeZone: options?.timeZone ?? tz,
    };
    return origToLocaleString.call(this, locales, effectiveOptions);
  });
}

function makeCash(branch = 'MAIN', openedAt = ISO_SHIFT_OPENED) {
  return {
    openShifts: [
      { shiftId: branch === 'SILOM' ? 'S-SILOM' : 'S-0001', tellerId: 'u-1', tellerName: 'EMP-007', openedAt },
    ],
    stock: [
      { code: 'THB', opening: '100000.00', net: '-2230.00', expected: '97770.00' },
      { code: 'USD', opening: '1000.00', net: '7.25', expected: '1100.50' },
    ],
    stats: {
      opening: '100000.00',
      incoming: '920.00',
      outgoing: '3150.00',
      remaining: branch === 'SILOM' ? '88888.88' : '12345.67',
    },
    reference: { withReference: 2, withoutReference: 1, spread: '240.00', percent: '6.74' },
    currencies: ['THB', 'EUR', 'USD', 'JPY'],
    lastCounted: [{ code: 'THB', amount: '5000.00' }],
  };
}

function makeMovements(created = ISO_MOVEMENT, reason = 'ส่งเซฟ MAIN') {
  return [
    {
      id: 'MV-1',
      created,
      shiftId: 'S-0001',
      direction: 'OUT',
      currency: 'USD',
      amount: '10.25',
      reason,
      staff: 'EMP-007',
    },
  ];
}

function makeShifts(openedAt = ISO_HISTORY_SHIFT, shiftId = 'S-0001') {
  return [
    {
      id: shiftId,
      status: 'OPEN',
      openedAt,
      staff: 'EMP-007',
      note: '',
      difference: null,
      balances: [{ code: 'THB', expected: '97770.00', counted: null, difference: null }],
    },
  ];
}

function jsonReply(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

let dispose: (() => void) | undefined;
let root: HTMLElement;

async function settle(): Promise<void> {
  for (let round = 0; round < 3; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function mount(init: { branch?: string; shown?: boolean } = {}) {
  const [branch, setBranch] = createSignal(init.branch ?? 'MAIN');
  const [period, setPeriod] = createSignal({ from: '2026-10-16', to: '2026-10-16' });
  const [shown, setShown] = createSignal(init.shown ?? true);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Cash
            branch={branch}
            period={period}
            shown={shown}
            {...({} as Omit<AdminMenuProps, 'branch' | 'period' | 'shown'>)}
          />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  return { el: root, setBranch, setPeriod, setShown };
}

beforeEach(() => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: string) => {
      const url = new URL(String(input), 'http://localhost');
      const branch = url.searchParams.get('branch') ?? 'MAIN';
      if (url.pathname === '/api/v3/views/me') {
        return jsonReply(200, {
          version: 1,
          changed: true,
          data: {
            kind: 'owner',
            id: 'owner',
            role: 'OWNER',
            branch: 'MAIN',
            name: 'Owner',
            username: 'owner',
          },
        });
      }
      if (url.pathname === QUEUE_CARD_PATH) {
        return jsonReply(200, queueCardUnchanged());
      }
      if (url.pathname === CASH) {
        return jsonReply(200, {
          version: branch === 'SILOM' ? 42 : 7,
          changed: true,
          data: makeCash(branch),
        });
      }
      if (url.pathname === MOVES) {
        return jsonReply(200, {
          version: 1,
          changed: true,
          data: makeMovements(
            ISO_MOVEMENT,
            branch === 'SILOM' ? 'โอนเงิน SILOM' : 'ส่งเซฟ MAIN',
          ),
        });
      }
      if (url.pathname === SHIFTS) {
        return jsonReply(200, {
          version: 1,
          changed: true,
          data: makeShifts(ISO_HISTORY_SHIFT, branch === 'SILOM' ? 'S-SILOM' : 'S-0001'),
        });
      }
      return jsonReply(200, { version: 1, changed: true, data: [] });
    }),
  );
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
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  if (nodeEnv) {
    if (origTz === undefined) {
      delete nodeEnv.TZ;
    } else {
      nodeEnv.TZ = origTz;
    }
  }
  location.hash = '';
  setLang('th');
  await settle();
});

describe('Admin Cash: format shift-opened and cash-movement times in Asia/Bangkok (#2506)', () => {
  it('row 1: Movement at 2026-10-15T18:30:00Z, device TZ UTC -> Shown as 16 Oct 01:30 (Bangkok, th-TH)', async () => {
    setDeviceTimeZone('UTC');

    const m = mount({ branch: 'MAIN' });
    await settle();

    const expectedMove = formatInBangkok(ISO_MOVEMENT);
    expect(expectedMove).toContain('16/10/2569');
    expect(expectedMove).toMatch(/0?1:30:00/);

    const moveCell = m.el.querySelector('.movements-table tbody tr td');
    expect(text(moveCell)).toBe(expectedMove);
  });

  it('row 2: Same timestamp, device TZ Asia/Bangkok -> Same text as device TZ UTC', async () => {
    setDeviceTimeZone('Asia/Bangkok');
    const bangkokMount = mount({ branch: 'MAIN' });
    await settle();
    const bangkokMoveText = text(bangkokMount.el.querySelector('.movements-table tbody tr td'));
    const bangkokBannerText = text(bangkokMount.el.querySelector('.stock-banner p'));
    const bangkokShiftText = text(
      bangkokMount.el.querySelectorAll('.shifts-table tbody tr td')[1],
    );
    dispose?.();
    dispose = undefined;
    root.remove();

    vi.restoreAllMocks();
    setDeviceTimeZone('UTC');
    const utcMount = mount({ branch: 'MAIN' });
    await settle();
    const utcMoveText = text(utcMount.el.querySelector('.movements-table tbody tr td'));
    const utcBannerText = text(utcMount.el.querySelector('.stock-banner p'));
    const utcShiftText = text(utcMount.el.querySelectorAll('.shifts-table tbody tr td')[1]);

    expect(utcMoveText).toBe(bangkokMoveText);
    expect(utcBannerText).toBe(bangkokBannerText);
    expect(utcShiftText).toBe(bangkokShiftText);
  });

  it('row 3: Cash reads and branch switch order (kept) while times stay in Asia/Bangkok under UTC device TZ', async () => {
    setDeviceTimeZone('UTC');

    const m = mount({ branch: 'MAIN' });
    await settle();
    expect(text(m.el)).toContain(formatMoney('12345.67'));
    expect(text(m.el.querySelector('.movements-table tbody tr td'))).toBe(
      formatInBangkok(ISO_MOVEMENT),
    );

    m.setBranch('SILOM');
    await settle();
    expect(text(m.el)).toContain(formatMoney('88888.88'));
    expect(text(m.el)).toContain('โอนเงิน SILOM');
    expect(text(m.el.querySelector('.movements-table tbody tr td'))).toBe(
      formatInBangkok(ISO_MOVEMENT),
    );
  });

  it('edge case: Non-Bangkok device zone (process.env.TZ = UTC) formats open-shift banner, movement row, and shift history row in Asia/Bangkok', async () => {
    setDeviceTimeZone('UTC');

    const m = mount({ branch: 'MAIN' });
    await settle();

    const bannerSub = text(m.el.querySelector('.stock-banner p'));
    const moveCell = text(m.el.querySelector('.movements-table tbody tr td'));
    const shiftCell = text(m.el.querySelectorAll('.shifts-table tbody tr td')[1]);

    expect(bannerSub).toContain(formatInBangkok(ISO_SHIFT_OPENED));
    expect(moveCell).toBe(formatInBangkok(ISO_MOVEMENT));
    expect(shiftCell).toBe(formatInBangkok(ISO_HISTORY_SHIFT));
  });
});
