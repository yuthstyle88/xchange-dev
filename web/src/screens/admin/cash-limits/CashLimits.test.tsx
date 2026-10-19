// FE-admin (#3324), survey row A22: tests for the admin cash-limits screen (ledger-inventory.md §3.3, Q-2).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiClient } from '../../../shared/api/client';
import type { CashLimitsData } from '../../../shared/api/generated/cash_limits';
import type { Me } from '../../../shared/api/generated/common';
import { setLang } from '../../../shared/i18n';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import { AdminShell } from '../AdminShell';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { menus } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { menu } from './menu';
import * as texts from './i18n';

const MANAGE = 'CashLimitsManage';

function meWith(permissions: string[]): Me {
  return {
    id: 'u-fin',
    name: 'ฝ่ายการเงิน',
    roles: [],
    permissions,
    branch: 'MAIN',
    branchName: 'สำนักงานใหญ่',
    branches: [
      { id: 'MAIN', code: 'MAIN', name: 'สำนักงานใหญ่' },
      { id: 'B1', code: 'B1', name: 'สาขาหนึ่ง' },
    ],
  };
}

const NO_ROW: CashLimitsData = {
  branch: 'MAIN',
  countDifferenceLimitThb: null,
  updatedBy: null,
  updatedAt: null,
};

const ROW_500: CashLimitsData = {
  branch: 'MAIN',
  countDifferenceLimitThb: '500.00',
  updatedBy: 'u-admin',
  updatedAt: '2026-10-12T01:00:00Z',
};

const ROW_250: CashLimitsData = {
  branch: 'MAIN',
  countDifferenceLimitThb: '250.50',
  updatedBy: 'u-fin',
  updatedAt: '2026-10-12T02:00:00Z',
};

interface Sent {
  method: string;
  path: string;
  query: URLSearchParams;
  body: string | null;
  key: string | null;
}

type Reply = { status: number; body?: unknown } | (() => Promise<Response>);

