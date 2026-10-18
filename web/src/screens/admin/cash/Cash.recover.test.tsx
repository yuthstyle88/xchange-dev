import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { requestRefresh } from '../parts/refresh';
import { Toast } from '../parts/toast';
import { PAGE_WIDE, QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

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
const VIEWS = [CASH, MOVES, SHIFTS];

const REGISTER = ['THB', 'EUR', 'USD'];

function openCash() {
  return {
    shift: { id: 'S-0001', openedAt: '2026-10-06T02:30:00.000Z', staff: 'EMP-007' },
    stock: [
      { code: 'THB', opening: '100000.00', net: '-2230.00', expected: '97770.00' },
      { code: 'USD', opening: '1000.00', net: '7.25', expected: '1100.50' },
    ],
    stats: { opening: '100000.00', incoming: '920.00', outgoing: '3150.00', remaining: '12345.67' },
    reference: { withReference: 2, withoutReference: 1, spread: '240.00', percent: '6.74' },
    currencies: REGISTER,
    lastCounted: [{ code: 'THB', amount: '5000.00' }],
  };
}

function movements() {
  return [
    {
      id: 'MV-2',
      created: '2026-10-06T03:00:00.000Z',
      shiftId: 'S-0001',
      direction: 'OUT',
      currency: 'USD',
      amount: '10.25',
      reason: 'ส่งเซฟ',
      staff: 'EMP-007',
    },
  ];
}

function shifts() {
  return [
    {
      id: 'S-0001',
      status: 'OPEN',
      openedAt: '2026-10-06T02:30:00.000Z',
      staff: 'EMP-007',
      note: '',
      difference: null,
      balances: [{ code: 'THB', expected: '97770.00', counted: null, difference: null }],
    },
  ];
}

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
}
type Reply = { status: number; body: unknown };

class Server {
  version = 7;
  cash = openCash();
  moves = movements();
  shifts = shifts();
  cashRefusal: Reply | null = null;
  movesRefusal: Reply | null = null;
  shiftsRefusal: Reply | null = null;
  sent: Sent[] = [];

  reads(path: string): Sent[] {
    return this.sent.filter((s) => s.path === path);
  }

  own(): Sent[] {
    return this.sent.filter((s) => !PAGE_WIDE.includes(s.path));
  }

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input), 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    this.sent.push({ path: url.pathname, params: new URLSearchParams(url.searchParams), method });
    const json = (reply: Reply): Response =>
      new Response(JSON.stringify(reply.body), {
        status: reply.status,
        headers: { 'content-type': 'application/json' },
      });
    if (url.pathname === '/api/v3/views/me') {
      return json({
        status: 200,
        body: {
          version: 1,
          changed: true,
          data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' },
        },
      });
    }
    if (url.pathname === QUEUE_CARD_PATH) {
      return json({ status: 200, body: queueCardUnchanged() });
    }
    if (method === 'GET' && url.pathname === CASH) {
      if (this.cashRefusal) return json(this.cashRefusal);
      if (url.searchParams.get('since') === String(this.version)) {
        return json({ status: 200, body: { version: this.version, changed: false } });
      }
      return json({ status: 200, body: { version: this.version, changed: true, data: this.cash } });
    }
    if (method === 'GET' && url.pathname === MOVES) {
      if (this.movesRefusal) return json(this.movesRefusal);
      return json({ status: 200, body: { version: this.version, changed: true, data: this.moves } });
    }
    if (method === 'GET' && url.pathname === SHIFTS) {
      if (this.shiftsRefusal) return json(this.shiftsRefusal);
      return json({ status: 200, body: { version: this.version, changed: true, data: this.shifts } });
    }
    return json({
      status: 500,
      body: { error: { code: 'serverError', message: `unexpected ${method} ${url.pathname}` } },
    });
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

function mount(init: { branch?: string; shown?: boolean } = {}) {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(init.branch ?? 'MAIN');
  const [period, setPeriod] = createSignal({ from: '2026-10-06', to: '2026-10-06' });
  const [shown, setShown] = createSignal(init.shown ?? true);
  const Component = menu.Component;
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
  return { el: root, setBranch, setPeriod, setShown };
}

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function button(el: ParentNode, name: string): HTMLButtonElement {
  const found = [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
  expect(found.length, `one button ${JSON.stringify(name)}`).toBe(1);
  return found[0]!;
}

const SERVER_ERR: Reply = {
  status: 500,
  body: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } },
};

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

