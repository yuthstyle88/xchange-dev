import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { requestRefresh } from '../parts/refresh';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}
interface TextsModule {
  th: Record<string, string>;
  en: Record<string, string>;
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });
const textFiles = import.meta.glob<TextsModule>('./i18n.ts', { eager: true });

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/reconciliation/menu.ts exists').toBeDefined();
  return m!.menu;
}

type K = 'note' | 'certify';

function say(key: K, lang: 'th' | 'en' = 'th'): string {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/reconciliation/i18n.ts exists').toBeDefined();
  const value = m![lang][key];
  expect(value, `admin.reconciliation i18n ${lang}.${key}`).toBeTruthy();
  return value!;
}

const VIEW = '/api/v3/views/finance_reconciliation';
const CERTIFY = '/api/v3/commands/reconciliation_certify';
const B01 = 'XC-B01';
const DAY = /^\d{4}-\d{2}-\d{2}$/;

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
  key: string | null;
  body: Record<string, any> | undefined;
}
interface Reply {
  status: number;
  body: unknown;
}

let sent: Sent[];
let hashes: Record<string, string>;
let certified: Record<string, { id: string; figuresHash: string; note: string }[]>;

function waiting(reason: string) {
  return { available: false, reason, data: null };
}

function sheetOf(branch: string, day: string) {
  const hash = hashes[branch]!;
  return {
    branch,
    date: day,
    transactions: {
      available: true,
      reason: null,
      data: {
        byMethod: [
          { method: 'Cash', buyThb: '1300.00', sellThb: '2000.00', fees: '30.00', bankFees: '0.00', count: 3 },
          { method: 'PromptPay', buyThb: '0.00', sellThb: '500.00', fees: '5.00', bankFees: '5.00', count: 1 },
        ],
        reversedThb: '250.00',
      },
    },
    counts: waiting('CA-3'),
    transfers: waiting('CA-5'),
    exceptions: {
      available: true,
      reason: null,
      data: [{ id: 'EX-77', position: 1, reviewed: false, summary: 'EX-77 short count' }],
    },
    integrity: waiting('LG-7'),
    reversalsPending: waiting('RV-1'),
    figuresHash: hash,
    certifications: [...(certified[branch] ?? [])].reverse().map((c) => ({
      id: c.id,
      certifiedBy: 'u-fin',
      certifiedAt: '2026-10-11T10:00:00Z',
      note: c.note,
      figuresHash: c.figuresHash,
      stale: c.figuresHash !== hash,
    })),
  };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function refusal(status: number, code: string, message: string): Reply {
  return { status, body: { error: { code, message } } };
}

function answer(s: Sent): Reply {
  const { path, params, method, body } = s;
  if (path === '/api/v3/views/me') {
    return {
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: { kind: 'staff', id: 'u-fin', role: 'FINANCE', roles: ['FINANCE'], branch: B01, name: 'Fin', username: 'fin' },
      },
    };
  }
  if (path === '/api/v3/views/kiosk_notifications') return { status: 200, body: { version: 1, changed: false } };
  if (path === VIEW && method === 'GET') {
    const branch = params.get('branch') ?? '';
    const day = params.get('date') ?? '';
    if (!(branch in hashes)) return refusal(400, 'noSuchBranch', 'no such branch (test)');
    if (!DAY.test(day)) return refusal(400, 'invalidRequest', 'bad date (test)');
    return { status: 200, body: { version: 1, changed: true, data: sheetOf(branch, day) } };
  }
  if (path === CERTIFY && method === 'POST') {
    const branch = String(body?.branch ?? '');
    if (body?.figuresHash !== hashes[branch]) {
      return refusal(409, 'staleVersion', 'The figures changed; read the sheet again (test)');
    }
    const row = { id: `C-${(certified[branch]?.length ?? 0) + 1}`, figuresHash: String(body?.figuresHash), note: String(body?.note ?? '') };
    (certified[branch] ??= []).push(row);
    return {
      status: 200,
      body: { id: row.id, certifiedBy: 'u-fin', certifiedAt: '2026-10-11T10:00:00Z', note: row.note, figuresHash: row.figuresHash, stale: false },
    };
  }
  return refusal(500, 'serverError', `unexpected ${method} ${path}`);
}

