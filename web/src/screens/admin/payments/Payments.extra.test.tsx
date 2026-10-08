// V2-payments (#470): extra tests for controller decisions
// Decision 3: cancelled branch switch keeps form and notice; save in-flight waits before switching

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { menu } from './menu';

interface Row {
  id: string;
  enabled: boolean;
  buy: boolean;
  sell: boolean;
  account: string;
  instruction: string;
}

interface Entry {
  version: number;
  actor: string;
  createdAt: string;
  beforeMethods: Row[];
  afterMethods: Row[];
}

interface Settings {
  version: number;
  branchId: string;
  settingsVersion: number;
  methods: Row[];
  updatedAt: string | null;
  updatedBy: string | null;
  canEdit: boolean;
  history: Entry[];
}

interface Read {
  view: string;
  branch: string | null;
  since: string | null;
}

interface Post {
  method: string;
  marker: string | null;
  path: string;
  body: unknown;
}

const PAGE_WIDE = ['/api/v3/views/me', '/api/v3/views/kiosk_notifications'];

const EDITED_AT = '2026-10-07T20:30:00.000Z';
const SAVED_AT = '2026-10-08T05:15:00.000Z';

function defaults(): Row[] {
  return [
    { id: 'Cash', enabled: true, buy: true, sell: true, account: '', instruction: '' },
    { id: 'Credit Card', enabled: true, buy: false, sell: true, account: '', instruction: '' },
    { id: 'Alipay', enabled: true, buy: false, sell: true, account: '', instruction: '' },
    { id: 'PromptPay', enabled: true, buy: false, sell: true, account: '', instruction: '' },
    { id: 'Bank Transfer', enabled: true, buy: true, sell: true, account: '', instruction: '' },
  ];
}

function mainRows(): Row[] {
  return [
    { id: 'Cash', enabled: true, buy: true, sell: true, account: 'Counter 1', instruction: 'Pay at counter 1' },
    { id: 'Credit Card', enabled: false, buy: false, sell: true, account: 'Card machine 2', instruction: 'Cards over 500 only' },
    { id: 'Alipay', enabled: true, buy: false, sell: true, account: 'Alipay till', instruction: 'Scan the shop code' },
    { id: 'PromptPay', enabled: true, buy: false, sell: true, account: 'PromptPay 0812345678', instruction: 'Show the slip' },
    { id: 'Bank Transfer', enabled: true, buy: true, sell: false, account: 'Company account', instruction: 'Transfer then wait' },
  ];
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

class Server {
  branches = [
    { id: 'MAIN', name: 'Main Desk' },
    { id: 'A1', name: 'Alpha Desk' },
  ];
  settings: Record<string, Settings> = {
    MAIN: {
      version: 41,
      branchId: 'MAIN',
      settingsVersion: 3,
      methods: mainRows(),
      updatedAt: EDITED_AT,
      updatedBy: 'somchai',
      canEdit: true,
      history: [],
    },
    A1: {
      version: 10,
      branchId: 'A1',
      settingsVersion: 0,
      methods: defaults(),
      updatedAt: null,
      updatedBy: null,
      canEdit: true,
      history: [],
    },
  };
  holdSave = false;
  private released: (() => void)[] = [];
  reads: Read[] = [];
  posts: Post[] = [];

  release(): void {
    const r = this.released;
    this.released = [];
    for (const f of r) f();
  }

  private wait(): Promise<void> {
    return new Promise((resolve) => this.released.push(resolve));
  }

  readsOf(view: string): Read[] {
    return this.reads.filter((r) => r.view === view);
  }

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const raw = String(input);
    const url = new URL(raw, 'http://desk.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method === 'GET' && PAGE_WIDE.includes(url.pathname)) {
      return json({ version: 1, changed: false });
    }
    const view = url.pathname.startsWith('/api/v3/views/') ? url.pathname.slice('/api/v3/views/'.length) : null;
    if (method === 'GET' && view === 'admin_payments') {
      const branch = url.searchParams.get('branch');
      const since = url.searchParams.get('since');
      this.reads.push({ view, branch, since });
      const s = this.settings[branch ?? 'MAIN'];
      if (!s) return json({ error: { code: 'noSuchBranch', message: 'not found' } }, 400);
      if (since !== null && since === String(s.version)) return json({ version: s.version, changed: false });
      const { version, ...data } = s;
      return json({ version, changed: true, data: JSON.parse(JSON.stringify(data)) });
    }
    if (method === 'GET' && view === 'admin_branches') {
      this.reads.push({ view, branch: url.searchParams.get('branch'), since: url.searchParams.get('since') });
      return json({ version: 1, changed: true, data: { branches: this.branches.map((b) => ({ ...b })) } });
    }
    if (method === 'POST' && url.pathname === '/api/payment-methods') {
      const headers = new Headers(init?.headers);
      const body = JSON.parse(String(init?.body ?? '{}')) as { action: string; payload: { branchId: string; version: number; methods: Row[] } };
      this.posts.push({ method, marker: headers.get('x-xchange-request'), path: url.pathname, body });
      if (this.holdSave) await this.wait();
      const s = this.settings[body.payload.branchId]!;
      s.version += 1;
      s.settingsVersion += 1;
      s.methods = body.payload.methods.map((r) => ({ ...r }));
      s.updatedAt = SAVED_AT;
      s.updatedBy = 'owner';
      return json({
        branchId: s.branchId,
        version: s.settingsVersion,
        methods: s.methods,
        updatedAt: s.updatedAt,
        updatedBy: s.updatedBy,
        canEdit: true,
        history: [],
      });
    }
    return json({ error: 'not found' }, 404);
  };
}

