import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeAdminCashData,
  makeAdminShiftRow,
  type AdminShiftRow,
} from '../../../shared/testing/fixtures';
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

type Reply = { status: number; body: unknown };

class Server {
  version = 1;
  shifts: AdminShiftRow[] = [];

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input), 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    const json = (reply: Reply): Response =>
      new Response(JSON.stringify(reply.body), {
        status: reply.status,
        headers: { 'content-type': 'application/json' },
      });
    if (url.pathname === '/api/v3/views/me') {
      return json({
        status: 200,
        body: { version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' } },
      });
    }
    if (url.pathname === QUEUE_CARD_PATH) return json({ status: 200, body: queueCardUnchanged() });
    if (method === 'GET' && url.pathname === CASH) {
      return json({ status: 200, body: { version: this.version, changed: true, data: makeAdminCashData() } });
    }
    if (method === 'GET' && url.pathname === MOVES) {
      return json({ status: 200, body: { version: this.version, changed: true, data: [] } });
    }
    if (method === 'GET' && url.pathname === SHIFTS) {
      return json({ status: 200, body: { version: this.version, changed: true, data: this.shifts } });
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

describe('Cash shift list: shift name and business date (CS-16)', () => {
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

  it('U6: a row with shiftName "B" and businessDate "2026-11-04" renders the label in the shift list', async () => {
    server.shifts = [
      makeAdminShiftRow({
        id: 'S-0001',
        staff: 'EMP-007',
        shiftName: 'B',
        businessDate: '2026-11-04',
      }),
    ];

    const el = await mounted();
    const table = el.querySelector('.shifts-table');
    expect(table).not.toBeNull();

    const row = table?.querySelector('tbody tr');
    expect(row).not.toBeNull();

    const cells = [...(row?.querySelectorAll('td') ?? [])];
    expect(cells.length).toBeGreaterThanOrEqual(3);

    // openedBy cell is index 2
    const openedByCell = cells[2];
    expect(text(openedByCell)).toContain('EMP-007');
    expect(text(openedByCell)).toContain('B · 4/11');

    const label = openedByCell.querySelector('.shift-label');
    expect(label).not.toBeNull();
    expect(text(label)).toBe('B · 4/11');
  });

  it('U6b: a row with shiftName null renders no label in the shift list', async () => {
    server.shifts = [
      makeAdminShiftRow({
        id: 'S-0002',
        staff: 'EMP-002',
        shiftName: null,
        businessDate: '2026-11-04',
      }),
    ];

    const el = await mounted();
    const table = el.querySelector('.shifts-table');
    expect(table).not.toBeNull();

    const row = table?.querySelector('tbody tr');
    expect(row).not.toBeNull();

    const cells = [...(row?.querySelectorAll('td') ?? [])];
    const openedByCell = cells[2];
    expect(text(openedByCell)).toBe('EMP-002');

    const label = openedByCell.querySelector('.shift-label');
    expect(label).toBeNull();
  });
});