function headerOf(init: RequestInit | undefined, name: string): string | null {
  const h = init?.headers;
  if (!h) return null;
  if (h instanceof Headers) return h.get(name);
  if (Array.isArray(h)) return h.find(([k]) => k.toLowerCase() === name)?.[1] ?? null;
  const rec = h as Record<string, string>;
  const k = Object.keys(rec).find((x) => x.toLowerCase() === name);
  return k === undefined ? null : rec[k]!;
}

async function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = typeof init?.body === 'string' && init.body !== '' ? JSON.parse(init.body) : undefined;
  const s: Sent = { path: url.pathname, params: url.searchParams, method, key: headerOf(init, 'idempotency-key'), body };
  sent.push(s);
  const reply = answer(s);
  return json(reply.status, reply.body);
}

function readsOf(branch: string): Sent[] {
  return sent.filter((s) => s.path === VIEW && s.method === 'GET' && s.params.get('branch') === branch);
}
function certifies(): Sent[] {
  return sent.filter((s) => s.path === CERTIFY && s.method === 'POST');
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

interface Mounted {
  el: HTMLElement;
  setBranch: (b: string) => void;
}

function mount(): Mounted {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(B01);
  const [period] = createSignal({ from: '2026-10-05', to: '2026-10-05' });
  const [shown] = createSignal(true);
  const Component = menu.Component;
  const el = document.createElement('div');
  root = el;
  document.body.appendChild(el);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    el,
  );
  return { el, setBranch };
}

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function button(el: ParentNode, name: string): HTMLButtonElement {
  const found = [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
  expect(found.length, `one button ${JSON.stringify(name)}`).toBe(1);
  expect(found[0]!.disabled, `the button ${JSON.stringify(name)} is enabled`).toBe(false);
  return found[0]!;
}

async function certify(el: ParentNode, lang: 'th' | 'en' = 'th'): Promise<void> {
  button(el, say('certify', lang)).click();
  await settle();
}

describe('Reconciliation refresh on refreshTick (#2598)', () => {
  beforeEach(() => {
    sent = [];
    hashes = { [B01]: 'hash-b01-aaaa' };
    certified = {};
    vi.stubGlobal('fetch', vi.fn(fakeFetch));
    toast('');
    location.hash = '#reconciliation';
    setLang('th');
  });

  afterEach(async () => {
    closeDialog();
    dispose?.();
    dispose = undefined;
    root?.remove();
    root = undefined;
    vi.unstubAllGlobals();
    location.hash = '';
    setLang('th');
    await settle();
  });

  it('row 1: Sheet shown, shell Refresh pressed -> The sheet is read again', async () => {
    mount();
    await settle();

    expect(readsOf(B01).length, 'initial read of B01').toBe(1);

    // Shell refresh is triggered
    requestRefresh();
    await settle();

    // Sheet must be read again
    expect(readsOf(B01).length, 're-read of B01 after refresh').toBe(2);
  });

  it('row 2: After a staleVersion refusal, Refresh, then Certify -> Certify uses the new version and is sent', async () => {
    const { el } = mount();
    await settle();

    expect(readsOf(B01).length).toBe(1);

    // Figures move on server
    hashes[B01] = 'hash-b01-updated';

    // First certify attempt: uses initial hash and gets refused with 409 staleVersion
    await certify(el);
    expect(certifies().length).toBe(1);
    expect(certifies()[0]!.body?.figuresHash).toBe('hash-b01-aaaa');
    expect(document.body.textContent ?? '').toContain('ข้อมูลมีการเปลี่ยนแปลง กรุณาโหลดหน้าใหม่แล้วลองอีกครั้ง');

    // User triggers shell refresh
    requestRefresh();
    await settle();

    expect(readsOf(B01).length, 'sheet re-read after refresh').toBe(2);

    // Second certify attempt: now sends the updated hash and succeeds
    await certify(el);
    expect(certifies().length).toBe(2);
    expect(certifies()[1]!.body?.figuresHash).toBe('hash-b01-updated');

    // Re-read after certify success
    expect(readsOf(B01).length, 'sheet re-read after certify success').toBe(3);
  });
});
