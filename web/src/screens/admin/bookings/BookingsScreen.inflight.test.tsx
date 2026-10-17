// BK-inflight (#2890): double-click protection in BookingsScreen dialogs (ticket §9 rows 1, 3, 5).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';
import { BookingsScreen } from './BookingsScreen';

const BOOKINGS = 'admin_bookings';
const SETTINGS = 'admin_booking_settings';
const BRANCHES = 'admin_branches';
const IGNORED = PAGE_WIDE;

interface Row {
  id: string;
  branchId: string;
  name: string;
  phone: string;
  member: boolean;
  type: 'BUY' | 'SELL';
  currency: string;
  amount: string;
  rate: string;
  status: string;
  createdAt: string;
  expiresAt: string | null;
  transactionRef: string | null;
  counterRate: string | null;
  counterPrincipal: string | null;
}

interface Settings {
  rules: { enabled: boolean; minutes: number };
  memberCount: number;
  publicUrl: string;
  counterRates: { currency: string; buy: string; sell: string }[];
  freshness?: { stale: boolean; sourceDate?: string; publishedAt?: string };
}

const PENDING_ROW: Row = {
  id: 'BK-p1',
  branchId: 'MAIN',
  name: 'Somchai Pending',
  phone: '081 234 5678',
  member: true,
  type: 'BUY',
  currency: 'USD',
  amount: '1000.00',
  rate: '36.5000',
  status: 'PENDING',
  createdAt: '2026-10-09T05:00:00.000Z',
  expiresAt: null,
  transactionRef: null,
  counterRate: '36.5000',
  counterPrincipal: '11111.11',
};

const CONFIRMED_ROW: Row = {
  id: 'BK-c1',
  branchId: 'MAIN',
  name: 'Anan Confirmed',
  phone: '02 111 2222',
  member: false,
  type: 'SELL',
  currency: 'EUR',
  amount: '250.50',
  rate: '40.1000',
  status: 'CONFIRMED',
  createdAt: '2026-10-09T05:00:00.000Z',
  expiresAt: '2026-10-09T05:30:02.000Z',
  transactionRef: null,
  counterRate: null,
  counterPrincipal: null,
};

function defaultSettings(): Settings {
  return {
    rules: { enabled: true, minutes: 30 },
    memberCount: 2,
    publicUrl: 'https://example.com/book',
    counterRates: [
      { currency: 'EUR', buy: '39.5000', sell: '40.1000' },
      { currency: 'USD', buy: '36.2500', sell: '36.7500' },
    ],
    freshness: { stale: false, sourceDate: '2026-10-14', publishedAt: '10:00' },
  };
}

interface Sent {
  method: string;
  path: string;
  query: URLSearchParams;
  body: { action?: string; payload?: Record<string, unknown> } | undefined;
  headers: Record<string, string>;
}

type Reply = { status: number; body: unknown };

interface Server {
  version: number;
  rows: Row[];
  settings: Settings;
  read?: (view: string, query: URLSearchParams) => Promise<Reply | undefined> | Reply | undefined;
  post: (action: string, payload: Record<string, unknown>) => Promise<Reply> | Reply;
}

let server: Server;
let sent: Sent[];
let container: HTMLDivElement;
let dispose: (() => void) | undefined;

function createDeferred<T = void>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function respond(r: Reply): Response {
  return new Response(JSON.stringify(r.body), {
    status: r.status,
    headers: { 'content-type': 'application/json' },
  });
}

function headersOf(init?: RequestInit): Record<string, string> {
  const out: Record<string, string> = {};
  const h = init?.headers;
  if (!h) return out;
  new Headers(h).forEach((v, k) => {
    out[k.toLowerCase()] = v;
  });
  return out;
}

