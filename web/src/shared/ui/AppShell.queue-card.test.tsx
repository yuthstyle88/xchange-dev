// OL-6 wave B queue-card-f52 (owner finding F52, relayed by the lane controller ~15:5x ICT 2026-11-07): the floating
// queue card covers "พิมพ์ซ้ำ" on /desk/transactions/<id> and shows on /desk/backdated, where it does not belong.
// Written before the fix by the ticket author claude/studio-claude-17 for the sub-controller claude/1cc51615. Do not
// edit, delete or weaken.
//
// Root: AppShell.tsx draws AccountBar for every staff page, and AccountBar.tsx draws <QueueCard> with no way for a page
// to say otherwise; the one exception so far was a pathname check inside QueueCard.tsx (#6659, /staff/report). The fix
// is a page flag: AppShell `queueCard` = 'open' (default) | 'folded' | 'hidden', set by the page's main.tsx.
//
// The shared session is a page singleton, so every test loads fresh modules (as AppShell.header.test.tsx does).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Me } from '../api/generated/common';
import { meOf } from '../../test-support/me';
import { warmModules } from '../testing/warm-modules';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

const TELLER: Me = meOf({ id: 'E7', role: 'FRONT', branch: 'B1', name: 'Nok', branchName: 'B1', branches: [] });

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let cleanup: Array<() => void> = [];

async function load() {
  vi.resetModules();
  const web = await import('solid-js/web');
  const solid = await import('solid-js');
  const { AppShell } = await import('./AppShell');
  const session = await import('../session/session');
  const i18n = await import('../i18n');
  return { web, solid, AppShell, session, i18n };
}

type Loaded = Awaited<ReturnType<typeof load>>;

async function mount(queueCard?: string): Promise<HTMLElement> {
  const m: Loaded = await load();
  m.i18n.setLang('th');
  const host = document.createElement('div');
  document.body.appendChild(host);
  const props = { title: 'หน้า', ...(queueCard === undefined ? {} : { queueCard }) };
  const dispose = m.web.render(
    () => m.solid.createComponent(m.AppShell, props as unknown as Parameters<typeof m.AppShell>[0]),
    host,
  );
  cleanup.push(dispose, () => m.session.sharedSession().dispose());
  await vi.waitFor(() => expect(host.querySelector('.account-bar')).not.toBeNull());
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
  return host;
}

const card = () => document.querySelector<HTMLElement>('aside.queue-card');

async function readSrc(rel: string): Promise<string> {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  return fs.readFileSync(here + rel, 'utf8');
}

beforeAll(
  () =>
    warmModules(
      () => import('solid-js/web'),
      () => import('solid-js'),
      () => import('./AppShell'),
      () => import('../session/session'),
      () => import('../i18n'),
    ),
  60_000,
);

beforeEach(() => {
  Reflect.deleteProperty(window, 'matchMedia');
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: string) => {
      const url = String(input);
      if (url.startsWith('/api/v3/views/me')) return json(200, { version: 3, changed: true, data: TELLER });
      return json(401, { error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } });
    }),
  );
});

afterEach(() => {
  for (const c of cleanup.splice(0)) c();
  document.body.innerHTML = '';
  document.documentElement.style.removeProperty('--xc-queue-card-space');
  vi.unstubAllGlobals();
});

describe('OL-6 B F52 a page decides whether the queue card shows', () => {
  it('QM-1 no flag: the card shows, open (every other page unchanged)', async () => {
    await mount();
    expect(card(), 'aside.queue-card').not.toBeNull();
    expect(card()!.classList.contains('minimized')).toBe(false);
  });

  it("QM-2 queueCard 'hidden': the account bar shows, the card does not, and no bottom space is reserved for it", async () => {
    const host = await mount('hidden');
    expect(host.querySelector('.account-bar')).not.toBeNull();
    expect(card()).toBeNull();
    expect(document.documentElement.style.getPropertyValue('--xc-queue-card-space')).toBe('');
  });

  it("QM-3 queueCard 'folded': the card shows, folded", async () => {
    await mount('folded');
    expect(card(), 'aside.queue-card').not.toBeNull();
    expect(card()!.classList.contains('minimized')).toBe(true);
  });

  it("QM-4 queueCard 'open': the same as no flag", async () => {
    await mount('open');
    expect(card()!.classList.contains('minimized')).toBe(false);
  });

  // restated by OL-8 staff-queue-pill (ruling (a)): the report keeps the pill, `queueCard="pill"`
  it('QM-5 the pages set the flag in their entry: backdated and transaction detail hidden, the report a pill', async () => {
    expect(await readSrc('../../screens/desk-backdated/main.tsx')).toMatch(/<AppShell[^>]*\squeueCard="hidden"/);
    expect(await readSrc('../../screens/desk-transaction/main.tsx')).toMatch(/<AppShell[^>]*\squeueCard="hidden"/);
    expect(await readSrc('../../screens/staff-report/main.tsx')).toMatch(/<AppShell[^>]*\squeueCard="pill"/);
    expect(await readSrc('../../screens/desk/main.tsx')).not.toContain('queueCard=');
  });

  it('QM-6 no page path is checked in the shared shell, bar or card', async () => {
    for (const f of ['QueueCard.tsx', 'AccountBar.tsx', 'AppShell.tsx']) {
      expect(await readSrc(f), f).not.toMatch(/location\.pathname\s*===|pathname\s*===\s*'\//);
    }
  });
});
