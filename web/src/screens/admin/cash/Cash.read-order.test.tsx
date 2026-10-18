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

describe('Admin Cash out-of-order read protection (#2273)', () => {
  it("row 1: Branch A to B, A's tick read answers last -> shows B's figures and uses B's version on next tick", async () => {
    const m = mount({ branch: 'MAIN' });
    await settle();
    expect(text(m.el)).toContain(formatMoney('12345.67'));

    // Hold MAIN's tick read in flight while switching to SILOM
    const lateTick = deferred<Reply>();
    server.cashHandler = (params) => {
      if (params.get('branch') === 'MAIN') {
        return lateTick.promise;
      }
      return {
        status: 200,
        body: {
          version: 42,
          changed: true,
          data: makeCash({ shiftId: 'S-SILOM', remaining: '88888.88' }),
        },
      };
    };

    requestRefresh();
    await settle();

    // Switch to SILOM; SILOM's readAll finishes first
    m.setBranch('SILOM');
    await settle();
    expect(text(m.el)).toContain(formatMoney('88888.88'));

    // Now MAIN's tick read answers last with version 9 and MAIN's figure
    lateTick.resolve({
      status: 200,
      body: {
        version: 9,
        changed: true,
        data: makeCash({ shiftId: 'S-MAIN', remaining: '11111.11' }),
      },
    });
    await settle();

    // B's figures must remain on screen
    expect(text(m.el)).toContain(formatMoney('88888.88'));
    expect(text(m.el)).not.toContain(formatMoney('11111.11'));

    // Next tick on SILOM must send SILOM's version (42), not MAIN's stale version (9)
    requestRefresh();
    await settle();
    const cashReads = server.reads(CASH);
    const lastRead = cashReads[cashReads.length - 1]!;
    expect(lastRead.params.get('branch')).toBe('SILOM');
    expect(lastRead.params.get('since')).toBe('42');
  });

  it('row 2: Toggle on then off, the all=true answer comes last -> shows the filtered list', async () => {
    const m = mount({ branch: 'MAIN' });
    await settle();

    const lateAllTrue = deferred<Reply>();
    server.cashHandler = (params) => {
      if (params.get('all') === 'true') {
        return lateAllTrue.promise;
      }
      return {
        status: 200,
        body: {
          version: 11,
          changed: true,
          data: makeCash({ includeZeroJpy: false }),
        },
      };
    };

    const checkbox = m.el.querySelector<HTMLInputElement>(
      '.stock-filter input[type="checkbox"]',
    )!;
    expect(checkbox).not.toBeNull();

    // Toggle ON (all=true in flight)
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();

    // Toggle OFF (all=false resolves immediately)
    checkbox.checked = false;
    checkbox.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();

    // Now the late all=true answer arrives with JPY
    lateAllTrue.resolve({
      status: 200,
      body: {
        version: 10,
        changed: true,
        data: makeCash({ includeZeroJpy: true }),
      },
    });
    await settle();

    const stockTableText = text(m.el.querySelector('.stock-table'));
    expect(checkbox.checked).toBe(false);
    expect(stockTableText).toContain('THB');
    expect(stockTableText).toContain('USD');
    expect(stockTableText).not.toContain('JPY');
  });

  it('row 3: An older read fails after a newer success -> no error and writes stay enabled', async () => {
    const m = mount({ branch: 'MAIN' });
    await settle();

    const lateFail = deferred<Reply>();
    server.cashHandler = (params) => {
      if (params.get('branch') === 'MAIN') {
        return lateFail.promise;
      }
      return {
        status: 200,
        body: {
          version: 42,
          changed: true,
          data: makeCash({ shiftId: 'S-SILOM', remaining: '88888.88' }),
        },
      };
    };

    // Start a tick read on MAIN that will fail later
    requestRefresh();
    await settle();

    // Switch to SILOM which succeeds
    m.setBranch('SILOM');
    await settle();
    expect(text(m.el)).toContain(formatMoney('88888.88'));

    // Now the older MAIN read fails with 500
    lateFail.resolve(SERVER_ERR);
    await settle();

    expect(m.el.querySelector('.error-banner')).toBeNull();
    expect(button(m.el, 'ดูตัวอย่าง 500,000').disabled).toBe(false);
    expect(text(m.el)).toContain(formatMoney('88888.88'));
  });

  it('row 4: Tick reads send the held since and changed:false keeps the data even after a stale read resolves', async () => {
    const m = mount({ branch: 'MAIN' });
    await settle();

    const staleToggleRead = deferred<Reply>();
    let currentVersion = 25;
    server.cashHandler = (params) => {
      if (params.get('all') === 'true') {
        return staleToggleRead.promise;
      }
      if (params.get('since') === String(currentVersion)) {
        return { status: 200, body: { version: currentVersion, changed: false } };
      }
      return {
        status: 200,
        body: {
          version: currentVersion,
          changed: true,
          data: makeCash({ remaining: '54321.00', includeZeroJpy: false }),
        },
      };
    };

    const checkbox = m.el.querySelector<HTMLInputElement>(
      '.stock-filter input[type="checkbox"]',
    )!;
    // Toggle ON (held in flight) then OFF (resolves with version 25 and remaining 54,321.00)
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();

    checkbox.checked = false;
    checkbox.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();
    expect(text(m.el)).toContain(formatMoney('54321.00'));

    // Stale all=true read resolves with version 99 and different figure
    staleToggleRead.resolve({
      status: 200,
      body: {
        version: 99,
        changed: true,
        data: makeCash({ remaining: '11111.11', includeZeroJpy: true }),
      },
    });
    await settle();

    // Next tick must send held since=25 (not 99), and changed:false keeps 54,321.00
    requestRefresh();
    await settle();

    const cashReads = server.reads(CASH);
    const tickRead = cashReads[cashReads.length - 1]!;
    expect(tickRead.params.get('since')).toBe('25');
    expect(text(m.el)).toContain(formatMoney('54321.00'));
    expect(text(m.el)).not.toContain(formatMoney('11111.11'));
  });

  it('row 5: A failed read recovers on the next refresh and ignores a stale failure landing after recovery', async () => {
    const m = mount({ branch: 'MAIN' });
    await settle();

    // First refresh fails immediately
    server.cashHandler = () => SERVER_ERR;
    requestRefresh();
    await settle();
    expect(m.el.querySelector('.error-banner')?.textContent).toContain('เซิร์ฟเวอร์ขัดข้อง');
    expect(button(m.el, 'ดูตัวอย่าง 500,000').disabled).toBe(true);

    // Start an in-flight toggle read that will also fail late, then trigger refresh recovery
    const staleFailure = deferred<Reply>();
    let recovered = false;
    server.cashHandler = () => {
      if (!recovered) {
        recovered = true;
        return staleFailure.promise;
      }
      return {
        status: 200,
        body: {
          version: 7,
          changed: true,
          data: makeCash({ remaining: '12345.67' }),
        },
      };
    };

    const checkbox = m.el.querySelector<HTMLInputElement>(
      '.stock-filter input[type="checkbox"]',
    )!;
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();

    // Recovery refresh (readAll(true)) succeeds with no since
    requestRefresh();
    await settle();

    // Now the stale failure from before recovery rejects/resolves 500
    staleFailure.resolve(SERVER_ERR);
    await settle();

    const cashReads = server.reads(CASH);
    const recoveryRead = cashReads[cashReads.length - 1]!;
    expect(recoveryRead.params.has('since')).toBe(false);
    expect(m.el.querySelector('.error-banner')).toBeNull();
    expect(text(m.el)).toContain(formatMoney('12345.67'));
    expect(button(m.el, 'ดูตัวอย่าง 500,000').disabled).toBe(false);
  });

  it('edge case: A branch switch while readAll is half done applies nothing from the old generation', async () => {
    const mainMoves = deferred<Reply>();
    const mainShifts = deferred<Reply>();

    server.movesHandler = (params) => {
      if (params.get('branch') === 'MAIN') {
        return mainMoves.promise;
      }
      return {
        status: 200,
        body: { version: 1, changed: true, data: makeMovements('โอนเงิน SILOM') },
      };
    };
    server.shiftsHandler = (params) => {
      if (params.get('branch') === 'MAIN') {
        return mainShifts.promise;
      }
      return {
        status: 200,
        body: { version: 1, changed: true, data: makeShifts('S-SILOM') },
      };
    };

    // Start mounting on MAIN: admin_cash finishes, while movements and shifts are still in flight
    const m = mount({ branch: 'MAIN' });
    await settle();

    // Switch to SILOM while MAIN's readAll is half done
    m.setBranch('SILOM');
    await settle();
    expect(text(m.el)).toContain('โอนเงิน SILOM');
    expect(text(m.el)).toContain('S-SILOM');

    // Now MAIN's in-flight movements and shifts resolve
    mainMoves.resolve({
      status: 200,
      body: { version: 1, changed: true, data: makeMovements('ส่งเซฟ MAIN') },
    });
    mainShifts.resolve({
      status: 200,
      body: { version: 1, changed: true, data: makeShifts('S-MAIN') },
    });
    await settle();

    expect(text(m.el)).toContain('โอนเงิน SILOM');
    expect(text(m.el)).not.toContain('ส่งเซฟ MAIN');
    expect(text(m.el.querySelector('.shifts-table'))).toContain('S-SILOM');
    expect(text(m.el.querySelector('.shifts-table'))).not.toContain('S-MAIN');
  });

  it('rework (a): ticking show-all while SILOM readAll is waiting keeps SILOM movements and shifts and drops older branch answers', async () => {
    const asokMoves = deferred<Reply>();
    const asokShifts = deferred<Reply>();
    const silomMoves = deferred<Reply>();
    const silomShifts = deferred<Reply>();

    server.movesHandler = (params) => {
      const b = params.get('branch');
      if (b === 'ASOK') return asokMoves.promise;
      if (b === 'SILOM') return silomMoves.promise;
      return {
        status: 200,
        body: { version: 1, changed: true, data: makeMovements('ส่งเซฟ MAIN') },
      };
    };
    server.shiftsHandler = (params) => {
      const b = params.get('branch');
      if (b === 'ASOK') return asokShifts.promise;
      if (b === 'SILOM') return silomShifts.promise;
      return {
        status: 200,
        body: { version: 1, changed: true, data: makeShifts('S-MAIN') },
      };
    };

    const m = mount({ branch: 'MAIN' });
    await settle();
    expect(text(m.el)).toContain('ส่งเซฟ MAIN');

    // Switch through ASOK (held) to SILOM (held)
    m.setBranch('ASOK');
    await settle();
    m.setBranch('SILOM');
    await settle();

    // (a) Tick show-all while SILOM's movements and shifts are still in flight
    const checkbox = m.el.querySelector<HTMLInputElement>(
      '.stock-filter input[type="checkbox"]',
    )!;
    checkbox.checked = true;
    checkbox.dispatchEvent(new Event('change', { bubbles: true }));
    await settle();

    // Release SILOM's movements and shifts: must be applied (not dropped by show-all click)
    silomMoves.resolve({
      status: 200,
      body: { version: 1, changed: true, data: makeMovements('โอนเงิน SILOM') },
    });
    silomShifts.resolve({
      status: 200,
      body: { version: 1, changed: true, data: makeShifts('S-SILOM') },
    });
    await settle();

    expect(text(m.el)).toContain('โอนเงิน SILOM');
    expect(text(m.el)).not.toContain('ส่งเซฟ MAIN');
    expect(text(m.el.querySelector('.shifts-table'))).toContain('S-SILOM');

    // Release older ASOK movements and shifts: must be ignored
    asokMoves.resolve({
      status: 200,
      body: { version: 1, changed: true, data: makeMovements('โอนเงิน ASOK') },
    });
    asokShifts.resolve({
      status: 200,
      body: { version: 1, changed: true, data: makeShifts('S-ASOK') },
    });
    await settle();

    expect(text(m.el)).toContain('โอนเงิน SILOM');
    expect(text(m.el)).not.toContain('โอนเงิน ASOK');
    expect(text(m.el.querySelector('.shifts-table'))).toContain('S-SILOM');
    expect(text(m.el.querySelector('.shifts-table'))).not.toContain('S-ASOK');
  });

  it('rework (b): requestRefresh() while SILOM readAll is waiting keeps SILOM movements and shifts and drops older branch answers', async () => {
    const asokMoves = deferred<Reply>();
    const asokShifts = deferred<Reply>();
    const silomMoves = deferred<Reply>();
    const silomShifts = deferred<Reply>();

    server.movesHandler = (params) => {
      const b = params.get('branch');
      if (b === 'ASOK') return asokMoves.promise;
      if (b === 'SILOM') return silomMoves.promise;
      return {
        status: 200,
        body: { version: 1, changed: true, data: makeMovements('ส่งเซฟ MAIN') },
      };
    };
    server.shiftsHandler = (params) => {
      const b = params.get('branch');
      if (b === 'ASOK') return asokShifts.promise;
      if (b === 'SILOM') return silomShifts.promise;
      return {
        status: 200,
        body: { version: 1, changed: true, data: makeShifts('S-MAIN') },
      };
    };

    const m = mount({ branch: 'MAIN' });
    await settle();
    expect(text(m.el)).toContain('ส่งเซฟ MAIN');

    // Switch through ASOK (held) to SILOM (held)
    m.setBranch('ASOK');
    await settle();
    m.setBranch('SILOM');
    await settle();

    // (b) Trigger requestRefresh() while SILOM's movements and shifts are still in flight
    requestRefresh();
    await settle();

    // Release SILOM's movements and shifts: must be applied (not dropped by tick readCash)
    silomMoves.resolve({
      status: 200,
      body: { version: 1, changed: true, data: makeMovements('โอนเงิน SILOM') },
    });
    silomShifts.resolve({
      status: 200,
      body: { version: 1, changed: true, data: makeShifts('S-SILOM') },
    });
    await settle();

    expect(text(m.el)).toContain('โอนเงิน SILOM');
    expect(text(m.el)).not.toContain('ส่งเซฟ MAIN');
    expect(text(m.el.querySelector('.shifts-table'))).toContain('S-SILOM');

    // Release older ASOK movements and shifts: must be ignored
    asokMoves.resolve({
      status: 200,
      body: { version: 1, changed: true, data: makeMovements('โอนเงิน ASOK') },
    });
    asokShifts.resolve({
      status: 200,
      body: { version: 1, changed: true, data: makeShifts('S-ASOK') },
    });
    await settle();

    expect(text(m.el)).toContain('โอนเงิน SILOM');
    expect(text(m.el)).not.toContain('โอนเงิน ASOK');
    expect(text(m.el.querySelector('.shifts-table'))).toContain('S-SILOM');
    expect(text(m.el.querySelector('.shifts-table'))).not.toContain('S-ASOK');
  });
});
