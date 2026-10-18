import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
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

const REGISTER = ['THB', 'EUR', 'USD', 'JPY'];

function makeCash(opts: {
  shiftId?: string;
  remaining?: string;
  includeZeroJpy?: boolean;
} = {}) {
  const stock = [
    { code: 'THB', opening: '100000.00', net: '-2230.00', expected: '97770.00' },
    { code: 'USD', opening: '1000.00', net: '7.25', expected: '1100.50' },
  ];
  if (opts.includeZeroJpy) {
    stock.push({ code: 'JPY', opening: '0.00', net: '0.00', expected: '0.00' });
  }
  return {
    shift: {
      id: opts.shiftId ?? 'S-0001',
      openedAt: '2026-10-06T02:30:00.000Z',
      staff: 'EMP-007',
    },
    stock,
    stats: {
      opening: '100000.00',
      incoming: '920.00',
      outgoing: '3150.00',
      remaining: opts.remaining ?? '12345.67',
    },
    reference: { withReference: 2, withoutReference: 1, spread: '240.00', percent: '6.74' },
    currencies: REGISTER,
    lastCounted: [{ code: 'THB', amount: '5000.00' }],
  };
}

function makeMovements(reason = 'ส่งเซฟ MAIN') {
  return [
    {
      id: 'MV-1',
      created: '2026-10-06T03:00:00.000Z',
      shiftId: 'S-0001',
      direction: 'OUT',
      currency: 'USD',
      amount: '10.25',
      reason,
      staff: 'EMP-007',
    },
  ];
}

function makeShifts(shiftId = 'S-0001') {
  return [
    {
      id: shiftId,
      status: 'OPEN',
      openedAt: '2026-10-06T02:30:00.000Z',
      staff: 'EMP-007',
      note: '',
      difference: null,
      balances: [{ code: 'THB', expected: '97770.00', counted: null, difference: null }],
    },
  ];
}

type Reply = { status: number; body: unknown };

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

function jsonReply(reply: Reply): Response {
  return new Response(JSON.stringify(reply.body), {
    status: reply.status,
    headers: { 'content-type': 'application/json' },
  });
}

const SERVER_ERR: Reply = {
  status: 500,
  body: { error: { code: 'serverError', message: 'เซิร์ฟเวอร์ขัดข้อง' } },
};

