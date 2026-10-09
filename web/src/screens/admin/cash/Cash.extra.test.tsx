import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { Toast } from '../parts/toast';
import type { AdminCashData } from '../../../shared/api/generated/admin_cash';
import type { AdminShiftRow } from '../../../shared/api/generated/admin_shifts';

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

const REGISTER = [
  'THB', 'USD', 'EUR', 'JPY', 'GBP', 'AUD', 'CAD', 'CHF', 'CNY', 'HKD',
  'SGD', 'KRW', 'TWD', 'NZD', 'INR', 'MYR', 'PHP', 'IDR', 'VND', 'AED',
  'SAR', 'QAR', 'KWD', 'BHD', 'OMR', 'JOD', 'ILS', 'TRY', 'RUB', 'ZAR',
  'BRL', 'MXN', 'PLN', 'SEK', 'NOK', 'DKK', 'HUF', 'CZK', 'RON', 'BGN',
  'HRK', 'RSD', 'ISK', 'UAH', 'KZT', 'EGP', 'MAD', 'NGN', 'KES', 'GHS',
  'TZS', 'UGX', 'PKR', 'BDT', 'LKR', 'NPR', 'MMK', 'KHR', 'LAK', 'BND',
  'MOP', 'MNT', 'KGS', 'UZS', 'TJS', 'TMT', 'AZN', 'GEL', 'AMD', 'MDL',
  'ALL', 'MKD', 'BAM', 'ARS', 'CLP', 'COP', 'PEN', 'UYU', 'BOB', 'PYG',
  'CRC', 'DOP', 'GTQ', 'HNL', 'NIO', 'PAB', 'TTD', 'JMD', 'BBD', 'BSD',
  'FJD', 'PGK', 'WST', 'TOP', 'VUV', 'SBD', 'SCR', 'MUR', 'MVR', 'LBP',
  'DZD', 'TND', 'LYD', 'IQD', 'SYP', 'YER', 'SDG', 'SOS', 'ETB', 'RWF',
  'BIF', 'DJF', 'MZN', 'AOA', 'ZMW', 'MWK', 'SZL', 'LSL', 'BWP', 'NAD',
  'GMD', 'GNF', 'SLL', 'LRD', 'CVE', 'STN', 'KMF', 'MRU', 'AFN', 'IRR',
  'XAF', 'XOF', 'XCD', 'XPF', 'ANG', 'AWG', 'BMD', 'KYD', 'CUP', 'HTG',
  'SRD', 'GYD', 'FKP', 'GIP', 'SHP', 'BTN', 'CUC', 'ERN', 'SSP', 'VES',
  'ZWL', 'BYN', 'XDR',
];

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
}

interface Post {
  action: string;
  payload: Record<string, unknown>;
}

class TestServer {
  version = 1;
  shiftsPages: Record<string, AdminShiftRow[]> = {};
  shiftsError: { status: number; body: unknown } | null = null;
  posts: Post[] = [];
  cashData: AdminCashData = {
    shift: {
      id: 'S-0002',
      openedAt: '2026-10-06T08:00:00.000Z',
      staff: 'EMP-001',
    },
    stock: [
      { code: 'THB', opening: '10000.00', net: '0.00', expected: '10000.00' },
      { code: 'USD', opening: '500.00', net: '0.00', expected: '500.00' },
    ],
    stats: {
      opening: '10000.00',
      incoming: '0.00',
      outgoing: '0.00',
      remaining: '10000.00',
    },
    reference: {
      withReference: 0,
      withoutReference: 0,
      spread: '0.00',
      percent: '0.00',
    },
    currencies: REGISTER,
    lastCounted: null,
  };
  sent: Sent[] = [];

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input), 'http://localhost');
    const method = (init?.method ?? 'GET').toUpperCase();
    this.sent.push({ path: url.pathname, params: url.searchParams, method });
    const json = (status: number, body: unknown) =>
      new Response(JSON.stringify(body), {
        status,
        headers: { 'content-type': 'application/json' },
      });

    if (url.pathname === '/api/v3/views/me') {
      return json(200, {
        version: 1,
        changed: true,
        data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' },
      });
    }
    if (url.pathname === '/api/v3/views/kiosk_notifications') {
      return json(200, { version: 1, changed: false });
    }
    if (method === 'GET' && url.pathname === CASH) {
      return json(200, { version: this.version, changed: true, data: this.cashData });
    }
    if (method === 'GET' && url.pathname === MOVES) {
      return json(200, { version: this.version, changed: true, data: [] });
    }
    if (method === 'GET' && url.pathname === SHIFTS) {
      if (this.shiftsError) {
        return json(this.shiftsError.status, this.shiftsError.body);
      }
      const before = url.searchParams.get('before') ?? 'initial';
      const data = this.shiftsPages[before] ?? [];
      return json(200, { version: this.version, changed: true, data });
    }
    if (method === 'POST' && url.pathname === '/api/rpc') {
      const body = JSON.parse(String(init?.body ?? '{}')) as Post;
      this.posts.push(body);
      if (body.action === 'adminAbandonRequest') {
        return json(200, { ok: true, requestResolution: 'CANCELLED' });
      }
      return json(200, { ok: true });
    }
    return json(404, { error: 'not found' });
  };
}

