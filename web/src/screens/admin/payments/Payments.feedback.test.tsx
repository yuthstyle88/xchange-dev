// #2489: save feedback ("saved" / save error) belongs to the branch it was saved on. It must not
// stay on screen after a branch switch or a Reload. Setup copied from Payments.extra.test.tsx.

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

function words(): string {
  return (page().textContent ?? '').replace(/\s+/g, ' ').trim();
}

function feedback(): string {
  const p = page().querySelector('#payment-feedback');
  expect(p, 'the feedback line is rendered').not.toBeNull();
  return (p!.textContent ?? '').replace(/\s+/g, ' ').trim();
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

describe('#2489 Payments save feedback does not outlive its branch', () => {
  it('save on the same branch still shows saving then saved', async () => {
    await mount();
    server.holdSave = true;
    await editAndSave('Counter 9');
    expect(feedback()).toBe('Saving…');
    server.release();
    await settle();
    expect(feedback()).toBe('Settings saved successfully');
  });

  it('save A ok, switch to B: no "saved" feedback on B', async () => {
    const m = await mount();
    await editAndSave('Counter 9');
    expect(feedback()).toBe('Settings saved successfully');

    m.setBranch('A1');
    await settle();

    expect(words()).toContain('Alpha Desk');
    expect(server.paymentReads).toContain('A1');
    expect(feedback()).toBe('');
  });

  it('save A fails, switch to B: no save error on B', async () => {
    vi.stubGlobal('confirm', vi.fn(() => true));
    const m = await mount();
    server.failSave = true;
    await editAndSave('Counter 9');
    const err = feedback();
    expect(err).not.toBe('');
    expect(err).not.toBe('Saving…');

    m.setBranch('A1');
    await settle();

    expect(server.paymentReads).toContain('A1');
    expect(words()).toContain('Alpha Desk');
    expect(feedback()).toBe('');
  });

  it('save ok, Reload: feedback cleared', async () => {
    await mount();
    await editAndSave('Counter 9');
    expect(feedback()).toBe('Settings saved successfully');
    const before = server.paymentReads.length;

    button('Reload latest').click();
    await settle();

    expect(server.paymentReads.length).toBeGreaterThan(before);
    expect(feedback()).toBe('');
  });

  it('save A in flight, switch to B, A answers: no feedback on B', async () => {
    const m = await mount();
    server.holdSave = true;
    await editAndSave('Counter 9');
    expect(feedback()).toBe('Saving…');

    m.setBranch('A1');
    await settle();
    server.release();
    await settle();

    expect(server.paymentReads).toContain('A1');
    expect(words()).toContain('Alpha Desk');
    expect(feedback()).toBe('');
  });

  it('edit A, switch to B, decline the confirm, save A (moves to B): no "saved" feedback on B', async () => {
    vi.stubGlobal('confirm', vi.fn(() => false));
    const m = await mount();
    type(accountInput('Cash'), 'Counter 9');
    await settle();

    m.setBranch('A1');
    await settle();
    expect(server.paymentReads).not.toContain('A1');

    button('Save settings').click();
    await settle();

    expect(server.paymentReads).toContain('A1');
    expect(words()).toContain('Alpha Desk');
    expect(feedback()).toBe('');
  });
});
