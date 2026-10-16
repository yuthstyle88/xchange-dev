// #2492: Admin Payments — a save that fails while a branch switch is parked must handle the
// parked switch via the dirty-switch confirm path. Setup copied from Payments.feedback.test.tsx.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { menu } from './menu';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';

interface Row {
  id: string;
  enabled: boolean;
  buy: boolean;
  sell: boolean;
  account: string;
  instruction: string;
}

interface Settings {
  version: number;
  branchId: string;
  settingsVersion: number;
  methods: Row[];
  updatedAt: string | null;
  updatedBy: string | null;
  canEdit: boolean;
  history: never[];
}

const SAVED_AT = '2026-10-08T05:15:00.000Z';

function rows(): Row[] {
  return [
    { id: 'Cash', enabled: true, buy: true, sell: true, account: '', instruction: '' },
    { id: 'Credit Card', enabled: true, buy: false, sell: true, account: '', instruction: '' },
    { id: 'Alipay', enabled: true, buy: false, sell: true, account: '', instruction: '' },
    { id: 'PromptPay', enabled: true, buy: false, sell: true, account: '', instruction: '' },
    { id: 'Bank Transfer', enabled: true, buy: true, sell: true, account: '', instruction: '' },
  ];
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function settingsFor(branchId: string, version: number): Settings {
  return {
    version,
    branchId,
    settingsVersion: 1,
    methods: rows(),
    updatedAt: null,
    updatedBy: null,
    canEdit: true,
    history: [],
  };
}

class Server {
  branches = [
    { id: 'MAIN', name: 'Main Desk' },
    { id: 'A1', name: 'Alpha Desk' },
  ];
  settings: Record<string, Settings> = {
    MAIN: settingsFor('MAIN', 41),
    A1: settingsFor('A1', 10),
  };
  holdSave = false;
  failSave = false;
  private released: (() => void)[] = [];
  paymentReads: string[] = [];

  release(): void {
    const r = this.released;
    this.released = [];
    for (const f of r) f();
  }

  private wait(): Promise<void> {
    return new Promise((resolve) => this.released.push(resolve));
  }

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input), 'http://desk.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method === 'GET' && PAGE_WIDE.includes(url.pathname)) {
      return json({ version: 1, changed: false });
    }
    const view = url.pathname.startsWith('/api/v3/views/') ? url.pathname.slice('/api/v3/views/'.length) : null;
    if (method === 'GET' && view === 'admin_payments') {
      const branch = url.searchParams.get('branch') ?? 'MAIN';
      this.paymentReads.push(branch);
      const s = this.settings[branch];
      if (!s) return json({ error: { code: 'noSuchBranch', message: 'not found' } }, 400);
      const { version, ...data } = s;
      return json({ version, changed: true, data: JSON.parse(JSON.stringify(data)) });
    }
    if (method === 'GET' && view === 'admin_branches') {
      return json({ version: 1, changed: true, data: { branches: this.branches.map((b) => ({ ...b })) } });
    }
    if (method === 'POST' && url.pathname === '/api/payment-methods') {
      const body = JSON.parse(String(init?.body ?? '{}')) as { payload: { branchId: string; methods: Row[] } };
      if (this.holdSave) await this.wait();
      if (this.failSave) {
        return json({ error: { code: 'conflict', message: 'settings changed elsewhere' } }, 409);
      }
      const s = this.settings[body.payload.branchId]!;
      s.version += 1;
      s.settingsVersion += 1;
      s.methods = body.payload.methods.map((r) => ({ ...r }));
      s.updatedAt = SAVED_AT;
      s.updatedBy = 'owner';
      return json({ branchId: s.branchId, version: s.settingsVersion, methods: s.methods, canEdit: true, history: [] });
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

async function mount(): Promise<{ setBranch: (b: string) => void }> {
  const [branch, setBranch] = createSignal('MAIN');
  const [shown] = createSignal(true);
  const period: Accessor<{ from: string; to: string }> = () => ({ from: '', to: '' });
  container = document.createElement('div');
  document.body.appendChild(container);
  const Component = menu.Component;
  dispose = render((): JSX.Element => <Component branch={branch} period={period} shown={shown} />, container);
  await settle();
  return { setBranch };
}

function page(): HTMLElement {
  expect(container, 'the menu is mounted').toBeDefined();
  return container!;
}

function heading(): string {
  return (page().querySelector('h2')?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function feedback(): string {
  const p = page().querySelector('#payment-feedback');
  expect(p, 'the feedback line is rendered').not.toBeNull();
  return (p!.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function branchNotice(): string | null {
  const el = page().querySelector('.xc-branch-notice');
  return el ? (el.textContent ?? '').replace(/\s+/g, ' ').trim() : null;
}

function button(text: string): HTMLButtonElement {
  const found = Array.from(page().querySelectorAll<HTMLButtonElement>('button')).filter(
    (b) => (b.textContent ?? '').replace(/\s+/g, ' ').trim() === text,
  );
  expect(found.length, `one button "${text}"`).toBe(1);
  return found[0]!;
}

function accountInput(id: string): HTMLInputElement {
  const card = page().querySelector<HTMLElement>(`[data-payment-id="${id}"]`);
  expect(card, `card ${id}`).not.toBeNull();
  const input = card!.querySelector<HTMLInputElement>('input[type="text"]');
  expect(input).not.toBeNull();
  return input!;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

async function editAndSave(value: string): Promise<void> {
  type(accountInput('Cash'), value);
  await settle();
  button('Save settings').click();
  await settle();
}

beforeEach(() => {
  setLang('en');
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

describe('#2492 Payments parked branch switch on failed save', () => {
  it('row 1: save on MAIN held, shell switches to A1, save fails 409, user CONFIRMS the switch', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    const m = await mount();
    server.holdSave = true;
    server.failSave = true;

    await editAndSave('Counter 9');
    expect(feedback()).toBe('Saving…');

    m.setBranch('A1');
    await settle();
    expect(confirm).not.toHaveBeenCalled();

    server.release();
    await settle();

    expect(confirm).toHaveBeenCalledTimes(1);
    expect(server.paymentReads).toContain('A1');
    expect(heading()).toContain('Alpha Desk');
    expect(feedback()).toBe('');
    expect(branchNotice()).toBeNull();
  });

  it('row 2: save on MAIN held, shell switches to A1, save fails 409, user DECLINES', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    const m = await mount();
    server.holdSave = true;
    server.failSave = true;

    await editAndSave('Counter 9');
    expect(feedback()).toBe('Saving…');

    m.setBranch('A1');
    await settle();
    expect(confirm).not.toHaveBeenCalled();

    server.release();
    await settle();

    expect(confirm).toHaveBeenCalledTimes(1);
    expect(server.paymentReads).toEqual(['MAIN']);
    expect(heading()).toContain('Main Desk');
    expect(feedback()).toBe('settings changed elsewhere');
    expect(branchNotice()).toBe('Showing edits for branch Main Desk (save or discard before switching)');
  });

  it('row 3: save on MAIN held, shell switches to A1 then back to MAIN, save fails 409', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    const m = await mount();
    server.holdSave = true;
    server.failSave = true;

    await editAndSave('Counter 9');
    expect(feedback()).toBe('Saving…');

    m.setBranch('A1');
    await settle();
    m.setBranch('MAIN');
    await settle();

    server.release();
    await settle();

    expect(confirm).not.toHaveBeenCalled();
    expect(branchNotice()).toBeNull();
    expect(heading()).toContain('Main Desk');
    expect(feedback()).toBe('settings changed elsewhere');
    expect(server.paymentReads).toEqual(['MAIN']);

    // Retrying the save on MAIN must stay on MAIN (parked switch was cleared)
    server.holdSave = false;
    server.failSave = false;
    button('Save settings').click();
    await settle();

    expect(confirm).not.toHaveBeenCalled();
    expect(branchNotice()).toBeNull();
    expect(heading()).toContain('Main Desk');
    expect(server.paymentReads).toEqual(['MAIN', 'MAIN']);
  });

  it('row 4 (kept): save fails with NO parked switch', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    await mount();
    server.failSave = true;

    await editAndSave('Counter 9');

    expect(confirm).not.toHaveBeenCalled();
    expect(branchNotice()).toBeNull();
    expect(heading()).toContain('Main Desk');
    expect(feedback()).toBe('settings changed elsewhere');
    expect(server.paymentReads).toEqual(['MAIN']);
  });

  it('row 5 (kept): save succeeds with a parked switch', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    const m = await mount();
    server.holdSave = true;

    await editAndSave('Counter 9');
    expect(feedback()).toBe('Saving…');

    m.setBranch('A1');
    await settle();
    expect(confirm).not.toHaveBeenCalled();

    server.release();
    await settle();

    expect(confirm).not.toHaveBeenCalled();
    expect(server.paymentReads).toContain('A1');
    expect(heading()).toContain('Alpha Desk');
    expect(feedback()).toBe('');
    expect(branchNotice()).toBeNull();
  });
});
