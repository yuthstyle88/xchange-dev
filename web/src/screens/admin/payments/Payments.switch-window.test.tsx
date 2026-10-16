import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type Accessor, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { requestRefresh } from '../parts/refresh';
import { PAGE_WIDE } from '../../../shared/testing/page-wide';
import { Payments } from './Payments';

interface Row {
  id: string;
  enabled: boolean;
  buy: boolean;
  sell: boolean;
  account: string;
  instruction: string;
}

const IDS = ['Cash', 'Credit Card', 'Alipay', 'PromptPay', 'Bank Transfer'] as const;
const BUY = ['Cash', 'Bank Transfer'];

function rows(tag: string): Row[] {
  return IDS.map((id) => ({
    id,
    enabled: true,
    buy: BUY.includes(id),
    sell: true,
    account: `${tag} ${id}`,
    instruction: `${tag} says hello`,
  }));
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

interface Branch {
  version: number;
  settingsVersion: number;
  methods: Row[];
}

interface Read {
  branch: string | null;
  since: string | null;
}

class Server {
  branches = [
    { id: 'A1', name: 'Alpha Desk' },
    { id: 'B2', name: 'Bravo Desk' },
  ];
  data: Record<string, Branch> = {
    A1: { version: 11, settingsVersion: 4, methods: rows('Alpha') },
    B2: { version: 22, settingsVersion: 7, methods: rows('Bravo') },
  };
  hold = new Set<string>();
  fail = new Set<string>();
  holdSave = false;
  private waiting: Record<string, (() => void)[]> = {};
  private waitingSave: (() => void)[] = [];
  reads: Read[] = [];
  posts: { branchId: string; version: number; methods: Row[] }[] = [];

  release(branch?: string): void {
    for (const b of branch ? [branch] : Object.keys(this.waiting)) {
      const w = this.waiting[b] ?? [];
      this.waiting[b] = [];
      for (const f of w) f();
    }
  }

  releaseSave(): void {
    const w = this.waitingSave;
    this.waitingSave = [];
    for (const f of w) f();
  }

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input), 'http://desk.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method === 'GET' && PAGE_WIDE.includes(url.pathname)) {
      return json({ version: 1, changed: false });
    }
    if (method === 'GET' && url.pathname === '/api/v3/views/admin_branches') {
      const list = this.branches.map((b) => ({ ...b }));
      return json({ version: 1, changed: true, data: { branches: list } });
    }
    if (method === 'GET' && url.pathname === '/api/v3/views/admin_payments') {
      const branch = url.searchParams.get('branch') ?? '';
      const since = url.searchParams.get('since');
      this.reads.push({ branch, since });
      if (this.hold.has(branch)) {
        this.hold.delete(branch);
        await new Promise<void>((resolve) => (this.waiting[branch] ??= []).push(resolve));
      }
      if (this.fail.has(branch)) {
        return json({ error: { code: 'serverError', message: `read of ${branch} failed` } }, 500);
      }
      const s = this.data[branch]!;
      if (since !== null && since === String(s.version)) {
        return json({ version: s.version, changed: false });
      }
      return json({
        version: s.version,
        changed: true,
        data: {
          branchId: branch,
          settingsVersion: s.settingsVersion,
          methods: s.methods.map((r) => ({ ...r })),
          updatedAt: null,
          updatedBy: null,
          canEdit: true,
          history: [],
        },
      });
    }
    if (method === 'POST' && url.pathname === '/api/payment-methods') {
      const body = JSON.parse(String(init?.body ?? '{}')) as {
        payload: { branchId: string; version: number; methods: Row[] };
      };
      this.posts.push(body.payload);
      if (this.holdSave) {
        this.holdSave = false;
        await new Promise<void>((resolve) => this.waitingSave.push(resolve));
      }
      const s = this.data[body.payload.branchId]!;
      s.methods = body.payload.methods.map((r) => ({ ...r }));
      s.settingsVersion += 1;
      s.version += 1;
      return json({
        branchId: body.payload.branchId,
        version: s.settingsVersion,
        methods: s.methods,
      });
    }
    return json({ error: { code: 'notFound', message: 'not found' } }, 404);
  };
}

let server: Server;
let dispose: (() => void) | undefined;
let container: HTMLDivElement | undefined;

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(branch: string): Promise<{ setBranch: (b: string) => void }> {
  const [b, setBranch] = createSignal(branch);
  const [shown] = createSignal(true);
  const period: Accessor<{ from: string; to: string }> = () => ({ from: '', to: '' });
  container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(
    (): JSX.Element => <Payments branch={b} period={period} shown={shown} />,
    container,
  );
  await settle();
  return { setBranch };
}