describe('Admin Cash recovery after a failed read (#2150 §9)', () => {
  it('row 1: admin_cash fails once, then the next refresh succeeds -> panel shows data, error is gone, write buttons enabled, recovery read carries no since', async () => {
    const m = mount();
    await settle();
    expect(server.reads(CASH).length).toBe(1);
    expect(text(m.el)).toContain(formatMoney('12345.67'));
    expect(button(m.el, 'ดูตัวอย่าง 500,000').disabled).toBe(false);

    // Next refresh fails on admin_cash after cursor version=7 was already stored
    server.cashRefusal = SERVER_ERR;
    requestRefresh();
    await settle();
    expect(m.el.querySelector('.error-banner')?.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง');
    expect(button(m.el, 'ดูตัวอย่าง 500,000').disabled).toBe(true);
    expect(text(m.el)).not.toContain(formatMoney('12345.67'));

    // Next refresh succeeds at the same server version (7): must carry no since and recover full panel
    server.cashRefusal = null;
    requestRefresh();
    await settle();
    const cashReads = server.reads(CASH);
    expect(cashReads.length).toBe(3);
    expect(cashReads[2]!.params.has('since'), 'recovery admin_cash read carries no since').toBe(false);
    expect(m.el.querySelector('.error-banner')).toBeNull();
    expect(text(m.el)).toContain('กะกำลังเปิด · S-0001');
    expect(text(m.el)).toContain(formatMoney('12345.67'));
    expect(button(m.el, 'ดูตัวอย่าง 500,000').disabled).toBe(false);
  });

  it('row 2: movements or shifts read fails once, then the next refresh succeeds -> error is gone and write buttons are enabled', async () => {
    server.movesRefusal = SERVER_ERR;
    server.shiftsRefusal = SERVER_ERR;
    const m = mount();
    await settle();
    expect(m.el.querySelector('.error-banner')?.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง');
    expect(button(m.el, 'ดูตัวอย่าง 500,000').disabled).toBe(true);

    // Server recovers while admin_cash version is unchanged
    server.movesRefusal = null;
    server.shiftsRefusal = null;
    requestRefresh();
    await settle();

    expect(m.el.querySelector('.error-banner')).toBeNull();
    expect(button(m.el, 'ดูตัวอย่าง 500,000').disabled).toBe(false);
    expect(server.reads(MOVES).length).toBe(2);
    expect(server.reads(SHIFTS).length).toBe(2);
    expect(text(m.el)).toContain('ส่งเซฟ');
  });

  it('row 3: a read still fails on the next refresh -> the error stays and the writes stay disabled', async () => {
    server.cashRefusal = SERVER_ERR;
    const m = mount();
    await settle();
    expect(m.el.querySelector('.error-banner')?.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง');
    expect(button(m.el, 'ดูตัวอย่าง 500,000').disabled).toBe(true);

    requestRefresh();
    await settle();
    expect(server.reads(CASH).length).toBe(2);
    expect(m.el.querySelector('.error-banner')?.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง');
    expect(button(m.el, 'ดูตัวอย่าง 500,000').disabled).toBe(true);
  });

  it('row 4: no error, and the poll answers changed: false -> nothing is re-read or re-rendered; the cursor is kept', async () => {
    const m = mount();
    await settle();
    expect(server.reads(CASH).length).toBe(1);
    expect(server.reads(MOVES).length).toBe(1);
    expect(server.reads(SHIFTS).length).toBe(1);
    const cardBefore = m.el.querySelector('.stat-card');

    requestRefresh();
    await settle();
    expect(server.reads(CASH).length).toBe(2);
    expect(server.reads(CASH)[1]!.params.get('since')).toBe('7');
    expect(server.reads(MOVES).length, 'movements not re-read on healthy poll').toBe(1);
    expect(server.reads(SHIFTS).length, 'shifts not re-read on healthy poll').toBe(1);
    expect(m.el.querySelector('.stat-card')).toBe(cardBefore);

    // Cursor is kept for subsequent polls too
    requestRefresh();
    await settle();
    expect(server.reads(CASH).length).toBe(3);
    expect(server.reads(CASH)[2]!.params.get('since')).toBe('7');
  });

  it('row 5: switching branch -> unchanged (full reload of all views without since)', async () => {
    const m = mount({ branch: 'MAIN' });
    await settle();
    for (const v of VIEWS) expect(server.reads(v).length).toBe(1);

    m.setBranch('B9');
    await settle();
    for (const v of VIEWS) {
      expect(server.reads(v).length, v).toBe(2);
      expect(server.reads(v)[1]!.params.get('branch'), v).toBe('B9');
      expect(server.reads(v)[1]!.params.has('since'), `${v} carries no since on branch switch`).toBe(false);
    }
  });
});