let server: TestServer;
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
  const buttons = [...el.querySelectorAll<HTMLButtonElement>('button')].filter(
    (b) => text(b) === name,
  );
  expect(buttons.length, `one button ${JSON.stringify(name)}`).toBe(1);
  return buttons[0]!;
}

function dialog(): HTMLElement {
  const d = document.querySelector('dialog');
  expect(d?.open, 'dialog is open').toBe(true);
  return d!;
}

describe('V2-cash extra tests: keyset pagination, 153 register currencies, localized currency names', () => {
  beforeEach(() => {
    server = new TestServer();
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

  it('shifts list: loads older shifts keyset by before until server returns empty', async () => {
    server.shiftsPages['initial'] = [
      {
        id: 'S-0002',
        status: 'OPEN',
        openedAt: '2026-10-06T08:00:00.000Z',
        staff: 'EMP-001',
        note: '',
        difference: null,
        balances: [{ code: 'THB', expected: '10000.00', counted: null, difference: null }],
      },
    ];
    server.shiftsPages['S-0002'] = [
      {
        id: 'S-0001',
        status: 'CLOSED',
        openedAt: '2026-10-05T08:00:00.000Z',
        staff: 'EMP-002',
        note: '',
        difference: [],
        balances: [{ code: 'THB', expected: '5000.00', counted: '5000.00', difference: '0.00' }],
      },
    ];
    server.shiftsPages['S-0001'] = [];

    const m = mount();
    await settle();

    // Verify initial shift is displayed
    expect(text(m.el)).toContain('S-0002');
    expect(text(m.el)).not.toContain('S-0001');

    // Click "โหลดรายการก่อนหน้า"
    const loadBtn = button(m.el, 'โหลดรายการก่อนหน้า');
    loadBtn.click();
    await settle();

    // Verify before param was sent
    const beforeCalls = server.sent.filter(
      (s) => s.path === SHIFTS && s.params.get('before') === 'S-0002',
    );
    expect(beforeCalls.length).toBe(1);

    // Now both shifts are rendered
    expect(text(m.el)).toContain('S-0002');
    expect(text(m.el)).toContain('S-0001');

    // Click "โหลดรายการก่อนหน้า" again
    button(m.el, 'โหลดรายการก่อนหน้า').click();
    await settle();

    const secondBeforeCalls = server.sent.filter(
      (s) => s.path === SHIFTS && s.params.get('before') === 'S-0001',
    );
    expect(secondBeforeCalls.length).toBe(1);

    // Empty list returned, so "โหลดรายการก่อนหน้า" button is hidden
    const remainingButtons = [...m.el.querySelectorAll('button')].filter(
      (b) => text(b) === 'โหลดรายการก่อนหน้า',
    );
    expect(remainingButtons.length).toBe(0);
  });

  it('close shift dialog: renders a required input and POSTS a count for every register currency', async () => {
    const m = mount();
    await settle();

    button(m.el, 'ตรวจนับและปิดกะ').click();
    await settle();

    const d = dialog();
    const rows = d.querySelectorAll('.close-currency-row');
    expect(rows.length).toBe(153);

    for (const code of REGISTER) {
      const input = d.querySelector<HTMLInputElement>(`#close-${code}`);
      expect(input, `input for ${code} exists`).not.toBeNull();
      expect(input?.required, `input for ${code} is required`).toBe(true);
    }

    const form = d.querySelector('form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    const post = server.posts.find((p) => p.action === 'adminCloseShift');
    expect(post, 'adminCloseShift was posted').toBeDefined();
    const payload = post!.payload as { counted: Record<string, string> };
    expect(Object.keys(payload.counted).length, 'posts a count for every register currency').toBe(153);
    for (const code of REGISTER) {
      expect(code in payload.counted, `count posted for ${code}`).toBe(true);
      expect(payload.counted[code], `untouched count for ${code} sends "", not "0"`).toBe('');
    }
  });

  it('close shift dialog: untouched input sends blank string "", not "0"', async () => {
    const m = mount();
    await settle();

    button(m.el, 'ตรวจนับและปิดกะ').click();
    await settle();

    const d = dialog();
    const thbInput = d.querySelector<HTMLInputElement>('#close-THB')!;
    thbInput.value = '10000';
    thbInput.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();

    const form = d.querySelector('form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    const post = server.posts.find((p) => p.action === 'adminCloseShift');
    expect(post, 'adminCloseShift was posted').toBeDefined();
    const payload = post!.payload as { counted: Record<string, string> };
    expect(payload.counted['THB']).toBe('10000');
    expect(payload.counted['USD'], 'untouched count sends "", not "0"').toBe('');
    expect(payload.counted['EUR'], 'untouched count sends "", not "0"').toBe('');
    expect(payload.counted['USD']).not.toBe('0');
  });

  it('shifts list: shows errorText when load older read fails, does not swallow it', async () => {
    server.shiftsPages['initial'] = [
      {
        id: 'S-0002',
        status: 'OPEN',
        openedAt: '2026-10-06T08:00:00.000Z',
        staff: 'EMP-001',
        note: '',
        difference: null,
        balances: [{ code: 'THB', expected: '10000.00', counted: null, difference: null }],
      },
    ];

    const m = mount();
    await settle();

    expect(text(m.el)).toContain('S-0002');

    server.shiftsError = {
      status: 403,
      body: { error: { code: 'ownerOnly', message: 'refused by the server' } },
    };

    button(m.el, 'โหลดรายการก่อนหน้า').click();
    await settle();

    const expectedMsg = errorText('ownerOnly', 'refused by the server');
    expect(text(m.el)).toContain(expectedMsg);
  });

  it('open shift dialog: currency names in picker follow active lang', async () => {
    server.cashData = {
      ...server.cashData,
      shift: null,
      lastCounted: null,
    };

    // 1. Thai
    setLang('th');
    const m = mount();
    await settle();

    button(m.el, 'ตั้งเงินสดเริ่มต้น / เปิดกะ').click();
    await settle();

    const d = dialog();
    const search = d.querySelector<HTMLInputElement>('input[placeholder="เช่น USD, ดอลลาร์, EUR"]')!;
    search.value = 'USD';
    search.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();

    // In Thai, USD should include Thai currency name 'ดอลลาร์สหรัฐ'
    expect(text(d)).toContain('ดอลลาร์สหรัฐ');

    closeDialog();
    await settle();
    dispose?.();
    root?.remove();

    // 2. English
    setLang('en');
    mount();
    await settle();

    button(root, 'Set Opening Cash / Open Shift').click();
    await settle();

    const dEn = dialog();
    const searchEn = dEn.querySelector<HTMLInputElement>('input[placeholder="e.g. USD, Dollar, EUR"]')!;
    searchEn.value = 'USD';
    searchEn.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();

    // In English, USD should include 'US Dollar'
    expect(text(dEn)).toContain('US Dollar');
  });
});