function norm(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

function account(id: string): HTMLInputElement {
  const card = container!.querySelector<HTMLElement>(`[data-payment-id="${id}"]`);
  expect(card, `card ${id}`).not.toBeNull();
  const input = card!.querySelector<HTMLInputElement>('input[type="text"]');
  expect(input, `account box of ${id}`).not.toBeNull();
  return input!;
}

function shownAccounts(): string[] {
  return IDS.map((id) => account(id).value);
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function buttonByText(label: string): HTMLButtonElement {
  const found = Array.from(container!.querySelectorAll<HTMLButtonElement>('button')).filter(
    (b) => norm(b.textContent) === label,
  );
  expect(found.length, `button "${label}"`).toBe(1);
  return found[0]!;
}

beforeEach(() => {
  setLang('th');
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
});

afterEach(async () => {
  server.release();
  server.releaseSave();
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

describe('#2403 Admin Payments: branch switch window before the new branch answers', () => {
  it("clean switch A→B while B's read is in flight hides A's methods and offers no Save", async () => {
    const m = await mount('A1');
    expect(shownAccounts()).toEqual(rows('Alpha').map((r) => r.account));

    server.hold.add('B2');
    m.setBranch('B2');
    await settle();

    expect(container!.querySelectorAll('[data-payment-id]').length).toBe(0);
    expect(container!.querySelectorAll('button[type="submit"]').length).toBe(0);
    expect(norm(container!.textContent)).not.toContain('Alpha Cash');

    server.release('B2');
    await settle();
    expect(shownAccounts()).toEqual(rows('Bravo').map((r) => r.account));
  });

  it("clicking Save or submitting in the A→B switch window while B's read is in flight sends no POST", async () => {
    const m = await mount('A1');
    expect(shownAccounts()).toEqual(rows('Alpha').map((r) => r.account));

    server.hold.add('B2');
    m.setBranch('B2');
    await settle();

    for (const b of Array.from(container!.querySelectorAll<HTMLButtonElement>('button[type="submit"]'))) {
      b.click();
    }
    for (const f of Array.from(container!.querySelectorAll('form'))) {
      f.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    }
    await settle();

    expect(server.posts).toEqual([]);

    server.release('B2');
    await settle();
    expect(server.posts).toEqual([]);
  });

  it("save in flight → switch → save answers → while B's read is in flight, A's form is not shown", async () => {
    const m = await mount('A1');
    type(account('Cash'), 'Alpha Edited');
    await settle();

    server.holdSave = true;
    server.hold.add('B2');
    const save = container!.querySelector<HTMLButtonElement>('button[type="submit"]');
    expect(save).not.toBeNull();
    save!.click();
    await settle();

    m.setBranch('B2');
    await settle();

    server.releaseSave();
    await settle();

    expect(container!.querySelectorAll('[data-payment-id]').length).toBe(0);
    expect(container!.querySelectorAll('button[type="submit"]').length).toBe(0);

    server.release('B2');
    await settle();
    expect(shownAccounts()).toEqual(rows('Bravo').map((r) => r.account));
  });

  it("ignores A's late answer after B's and clears A's methods during the switch window", async () => {
    const m = await mount('A1');
    expect(shownAccounts()).toEqual(rows('Alpha').map((r) => r.account));

    server.hold.add('A1');
    buttonByText('โหลดล่าสุด').click();
    await settle();

    server.hold.add('B2');
    m.setBranch('B2');
    await settle();

    expect(container!.querySelectorAll('[data-payment-id]').length).toBe(0);

    server.release('B2');
    await settle();
    expect(shownAccounts()).toEqual(rows('Bravo').map((r) => r.account));

    server.release('A1');
    await settle();
    expect(shownAccounts()).toEqual(rows('Bravo').map((r) => r.account));
  });

  it('shows methods on a single load and asks nothing on a clean switch while clearing A during the switch window', async () => {
    const confirm = vi.fn(() => false);
    vi.stubGlobal('confirm', confirm);

    const m = await mount('A1');
    expect(shownAccounts()).toEqual(rows('Alpha').map((r) => r.account));

    server.hold.add('B2');
    m.setBranch('B2');
    await settle();

    expect(confirm).not.toHaveBeenCalled();
    expect(container!.querySelectorAll('[data-payment-id]').length).toBe(0);

    server.release('B2');
    await settle();
    expect(shownAccounts()).toEqual(rows('Bravo').map((r) => r.account));
  });

  it('keeps data on a same-branch refresh or reload failure, but clears data immediately when switching branch', async () => {
    const m = await mount('A1');
    expect(shownAccounts()).toEqual(rows('Alpha').map((r) => r.account));

    // Same-branch refresh keeps data while in flight
    server.hold.add('A1');
    requestRefresh();
    await settle();
    expect(shownAccounts()).toEqual(rows('Alpha').map((r) => r.account));
    server.release('A1');
    await settle();

    // Same-branch reload failure keeps data
    server.fail.add('A1');
    buttonByText('โหลดล่าสุด').click();
    await settle();
    expect(shownAccounts()).toEqual(rows('Alpha').map((r) => r.account));
    server.fail.delete('A1');

    // Cross-branch switch clears A1 immediately while B2 is in flight
    server.hold.add('B2');
    m.setBranch('B2');
    await settle();
    expect(container!.querySelectorAll('[data-payment-id]').length).toBe(0);
  });

  it("keeps A on a declined switch, and clears A's form while B's read is in flight when Discard switches to B", async () => {
    const m = await mount('A1');
    type(account('Cash'), 'Unsaved Alpha');
    await settle();

    vi.stubGlobal('confirm', vi.fn(() => false));
    m.setBranch('B2');
    await settle();
    expect(account('Cash').value).toBe('Unsaved Alpha');

    server.hold.add('B2');
    buttonByText('ยกเลิกการแก้ไข').click();
    await settle();

    expect(container!.querySelectorAll('[data-payment-id]').length).toBe(0);

    server.release('B2');
    await settle();
    expect(shownAccounts()).toEqual(rows('Bravo').map((r) => r.account));
  });

  it("clears the version cursor in the switch window so a refresh while B's read is in flight sends since === null and shows B's methods", async () => {
    server.data.B2!.version = server.data.A1!.version;
    const m = await mount('A1');
    expect(shownAccounts()).toEqual(rows('Alpha').map((r) => r.account));

    server.hold.add('B2');
    m.setBranch('B2');
    await settle();

    requestRefresh();
    await settle();

    server.release('B2');
    await settle();

    const b2Reads = server.reads.filter((r) => r.branch === 'B2');
    expect(b2Reads.length).toBeGreaterThanOrEqual(2);
    expect(b2Reads.every((r) => r.since === null)).toBe(true);
    expect(shownAccounts()).toEqual(rows('Bravo').map((r) => r.account));
  });
});