async function fakeFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = new URL(String(input), 'http://desk.test');
  const method = (init?.method ?? 'GET').toUpperCase();
  if (IGNORED.includes(url.pathname)) {
    return respond({
      status: 200,
      body: { version: 1, changed: true, data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN' } },
    });
  }
  const body = init?.body ? JSON.parse(String(init.body)) : undefined;
  sent.push({ method, path: url.pathname, query: url.searchParams, body, headers: headersOf(init) });

  if (method === 'GET' && url.pathname.startsWith('/api/v3/views/')) {
    const view = url.pathname.slice('/api/v3/views/'.length);
    const custom = server.read ? await server.read(view, url.searchParams) : undefined;
    if (custom) return respond(custom);
    const since = url.searchParams.get('since');
    if (since !== null && Number(since) === server.version && !url.searchParams.has('before')) {
      return respond({ status: 200, body: { version: server.version, changed: false } });
    }
    if (view === BOOKINGS) {
      return respond({ status: 200, body: { version: server.version, changed: true, data: server.rows } });
    }
    if (view === SETTINGS) {
      return respond({ status: 200, body: { version: server.version, changed: true, data: server.settings } });
    }
    if (view === BRANCHES) {
      return respond({
        status: 200,
        body: {
          version: server.version,
          changed: true,
          data: { branches: [{ id: 'MAIN', name: 'Main Branch' }] },
        },
      });
    }
  }

  if (method === 'POST' && url.pathname === '/api/v2/bookings') {
    const reply = await server.post(body?.action ?? '', body?.payload ?? {});
    return respond(reply);
  }

  return respond({ status: 404, body: { error: { code: 'notFound', message: `no ${method} ${url.pathname}` } } });
}

function reads(view: string): Sent[] {
  return sent.filter((s) => s.method === 'GET' && s.path === `/api/v3/views/${view}`);
}

function posts(): Sent[] {
  return sent.filter((s) => s.method === 'POST');
}

