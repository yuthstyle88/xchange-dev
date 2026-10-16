// #2246: Admin Payments — switching branch quickly must not show (or save) the previous branch's
// payment methods under the new branch. One test per row of the ticket's behaviour table (§8).
//
// Render and fake-server set-up follow the neighbouring `Payments.test.tsx`: the menu is mounted
// as the shell mounts it (`branch`, `period`, `shown`) and talks to the global `fetch`, which the
// fake below answers. Each branch's `admin_payments` read can be held (answered later) or failed,
// so the reads can be made to answer out of order.
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
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
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
  /** Branches whose next `admin_payments` read waits until `release(branch)`. */
  hold = new Set<string>();
  /** Branches whose `admin_payments` read is refused. */
  fail = new Set<string>();
  /** When set, a read with `since` answers `{changed: false}` with this version. */
  unchangedAt: number | null = null;
  /** When set, the next `admin_branches` read waits until `release('branches')`. */
  holdBranches = false;
  /** When set, a held `admin_branches` read is refused when released. */
  failHeldBranches = false;
  private waiting: Record<string, (() => void)[]> = {};
  reads: Read[] = [];
  posts: { branchId: string; version: number; methods: Row[] }[] = [];

  release(branch?: string): void {
    for (const b of branch ? [branch] : Object.keys(this.waiting)) {
      const w = this.waiting[b] ?? [];
      this.waiting[b] = [];
      for (const f of w) f();
    }
  }

  fetch = async (input: string, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input), 'http://desk.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method === 'GET' && PAGE_WIDE.includes(url.pathname)) return json({ version: 1, changed: false });
    if (method === 'GET' && url.pathname === '/api/v3/views/admin_branches') {
      // the list as it is when the read is made: a held read answers with what was true then
      const list = this.branches.map((b) => ({ ...b }));
      if (this.holdBranches) {
        this.holdBranches = false;
        await new Promise<void>((resolve) => (this.waiting['branches'] ??= []).push(resolve));
        if (this.failHeldBranches) {
          return json({ error: { code: 'serverError', message: 'read of branches failed' } }, 500);
        }
      }
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
      if (since !== null && this.unchangedAt !== null) return json({ version: this.unchangedAt, changed: false });
      if (since !== null && since === String(s.version)) return json({ version: s.version, changed: false });
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
      const s = this.data[body.payload.branchId]!;
      s.methods = body.payload.methods.map((r) => ({ ...r }));
      s.settingsVersion += 1;
      s.version += 1;
      return json({ branchId: body.payload.branchId, version: s.settingsVersion, methods: s.methods });
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
  dispose = render((): JSX.Element => <Payments branch={b} period={period} shown={shown} />, container);
  await settle();
  return { setBranch };
}