class Server {
  me: Me = meWith([MANAGE]);
  readsByBranch: Record<string, CashLimitsData | (() => Promise<CashLimitsData>)> = {
    MAIN: NO_ROW,
  };
  saves: Reply[] = [{ status: 200, body: ROW_250 }];
  sent: Sent[] = [];
  private puts = 0;

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(raw, 'http://xc.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    const headers = new Headers(init?.headers);
    const body = typeof init?.body === 'string' ? init.body : null;
    this.sent.push({ method, path: u.pathname, query: u.searchParams, body, key: headers.get('idempotency-key') });
    const view = (data: unknown) => json(200, { version: 1, changed: true, data });
    if (method === 'GET') {
      switch (u.pathname) {
        case '/api/v3/views/me':
          return view(this.me);
        case QUEUE_CARD_PATH:
          return json(200, queueCardUnchanged());
        case '/api/v3/views/admin_branches':
          return view({
            branches: [
              { id: 'MAIN', name: 'สำนักงานใหญ่' },
              { id: 'B1', name: 'สาขาหนึ่ง' },
            ],
          });
        case '/api/v3/views/cash_limits': {
          const b = u.searchParams.get('branch') ?? 'MAIN';
          const handler = this.readsByBranch[b] ?? { ...NO_ROW, branch: b };
          const data = typeof handler === 'function' ? await handler() : handler;
          return view(data);
        }
        default:
          return view(null);
      }
    }
    if (method === 'PUT' && u.pathname === '/api/cash-limits') {
      const r = this.saves[Math.min(this.puts, this.saves.length - 1)]!;
      this.puts++;
      if (typeof r === 'function') return r();
      return json(r.status, r.body ?? {});
    }
    return json(404, { error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` } });
  };

  writes(): Sent[] {
    return this.sent.filter((s) => s.method === 'PUT' && s.path === '/api/cash-limits');
  }

  reads(): Sent[] {
    return this.sent.filter((s) => s.method === 'GET' && s.path === '/api/v3/views/cash_limits');
  }
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let server: Server;
let dispose: (() => void) | undefined;
let root: HTMLElement;
let setBranchSignal: Setter<string>;

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 12; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

async function mount(over: Partial<Pick<Server, 'readsByBranch' | 'saves'>> = {}): Promise<HTMLElement> {
  Object.assign(server, over);
  const [branch, setBranch] = createSignal('MAIN');
  setBranchSignal = setBranch;
  const [period] = createSignal({ from: '2026-10-12', to: '2026-10-12' });
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
  return root;
}

async function mountShell(permissions: string[]): Promise<HTMLElement> {
  server.me = meWith(permissions);
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(() => <AdminShell api={new ApiClient()} menus={menus} />, root);
  await settle();
  await settle();
  return root;
}

beforeEach(() => {
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
  vi.stubGlobal('matchMedia', (query: string) => ({
    media: query,
    matches: false,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    onchange: null,
    dispatchEvent: () => false,
  }));
  location.hash = '#cash-limits';
  setLang('th');
});

afterEach(async () => {
  await settle();
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  document.body.innerHTML = '';
  vi.useRealTimers();
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
});

const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const screen = (el: ParentNode) => el.querySelector<HTMLElement>('[data-section="cash-limits"]');
const limitInput = (el: ParentNode) => el.querySelector<HTMLInputElement>('input[name="countDifferenceLimitThb"]');
const noLimitBox = (el: ParentNode) => el.querySelector<HTMLInputElement>('input[name="noLimit"]');
const saveButton = (el: ParentNode) => el.querySelector<HTMLButtonElement>('button[data-action="save"]');

function typeInput(el: HTMLInputElement | null, value: string): void {
  expect(el, 'the limit input exists').not.toBeNull();
  el!.value = value;
  el!.dispatchEvent(new Event('input', { bubbles: true }));
  el!.dispatchEvent(new Event('change', { bubbles: true }));
}

describe('CashLimits (#3324)', () => {
  it('1: a branch with no row (countDifferenceLimitThb: null) shows the no-limit wording and no updatedBy', async () => {
    const el = await mount({
      readsByBranch: {
        MAIN: { branch: 'MAIN', countDifferenceLimitThb: null, updatedBy: 'u-ghost', updatedAt: null },
      },
    });
    const s = screen(el)!;
    expect(text(s)).toContain(texts.th.noLimit);
    expect(text(s)).not.toContain('u-ghost');
  });

  it('2: a branch with "500.00" shows 500.00, plus who and when it was set', async () => {
    const el = await mount({ readsByBranch: { MAIN: ROW_500 } });
    const s = screen(el)!;
    expect(text(s)).toContain('500.00');
    expect(text(s)).toContain('u-admin');
    expect(limitInput(s)?.value).toBe('500.00');
    expect(noLimitBox(s)?.checked).toBe(false);
  });

  it('3: type 250.50, save -> PUT /api/cash-limits body {branch, countDifferenceLimitThb: "250.50"}, toast "saved", shows the answer', async () => {
    const el = await mount({
      readsByBranch: { MAIN: NO_ROW },
      saves: [{ status: 200, body: ROW_250 }],
    });
    const s = screen(el)!;
    typeInput(limitInput(s), '250.50');
    saveButton(s)!.click();
    await settle();

    const w = server.writes();
    expect(w).toHaveLength(1);
    expect(JSON.parse(w[0]!.body ?? 'null')).toEqual({
      branch: 'MAIN',
      countDifferenceLimitThb: '250.50',
    });
    expect(text(document.body)).toContain(texts.th.saved);
    expect(text(s)).toContain('250.50');
    expect(text(s)).toContain('u-fin');
  });

  it('4: choose "no limit", save -> body countDifferenceLimitThb: null', async () => {
    const el = await mount({
      readsByBranch: { MAIN: ROW_500 },
      saves: [{ status: 200, body: NO_ROW }],
    });
    const s = screen(el)!;
    const box = noLimitBox(s)!;
    box.checked = true;
    box.dispatchEvent(new Event('change', { bubbles: true }));
    saveButton(s)!.click();
    await settle();

    const w = server.writes();
    expect(w).toHaveLength(1);
    expect(JSON.parse(w[0]!.body ?? 'null')).toEqual({
      branch: 'MAIN',
      countDifferenceLimitThb: null,
    });
    expect(text(s)).toContain(texts.th.noLimit);
  });

  it('5: server 400 invalidRequest on countDifferenceLimitThb (e.g. 1.234) shows refusal at the field and nothing else changes', async () => {
    const el = await mount({
      readsByBranch: { MAIN: ROW_500 },
      saves: [
        {
          status: 400,
          body: {
            error: {
              code: 'invalidRequest',
              field: 'countDifferenceLimitThb',
              message: 'bad decimal',
            },
          },
        },
      ],
    });
    const s = screen(el)!;
    typeInput(limitInput(s), '1.234');
    saveButton(s)!.click();
    await settle();

    const fieldErr = s.querySelector('[data-field-error="countDifferenceLimitThb"]');
    expect(fieldErr).not.toBeNull();
    expect(text(fieldErr)).toContain(texts.th['error.countDifferenceLimitThb']);
    expect(text(s.querySelector('[data-current-limit]'))).toContain('500.00');
    expect(limitInput(s)?.value).toBe('1.234');
  });

  it('6: switch branch re-reads cash_limits for the new branch and drops a late answer from the old branch', async () => {
    let resolveMain!: (d: CashLimitsData) => void;
    const mainLate = new Promise<CashLimitsData>((r) => {
      resolveMain = r;
    });
    const b1Row: CashLimitsData = {
      branch: 'B1',
      countDifferenceLimitThb: '800.00',
      updatedBy: 'u-b1',
      updatedAt: '2026-10-12T03:00:00Z',
    };

    const el = await mount({
      readsByBranch: {
        MAIN: () => mainLate,
        B1: b1Row,
      },
    });

    setBranchSignal('B1');
    await settle();
    const s = screen(el)!;
    expect(text(s)).toContain('800.00');
    expect(text(s)).toContain('u-b1');

    resolveMain({
      branch: 'MAIN',
      countDifferenceLimitThb: '999.00',
      updatedBy: 'u-stale',
      updatedAt: '2026-10-12T00:00:00Z',
    });
    await settle();

    expect(text(s)).toContain('800.00');
    expect(text(s)).not.toContain('999.00');
    expect(text(s)).not.toContain('u-stale');
    expect(server.reads().map((r) => r.query.get('branch'))).toEqual(['MAIN', 'B1']);
  });

  it('7: a user without CashLimitsManage does not see the menu listed, while a user with it sees it', async () => {
    const withoutEl = await mountShell(['CashManage', 'SystemSettings']);
    expect(withoutEl.querySelector('a[href="#cash-limits"]')).toBeNull();
    expect(screen(withoutEl)).toBeNull();

    dispose?.();
    root?.remove();

    const withEl = await mountShell([MANAGE]);
    expect(withEl.querySelector('a[href="#cash-limits"]')).not.toBeNull();
    expect(screen(withEl)).not.toBeNull();
  });

  it('8: a double click on save sends one write while in flight', async () => {
    let resolveSave!: (r: Response) => void;
    const slowSave = new Promise<Response>((r) => {
      resolveSave = r;
    });
    const el = await mount({
      readsByBranch: { MAIN: NO_ROW },
      saves: [() => slowSave],
    });
    const s = screen(el)!;
    typeInput(limitInput(s), '250.50');
    const btn = saveButton(s)!;
    btn.click();
    btn.click();
    expect(server.writes()).toHaveLength(1);

    resolveSave(json(200, ROW_250));
    await settle();
    expect(server.writes()).toHaveLength(1);
    expect(text(s)).toContain('250.50');
  });
});