let server: Server;
let dispose: (() => void) | undefined;
let container: HTMLDivElement | undefined;

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

interface Mounted {
  setBranch: (b: string) => void;
  setShown: (s: boolean) => void;
}

async function mount(opts: { branch?: string; shown?: boolean } = {}): Promise<Mounted> {
  const [branch, setBranch] = createSignal(opts.branch ?? 'MAIN');
  const [shown, setShown] = createSignal(opts.shown ?? true);
  const period: Accessor<{ from: string; to: string }> = () => ({ from: '', to: '' });
  container = document.createElement('div');
  document.body.appendChild(container);
  const Component = menu.Component;
  dispose = render((): JSX.Element => <Component branch={branch} period={period} shown={shown} />, container);
  await settle();
  return { setBranch, setShown };
}

function page(): HTMLElement {
  expect(container, 'the menu is mounted').toBeDefined();
  return container!;
}

function words(): string {
  return (page().textContent ?? '').replace(/\s+/g, ' ').trim();
}

function button(root: ParentNode, text: string): HTMLButtonElement {
  const all = Array.from(root.querySelectorAll<HTMLButtonElement>('button'));
  const found = all.filter((b) => (b.textContent ?? '').replace(/\s+/g, ' ').trim() === text);
  expect(found.length, `one button "${text}"`).toBe(1);
  return found[0]!;
}

function card(id: string): HTMLElement {
  const found = Array.from(page().querySelectorAll<HTMLElement>('[data-payment-id]')).filter((e) => e.dataset.paymentId === id);
  expect(found.length, `one card data-payment-id="${id}"`).toBe(1);
  return found[0]!;
}