function norm(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

/** The account box of a channel's card. */
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

function alerts(): string {
  return Array.from(container!.querySelectorAll('[role="alert"]'))
    .map((e) => norm(e.textContent))
    .join(' | ');
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

describe('#2246 Admin Payments: out-of-order branch reads', () => {
  it("row 1: switch A→B, A's answer arrives after B's — B's methods shown, A's ignored, Save writes B's methods", async () => {
    server.hold.add('A1');
    const m = await mount('A1');
    m.setBranch('B2');
    await settle();
    expect(shownAccounts(), "B's answer is shown").toEqual(rows('Bravo').map((r) => r.account));

    // A's slow answer arrives now, after B's
    server.release('A1');
    await settle();
    expect(shownAccounts(), "A's late answer is ignored").toEqual(rows('Bravo').map((r) => r.account));
    expect(norm(container!.querySelector('h2')?.textContent)).toContain('Bravo Desk');

    const save = Array.from(container!.querySelectorAll<HTMLButtonElement>('button[type="submit"]'));
    expect(save.length).toBe(1);
    save[0]!.click();
    await settle();
    expect(server.posts.length, 'one save posted').toBe(1);
    const p = server.posts[0]!;
    expect(p.branchId).toBe('B2');
    expect(p.version, "B's own setting version").toBe(7);
    expect(p.methods.map((r) => r.account), "B's methods are saved").toEqual(rows('Bravo').map((r) => r.account));
  });

  it("row 2: switch A→B, A's read fails after B's answer — B's data stays, no error from A", async () => {
    server.hold.add('A1');
    server.fail.add('A1');
    const m = await mount('A1');
    m.setBranch('B2');
    await settle();
    expect(shownAccounts()).toEqual(rows('Bravo').map((r) => r.account));

    server.release('A1');
    await settle();
    expect(container!.querySelectorAll('[data-payment-id]').length, "B's cards stay").toBe(IDS.length);
    expect(shownAccounts(), "B's data stays").toEqual(rows('Bravo').map((r) => r.account));
    expect(alerts(), "no error from A's read").toBe('');
    expect(norm(container!.textContent)).not.toContain('read of A1 failed');
  });

  /** What B's screen shows: the heading, every account box, the alerts and all the words. */
  function screenOfB(): { heading: string; accounts: string[]; alerts: string; words: string } {
    return {
      heading: norm(container!.querySelector('h2')?.textContent),
      accounts: shownAccounts(),
      alerts: alerts(),
      words: norm(container!.textContent),
    };
  }

  it("row 2 (branches read fails): switch A→B, A's admin_branches read fails after B is shown — B's screen unchanged", async () => {
    server.holdBranches = true;
    server.failHeldBranches = true;
    const m = await mount('A1');
    m.setBranch('B2');
    await settle();
    const before = screenOfB();
    expect(before.heading).toContain('Bravo Desk');
    expect(before.accounts).toEqual(rows('Bravo').map((r) => r.account));
    expect(before.alerts).toBe('');

    // A's first read (the branch names) fails now, after B is on screen
    server.release('branches');
    await settle();
    expect(alerts(), "no error from A's branches read").toBe('');
    expect(screenOfB(), "B's screen is unchanged").toEqual(before);
  });

  it("row 2 (branches read answers): switch A→B, A's admin_branches read answers after B is shown — B's screen unchanged", async () => {
    // when A asks, B's name is still the old one; it is renamed before B's own read
    server.branches = [
      { id: 'A1', name: 'Alpha Desk' },
      { id: 'B2', name: 'Bravo Old Name' },
    ];
    server.holdBranches = true;
    const m = await mount('A1');
    server.branches = [
      { id: 'A1', name: 'Alpha Desk' },
      { id: 'B2', name: 'Bravo Desk' },
    ];
    m.setBranch('B2');
    await settle();
    const before = screenOfB();
    expect(before.heading).toContain('Bravo Desk');
    expect(before.accounts).toEqual(rows('Bravo').map((r) => r.account));

    // A's stale branch list arrives now, after B is on screen
    server.release('branches');
    await settle();
    expect(norm(container!.textContent), "A's stale branch list is ignored").not.toContain('Bravo Old Name');
    expect(screenOfB(), "B's screen is unchanged").toEqual(before);
  });

  it('row 3 (kept): a single branch load shows its methods', async () => {
    await mount('A1');
    expect(server.reads).toEqual([{ branch: 'A1', since: null }]);
    expect(shownAccounts()).toEqual(rows('Alpha').map((r) => r.account));
    expect(alerts()).toBe('');
  });

  it('row 4 (kept): a refresh with since and changed:false keeps the data and takes the new version', async () => {
    await mount('A1');
    requestRefresh();
    await settle();
    expect(server.reads.at(-1), 'refresh carries the version held').toEqual({ branch: 'A1', since: '11' });
    expect(shownAccounts(), 'changed: false keeps the data').toEqual(rows('Alpha').map((r) => r.account));

    // an unchanged answer with a newer version: data kept, and the next refresh carries the new version
    server.unchangedAt = 15;
    requestRefresh();
    await settle();
    expect(server.reads.at(-1)).toEqual({ branch: 'A1', since: '11' });
    expect(shownAccounts()).toEqual(rows('Alpha').map((r) => r.account));
    requestRefresh();
    await settle();
    expect(server.reads.at(-1), 'version updated from the changed:false answer').toEqual({ branch: 'A1', since: '15' });
    expect(shownAccounts()).toEqual(rows('Alpha').map((r) => r.account));
  });
});