class Server {
  sent: Sent[] = [];
  cashHandler: (params: URLSearchParams) => Reply | Promise<Reply> = (params) => {
    const branch = params.get('branch') ?? 'MAIN';
    const all = params.get('all') === 'true';
    const version = branch === 'SILOM' ? 42 : 7;
    if (params.get('since') === String(version)) {
      return { status: 200, body: { version, changed: false } };
    }
    return {
      status: 200,
      body: {
        version,
        changed: true,
        data: makeCash({
          shiftId: branch === 'SILOM' ? 'S-SILOM' : 'S-MAIN',
          remaining: branch === 'SILOM' ? '88888.88' : '12345.67',
          includeZeroJpy: all,
        }),
      },
    };
  };
  movesHandler: (params: URLSearchParams) => Reply | Promise<Reply> = (params) => {
    const branch = params.get('branch') ?? 'MAIN';
    return {
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: makeMovements(branch === 'SILOM' ? 'โอนเงิน SILOM' : 'ส่งเซฟ MAIN'),
      },
    };
  };
  shiftsHandler: (params: URLSearchParams) => Reply | Promise<Reply> = (params) => {
    const branch = params.get('branch') ?? 'MAIN';
    return {
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: makeShifts(branch === 'SILOM' ? 'S-SILOM' : 'S-MAIN'),
      },
    };
  };

  reads(path: string): Sent[] {
    return this.sent.filter((s) => s.path === path);
  }

  own(): Sent[] {
    return this.sent.filter((s) => !PAGE_WIDE.includes(s.path));
  }

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input), 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    const params = new URLSearchParams(url.searchParams);
    this.sent.push({ path: url.pathname, params, method });

    if (url.pathname === '/api/v3/views/me') {
      return jsonReply({
        status: 200,
        body: {
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
        },
      });
    }
    if (url.pathname === QUEUE_CARD_PATH) {
      return jsonReply({ status: 200, body: queueCardUnchanged() });
    }
    if (method === 'GET' && url.pathname === CASH) {
      return jsonReply(await this.cashHandler(params));
    }
    if (method === 'GET' && url.pathname === MOVES) {
      return jsonReply(await this.movesHandler(params));
    }
    if (method === 'GET' && url.pathname === SHIFTS) {
      return jsonReply(await this.shiftsHandler(params));
    }
    return jsonReply({
      status: 200,
      body: { version: 1, changed: true, data: [] },
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
  const found = [...el.querySelectorAll<HTMLButtonElement>('button')].filter(
    (b) => text(b) === name,
  );
  expect(found.length, `one button ${JSON.stringify(name)}`).toBe(1);
  return found[0]!;
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

const LOAD_OLDER = 'โหลดรายการก่อนหน้า';

function makeOlderShift(id: string) {
  return {
    id,
    status: 'CLOSED',
    openedAt: '2026-10-05T02:30:00.000Z',
    staff: 'EMP-008',
    note: '',
    difference: [],
    balances: [{ code: 'THB', expected: '5000.00', counted: '5000.00', difference: '0.00' }],
  };
}

function shiftsText(el: Element): string {
  return text(el.querySelector('.shifts-table'));
}

function olderAlert(el: Element): Element | null {
  return el.querySelector('.load-older-shifts [role="alert"]');
}

function loadOlderButtons(el: Element): HTMLButtonElement[] {
  return [...el.querySelectorAll<HTMLButtonElement>('button')].filter(
    (b) => text(b) === LOAD_OLDER,
  );
}

/**
 * MAIN's older page is held until the test releases it; SILOM's older page
 * answers at once with one older row.
 */
function holdMainOlder(): Deferred<Reply> {
  const mainOlder = deferred<Reply>();
  server.shiftsHandler = (params) => {
    const branch = params.get('branch') ?? 'MAIN';
    if (params.has('before')) {
      if (branch === 'MAIN') return mainOlder.promise;
      return {
        status: 200,
        body: { version: 1, changed: true, data: [makeOlderShift('S-SILOM-OLD')] },
      };
    }
    return {
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: makeShifts(branch === 'SILOM' ? 'S-SILOM' : 'S-MAIN'),
      },
    };
  };
  return mainOlder;
}

describe('Admin Cash older shifts across a branch switch (#2470)', () => {
  it("row 1: load older in A, switch to B, A's page answers -> B's shift list has no A rows", async () => {
    const mainOlder = holdMainOlder();
    const m = mount({ branch: 'MAIN' });
    await settle();
    expect(shiftsText(m.el)).toContain('S-MAIN');

    button(m.el, LOAD_OLDER).click();
    await settle();

    m.setBranch('SILOM');
    await settle();
    expect(shiftsText(m.el)).toContain('S-SILOM');

    mainOlder.resolve({
      status: 200,
      body: { version: 1, changed: true, data: [makeOlderShift('S-MAIN-OLD')] },
    });
    await settle();

    expect(shiftsText(m.el)).toContain('S-SILOM');
    expect(shiftsText(m.el)).not.toContain('S-MAIN-OLD');
    expect(shiftsText(m.el)).not.toContain('S-MAIN');
  });

  it("row 2: load older in A, switch to B, A's page fails -> no older-shifts error on B", async () => {
    const mainOlder = holdMainOlder();
    const m = mount({ branch: 'MAIN' });
    await settle();

    button(m.el, LOAD_OLDER).click();
    await settle();

    m.setBranch('SILOM');
    await settle();

    mainOlder.resolve(SERVER_ERR);
    await settle();

    expect(olderAlert(m.el)).toBeNull();
  });

  it("row 2b: A's older-shifts error shown before the switch is cleared on B", async () => {
    const mainOlder = holdMainOlder();
    const m = mount({ branch: 'MAIN' });
    await settle();

    button(m.el, LOAD_OLDER).click();
    mainOlder.resolve(SERVER_ERR);
    await settle();
    expect(text(olderAlert(m.el))).toContain('เซิร์ฟเวอร์ขัดข้อง');

    m.setBranch('SILOM');
    await settle();

    expect(olderAlert(m.el)).toBeNull();
  });

  it("row 3: load older in B after the switch -> pages from B's last row", async () => {
    const mainOlder = holdMainOlder();
    const m = mount({ branch: 'MAIN' });
    await settle();

    button(m.el, LOAD_OLDER).click();
    await settle();

    m.setBranch('SILOM');
    await settle();

    mainOlder.resolve({
      status: 200,
      body: { version: 1, changed: true, data: [makeOlderShift('S-MAIN-OLD')] },
    });
    await settle();

    const btn = button(m.el, LOAD_OLDER);
    expect(btn.disabled).toBe(false);
    btn.click();
    await settle();

    const silomOlder = server
      .reads(SHIFTS)
      .filter((s) => s.params.get('branch') === 'SILOM' && s.params.has('before'));
    expect(silomOlder.length).toBe(1);
    expect(silomOlder[0]!.params.get('before')).toBe('S-SILOM');
    expect(shiftsText(m.el)).toContain('S-SILOM-OLD');
  });

  it("row 3b: while A's older page is still in flight, B's load older is usable and pages from B's last row", async () => {
    holdMainOlder();
    const m = mount({ branch: 'MAIN' });
    await settle();

    button(m.el, LOAD_OLDER).click();
    await settle();

    m.setBranch('SILOM');
    await settle();

    const btn = button(m.el, LOAD_OLDER);
    expect(btn.disabled).toBe(false);
    btn.click();
    await settle();

    const silomOlder = server
      .reads(SHIFTS)
      .filter((s) => s.params.get('branch') === 'SILOM' && s.params.has('before'));
    expect(silomOlder.length).toBe(1);
    expect(silomOlder[0]!.params.get('before')).toBe('S-SILOM');
  });

  it("edge: hasMoreShifts from A's empty answer does not hide B's load older", async () => {
    const mainOlder = holdMainOlder();
    const m = mount({ branch: 'MAIN' });
    await settle();

    button(m.el, LOAD_OLDER).click();
    await settle();

    m.setBranch('SILOM');
    await settle();

    mainOlder.resolve({ status: 200, body: { version: 1, changed: true, data: [] } });
    await settle();

    expect(loadOlderButtons(m.el).length).toBe(1);
  });

  it("row 3c: load older clicked after the switch but before B's shift list lands -> B's list has only B's rows and pages from B's last row", async () => {
    const silomList = deferred<Reply>();
    const silomOlder = deferred<Reply>();
    server.shiftsHandler = (params) => {
      const branch = params.get('branch') ?? 'MAIN';
      if (branch === 'SILOM') {
        if (!params.has('before')) return silomList.promise;
        if (params.get('before') === 'S-MAIN') return silomOlder.promise;
        return {
          status: 200,
          body: { version: 1, changed: true, data: [makeOlderShift('S-SILOM-OLD')] },
        };
      }
      return {
        status: 200,
        body: { version: 1, changed: true, data: makeShifts('S-MAIN') },
      };
    };
    const m = mount({ branch: 'MAIN' });
    await settle();
    expect(shiftsText(m.el)).toContain('S-MAIN');

    m.setBranch('SILOM');
    await settle();

    // B's list is still held, so A's rows (and A's load older) may still be on screen.
    for (const b of loadOlderButtons(m.el)) b.click();
    await settle();

    silomList.resolve({
      status: 200,
      body: { version: 1, changed: true, data: makeShifts('S-SILOM') },
    });
    await settle();

    silomOlder.resolve({
      status: 200,
      body: { version: 1, changed: true, data: [makeOlderShift('S-OLDER-FROM-A-CURSOR')] },
    });
    await settle();

    expect(shiftsText(m.el)).toContain('S-SILOM');
    expect(shiftsText(m.el)).not.toContain('S-OLDER-FROM-A-CURSOR');
    expect(shiftsText(m.el)).not.toContain('S-MAIN');

    const btn = button(m.el, LOAD_OLDER);
    expect(btn.disabled).toBe(false);
    btn.click();
    await settle();

    const silomOlderReads = server
      .reads(SHIFTS)
      .filter((s) => s.params.get('branch') === 'SILOM' && s.params.has('before'));
    expect(silomOlderReads.length).toBeGreaterThan(0);
    for (const s of silomOlderReads) expect(s.params.get('before')).toBe('S-SILOM');
    expect(shiftsText(m.el)).toContain('S-SILOM-OLD');
  });
});