function cardHeading(id: string): string {
  const h3 = card(id).querySelector('h3');
  return (h3?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function historyRows(): string[][] {
  return Array.from(page().querySelectorAll<HTMLTableRowElement>('.xc-payment-history-table tbody tr')).map((r) =>
    Array.from(r.cells).map((c) => (c.textContent ?? '').replace(/\s+/g, ' ').trim()),
  );
}

function control(root: ParentNode, start: string): HTMLInputElement {
  const labels = Array.from(root.querySelectorAll<HTMLLabelElement>('label')).filter((l) => {
    const copy = l.cloneNode(true) as HTMLElement;
    for (const c of Array.from(copy.querySelectorAll('input, select, textarea, button'))) c.remove();
    return (copy.textContent ?? '').replace(/\s+/g, ' ').trim().startsWith(start);
  });
  expect(labels.length, `one control labelled "${start}"`).toBe(1);
  const input = labels[0]!.querySelector('input');
  expect(input).not.toBeNull();
  return input!;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

beforeEach(() => {
  setLang('th');
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
});

afterEach(async () => {
  server.release();
  await settle();
  dispose?.();
  dispose = undefined;
  container?.remove();
  container = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  setLang('th');
});

describe('V2-payments extra tests: controller decisions (#470 §12; decision 3)', () => {
  it('cancelled branch switch keeps previous branch form, edits and notice; discard then switches and reads new branch', async () => {
    const m = await mount();
    type(control(card('Cash'), 'ชื่อบัญชี / จุดรับ–จ่าย'), 'Unsaved on MAIN');
    await settle();

    const confirm = vi.fn(() => false);
    vi.stubGlobal('confirm', confirm);

    m.setBranch('A1');
    await settle();

    expect(confirm).toHaveBeenCalledWith('เปลี่ยนสาขาและทิ้งการแก้ไขวิธีชำระเงินที่ยังไม่บันทึก?');
    // A1 was not read
    expect(server.readsOf('admin_payments').filter((r) => r.branch === 'A1')).toEqual([]);
    // Notice naming the previous branch is shown
    expect(words()).toContain('กำลังแสดงการแก้ไขของสาขา Main Desk');
    expect(words()).toContain('ช่องทางของ Main Desk');
    expect(control(card('Cash'), 'ชื่อบัญชี / จุดรับ–จ่าย').value).toBe('Unsaved on MAIN');
    expect(words()).toContain('มีการแก้ไขที่ยังไม่บันทึก');

    // User discards: drops edits, clears notice, switches to A1 and reads A1
    button(page(), 'ยกเลิกการแก้ไข').click();
    await settle();

    expect(server.readsOf('admin_payments').filter((r) => r.branch === 'A1').length).toBeGreaterThanOrEqual(1);
    expect(words()).toContain('ช่องทางของ Alpha Desk');
    expect(words()).not.toContain('กำลังแสดงการแก้ไขของสาขา');
    expect(words()).toContain('บันทึกแล้ว / ยังไม่มีการแก้ไข');
  });

  it('cancelled branch switch saves to previous branch, then switches to and reads new branch', async () => {
    const m = await mount();
    type(control(card('Cash'), 'ชื่อบัญชี / จุดรับ–จ่าย'), 'Saved to MAIN');
    await settle();

    const confirm = vi.fn(() => false);
    vi.stubGlobal('confirm', confirm);

    m.setBranch('A1');
    await settle();

    expect(confirm).toHaveBeenCalled();
    expect(words()).toContain('กำลังแสดงการแก้ไขของสาขา Main Desk');

    // User saves: posts save to MAIN
    button(page(), 'บันทึกการตั้งค่า').click();
    await settle();

    expect(server.posts.length).toBe(1);
    expect((server.posts[0]!.body as { payload: { branchId: string } }).payload.branchId).toBe('MAIN');

    // After save completes, switches to A1 and reads A1
    expect(server.readsOf('admin_payments').filter((r) => r.branch === 'A1').length).toBeGreaterThanOrEqual(1);
    expect(words()).toContain('ช่องทางของ Alpha Desk');
    expect(words()).not.toContain('กำลังแสดงการแก้ไขของสาขา');
  });

  it('while save is in flight on branch change, the re-read for new branch waits until save answers', async () => {
    const confirm = vi.fn(() => true);
    vi.stubGlobal('confirm', confirm);

    const m = await mount();
    type(control(card('Cash'), 'ชื่อบัญชี / จุดรับ–จ่าย'), 'In flight on MAIN');
    await settle();

    server.holdSave = true;
    button(page(), 'บันทึกการตั้งค่า').click();
    await settle();

    // Now branch changes while save is in flight
    m.setBranch('A1');
    await settle();

    // While save is in flight, confirm must not be called, and A1 is NOT yet read
    expect(confirm).not.toHaveBeenCalled();
    expect(server.readsOf('admin_payments').filter((r) => r.branch === 'A1')).toEqual([]);

    // Now release save
    server.release();
    await settle();

    // After save finishes, A1 is read and shown
    expect(server.readsOf('admin_payments').filter((r) => r.branch === 'A1').length).toBeGreaterThanOrEqual(1);
    expect(words()).toContain('ช่องทางของ Alpha Desk');
  });

  it('shows English channel names in card headings and history changes column under English mode', async () => {
    setLang('en');
    server.settings.MAIN.history = [
      {
        version: 2,
        actor: 'owner',
        createdAt: '2026-10-08T05:15:00.000Z',
        beforeMethods: defaults(),
        afterMethods: defaults().map((m) => (m.id === 'Bank Transfer' ? { ...m, enabled: false } : m)),
      },
      {
        version: 1,
        actor: 'owner',
        createdAt: '2026-10-07T20:30:00.000Z',
        beforeMethods: defaults(),
        afterMethods: defaults().map((m) => (m.id === 'Cash' || m.id === 'PromptPay' ? { ...m, enabled: false } : m)),
      },
    ];

    await mount();

    // Card headings in English
    expect(cardHeading('Cash')).toBe('Cash');
    expect(cardHeading('Credit Card')).toBe('Credit Card');
    expect(cardHeading('Alipay')).toBe('Alipay');
    expect(cardHeading('PromptPay')).toBe('PromptPay');
    expect(cardHeading('Bank Transfer')).toBe('Bank Transfer');

    // History changes column (cell index 2) in English
    const rows = historyRows();
    expect(rows.length).toBe(2);
    expect(rows[0]![2]).toBe('Bank Transfer');
    expect(rows[1]![2]).toBe('Cash, PromptPay');
  });
});