function norm(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

function visible(el: Element | null | undefined): boolean {
  if (!el || !el.isConnected) return false;
  for (let e: Element | null = el; e; e = e.parentElement) {
    if (e.hasAttribute('hidden')) return false;
    if (e instanceof HTMLElement && e.style.display === 'none') return false;
  }
  return true;
}

function theDialog(): HTMLDialogElement | null {
  const d = container.querySelector('dialog');
  return d && d.open ? d : null;
}

function openDialogEl(): HTMLDialogElement {
  const d = theDialog();
  expect(d, 'a dialog is open').not.toBeNull();
  return d!;
}

function inMenu<T extends Element>(selector: string): T[] {
  return [...container.querySelectorAll<T>(selector)].filter((e) => !e.closest('dialog') && visible(e));
}

function buttonsIn(root: ParentNode, text: string): HTMLButtonElement[] {
  return [...root.querySelectorAll('button')].filter((b) => norm(b.textContent) === text && visible(b));
}

function menuButton(text: string): HTMLButtonElement {
  const found = inMenu<HTMLButtonElement>('button').filter((b) => norm(b.textContent) === text);
  expect(found.length, `one menu button "${text}"`).toBe(1);
  return found[0]!;
}

function dialogButton(text: string): HTMLButtonElement {
  const found = buttonsIn(openDialogEl(), text);
  expect(found.length, `one dialog button "${text}"`).toBe(1);
  return found[0]!;
}

function rowsShown(): HTMLTableRowElement[] {
  return inMenu<HTMLTableRowElement>('tbody tr');
}

function rowOf(id: string): HTMLTableRowElement {
  const found = rowsShown().filter((r) => norm(r.textContent).includes(id));
  expect(found.length, `one row for ${id}`).toBe(1);
  return found[0]!;
}

function toastText(): string | null {
  const status = [...container.querySelectorAll('[role="status"]')].find((s) => s.classList.contains('toast'));
  if (!status || !visible(status)) return null;
  const t = norm(status.textContent);
  return t === '' ? null : t;
}

function setValue(el: HTMLInputElement | HTMLSelectElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

function mount(opts: { branch?: string; shown?: boolean } = {}) {
  const [branch, setBranch] = createSignal(opts.branch ?? 'MAIN');
  const [shown, setShown] = createSignal(opts.shown ?? true);
  const period = () => ({ from: '', to: '' });
  container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(
    () =>
      (
        <>
          <BookingsScreen branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    container,
  );
  return { setBranch, setShown };
}

async function opened(opts: { branch?: string } = {}) {
  const handles = mount(opts);
  await vi.waitFor(() => {
    expect(reads(BOOKINGS).length).toBeGreaterThan(0);
    expect(reads(SETTINGS).length).toBeGreaterThan(0);
    expect(rowsShown().length).toBe(server.rows.length);
  });
  return handles;
}

describe('BookingsScreen in-flight double-click protection (#2890)', () => {
  beforeEach(() => {
    setLang('th');
    sent = [];
    server = {
      version: 1,
      rows: [PENDING_ROW, CONFIRMED_ROW],
      settings: defaultSettings(),
      post: () => ({ status: 200, body: { ok: true } }),
    };
    vi.stubGlobal('fetch', fakeFetch);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    container?.remove();
    closeDialog();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('row 1: Click "Confirm lock rate" twice before the answer -> exactly one POST with action: confirm; button is disabled while in flight', async () => {
    const deferred = createDeferred<Reply>();
    server.post = () => deferred.promise;

    await opened();
    const row = rowOf('BK-p1');
    const confirmBtn = buttonsIn(row, 'ยืนยันเรทและเวลา')[0]!;
    confirmBtn.click();
    await vi.waitFor(() => expect(theDialog()).not.toBeNull());

    const expiryInput = openDialogEl().querySelector<HTMLInputElement>('input[type="datetime-local"]')!;
    expect(expiryInput).not.toBeNull();
    setValue(expiryInput, '2026-10-20T10:00');

    const lockRateBtn = dialogButton('ยืนยันล็อกเรท');
    lockRateBtn.click();

    // Check while in-flight
    expect(lockRateBtn.disabled).toBe(true);

    // Try clicking again
    lockRateBtn.click();

    // Still only 1 POST initiated
    const confirmPosts = posts().filter((p) => p.body?.action === 'confirm');
    expect(confirmPosts).toHaveLength(1);

    // Resolve write
    deferred.resolve({ status: 200, body: { ok: true } });
    await vi.waitFor(() => expect(theDialog()).toBeNull());

    // After resolution, still exactly 1 POST
    expect(posts().filter((p) => p.body?.action === 'confirm')).toHaveLength(1);
  });

  it('row 2: Confirm succeeds / fails -> succeeds: dialog closes and list is read again; fails: error shows in dialog and button is enabled again so retry works', async () => {
    // 2a. Confirm succeeds
    await opened();
    const row = rowOf('BK-p1');
    const confirmBtn = buttonsIn(row, 'ยืนยันเรทและเวลา')[0]!;
    confirmBtn.click();
    await vi.waitFor(() => expect(theDialog()).not.toBeNull());

    setValue(openDialogEl().querySelector<HTMLInputElement>('input[type="datetime-local"]')!, '2026-10-20T10:00');
    const initialReads = reads(BOOKINGS).length;
    dialogButton('ยืนยันล็อกเรท').click();
    await vi.waitFor(() => expect(theDialog()).toBeNull());
    await vi.waitFor(() => expect(reads(BOOKINGS).length).toBeGreaterThan(initialReads));

    // 2b. Confirm fails, shows error in dialog, button enabled, retry works
    server.post = () => ({ status: 400, body: { error: 'เรทเปลี่ยน กรุณาโหลดใหม่' } });
    buttonsIn(rowOf('BK-p1'), 'ยืนยันเรทและเวลา')[0]!.click();
    await vi.waitFor(() => expect(theDialog()).not.toBeNull());
    setValue(openDialogEl().querySelector<HTMLInputElement>('input[type="datetime-local"]')!, '2026-10-20T10:00');

    const lockRateBtn = dialogButton('ยืนยันล็อกเรท');
    lockRateBtn.click();

    await vi.waitFor(() => expect(openDialogEl().querySelector('.error')?.textContent).toContain('เรทเปลี่ยน'));
    expect(lockRateBtn.disabled).toBe(false);

    // Retry succeeds
    server.post = () => ({ status: 200, body: { ok: true } });
    lockRateBtn.click();
    await vi.waitFor(() => expect(theDialog()).toBeNull());
  });

  it('row 3: Click "Confirm cancel" twice before the answer -> exactly one POST with action: cancel; button is disabled while in flight; on success no error toast appears', async () => {
    const deferred = createDeferred<Reply>();
    server.post = () => deferred.promise;

    await opened();
    const row = rowOf('BK-c1');
    const cancelBtn = buttonsIn(row, 'ยกเลิก')[0]!;
    cancelBtn.click();
    await vi.waitFor(() => expect(theDialog()).not.toBeNull());

    const confirmCancelBtn = dialogButton('ยืนยันยกเลิก');
    confirmCancelBtn.click();

    // Check while in-flight
    expect(confirmCancelBtn.disabled).toBe(true);

    // Try clicking again
    confirmCancelBtn.click();

    // Still only 1 POST initiated
    const cancelPosts = posts().filter((p) => p.body?.action === 'cancel');
    expect(cancelPosts).toHaveLength(1);

    // Resolve write
    deferred.resolve({ status: 200, body: { ok: true } });
    await vi.waitFor(() => expect(theDialog()).toBeNull());

    // After resolution, still exactly 1 POST and no error toast appears
    expect(posts().filter((p) => p.body?.action === 'cancel')).toHaveLength(1);
    expect(toastText()).toBeNull();
  });

  it('row 4: Cancel fails -> the dialog closes and the error toast is shown', async () => {
    server.post = () => ({ status: 400, body: { error: 'การจองนี้ถูกยกเลิกไปแล้ว' } });

    await opened();
    const row = rowOf('BK-c1');
    const cancelBtn = buttonsIn(row, 'ยกเลิก')[0]!;
    cancelBtn.click();
    await vi.waitFor(() => expect(theDialog()).not.toBeNull());

    const confirmCancelBtn = dialogButton('ยืนยันยกเลิก');
    confirmCancelBtn.click();

    await vi.waitFor(() => expect(theDialog()).toBeNull());
    await vi.waitFor(() => expect(toastText()).toBe('การจองนี้ถูกยกเลิกไปแล้ว'));
  });

  it('row 5: Click "Save" in the settings dialog twice before the answer -> exactly one POST with action: settings; button is disabled while in flight', async () => {
    const deferred = createDeferred<Reply>();
    server.post = () => deferred.promise;

    await opened();
    const settingsBtn = menuButton('ตั้งค่าการจอง');
    settingsBtn.click();
    await vi.waitFor(() => expect(theDialog()).not.toBeNull());

    const saveBtn = dialogButton('บันทึก');
    saveBtn.click();

    // Check while in-flight
    expect(saveBtn.disabled).toBe(true);

    // Try clicking again
    saveBtn.click();

    // Still only 1 POST initiated
    const settingsPosts = posts().filter((p) => p.body?.action === 'settings');
    expect(settingsPosts).toHaveLength(1);

    // Resolve write
    deferred.resolve({ status: 200, body: { ok: true } });
    await vi.waitFor(() => expect(theDialog()).toBeNull());

    // After resolution, still exactly 1 POST
    expect(posts().filter((p) => p.body?.action === 'settings')).toHaveLength(1);
  });

  it('row 6: Settings save succeeds / fails -> succeeds: dialog closes and settings are read again; fails: error shows in dialog and Save is enabled again', async () => {
    // 6a. Settings save succeeds
    await opened();
    menuButton('ตั้งค่าการจอง').click();
    await vi.waitFor(() => expect(theDialog()).not.toBeNull());

    const initialSettingsReads = reads(SETTINGS).length;
    dialogButton('บันทึก').click();
    await vi.waitFor(() => expect(theDialog()).toBeNull());
    await vi.waitFor(() => expect(reads(SETTINGS).length).toBeGreaterThan(initialSettingsReads));

    // 6b. Settings save fails
    server.post = () => ({ status: 400, body: { error: 'ค่าไม่ถูกต้อง' } });
    menuButton('ตั้งค่าการจอง').click();
    await vi.waitFor(() => expect(theDialog()).not.toBeNull());

    const saveBtn = dialogButton('บันทึก');
    saveBtn.click();

    await vi.waitFor(() => expect(openDialogEl().querySelector('.error')?.textContent).toContain('ค่าไม่ถูกต้อง'));
    expect(saveBtn.disabled).toBe(false);

    // Retry succeeds
    server.post = () => ({ status: 200, body: { ok: true } });
    saveBtn.click();
    await vi.waitFor(() => expect(theDialog()).toBeNull());
  });
});
