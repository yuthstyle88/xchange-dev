// S-11 #1281: /staff renders inside the shared AppShell (design docs/design/xchange-v3-wave3-staff.md row S-11 and §8.1,
// PR 1280 merged 9b5dbea6; Blueprint §03 "Desktop มี sidebar + branch selector + status bar", BP:268).
// Written before the implementation by the up-front test writer (claude/5d45b017, on the xchange FE lane controller
// claude/5edf7bb5's order). Do not edit, delete or weaken these tests; add your own in another file.
//
// One test per #1281 §9 edge case:
// - signed out: the S1 redirect, and no `.xc-shell` is ever rendered (not even before the redirect);
// - signed in: exactly one `.xc-shell` and one account bar (AccountBar's root `.account-bar`, `AccountBar.tsx:46`);
// - exactly one `<h1>` on the page, and it is the shell's (`h1.xc-shell-title`, `AppShell.tsx`), carrying the staff
//   page's title;
// - the hours line (`data-testid="staff-hours"`), the refresh button (`data-testid="staff-refresh"`) and the report
//   link (`a[href="/staff/report"]`) keep their S-3 conditions and sit in the shell's main area;
// - the cards are still mounted from the registry, each inside `data-card="<id>"`, in order.
//
// The surface is S-3's, unchanged: `StaffPage(props: { api: ApiClient; cards: readonly StaffCard[];
// navigate: (url: string) => void })`, reached through import.meta.glob as `StaffPage.test.tsx` does. The shell's own
// reads (the shared session's `/api/v3/views/me`, the version footer's `/health/ready`) go to the global `fetch`,
// which this file stubs with the same route table as the page's `ApiClient`.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the session `Me` fixture takes #1030 §7's shape (`kind`/`role`/`username` → `roles: []`,
// `permissions: []`), written out here rather than through `web/src/test-support/me.ts` because a role
// literal would trip S-3's gate. Nothing else changes.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { ApiClient } from '../../shared/api/client';
import {
  makeBranchRef,
  makeMe,
  makeStaffFloorHours,
  makeStaffFloorView,
  makeStaffMeView,
  type StaffFloorView,
} from '../../shared/testing/fixtures';

type PageModule = {
  StaffPage: (p: { api: ApiClient; cards: readonly unknown[]; navigate: (url: string) => void }) => JSX.Element;
};
type TextsModule = { th: Record<string, string>; en: Record<string, string> };

const pageMods = import.meta.glob<PageModule>('./StaffPage.tsx');
const textMods = import.meta.glob<TextsModule>('./i18n.ts', { eager: true });

async function page(): Promise<PageModule> {
  const l = pageMods['./StaffPage.tsx'];
  expect(l, 'web/src/screens/staff/StaffPage.tsx exists').toBeDefined();
  return l!();
}

function staffTitle(): string {
  const th = textMods['./i18n.ts']?.th;
  expect(th?.['title'], 'staff i18n th.title').toBeTypeOf('string');
  return th!['title']!;
}

// ── fixtures, as S-3's StaffPage.test.tsx has them ──────────────────────────────────────────

function staffMe(permissions: string[]) {
  return makeStaffMeView({
    userId: '00000000-0000-4000-8000-0000000000a1',
    name: 'Ploy',
    roles: [],
    branchId: 'MAIN',
    branchName: 'XC-B',
    permissions,
  });
}

/** The shared session's `me` (v3 `Me`), for the shell's account bar. */
const SESSION_ME = makeMe({
  id: '00000000-0000-4000-8000-0000000000a1',
  name: 'Ploy',
  roles: [], // no role literal: it would trip S-3's no-role-names gate (SD:S2); the bar's role text is not under test
  permissions: [],
  branch: 'MAIN',
  branchName: 'XC-B',
  branches: [makeBranchRef({ id: 'MAIN', code: 'XC-B', name: 'XC-B' })],
});

const FLOOR_BASE = makeStaffFloorView({
  phase: 'OPEN',
  hours: null,
  myClockIn: null,
  myShift: null,
  cutoff: null,
  target: null,
  safe: null,
});
const FLOOR_WITH_HOURS = makeStaffFloorView({
  ...FLOOR_BASE,
  hours: makeStaffFloorHours({ open: '2026-10-11T03:00:00Z', close: '2026-10-11T13:00:00Z' }), // 10:00–20:00 Bangkok
});

type Route = { status: number; body: unknown };
let routes: Record<string, () => Route>;
let fetchSpy: ReturnType<typeof vi.fn>;

function ok(data: unknown): Route {
  return { status: 200, body: { version: 1, changed: true, data } };
}

const SIGNED_OUT: Route = { status: 401, body: { error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } } };

function server(meAnswer: Route, floor: StaffFloorView = FLOOR_WITH_HOURS) {
  routes = {
    '/api/v3/views/staff_me': () => meAnswer,
    '/api/v3/views/staff_floor': () => ok(floor),
    // the shell's own reads
    '/api/v3/views/me': () => (meAnswer.status === 200 ? ok(SESSION_ME) : meAnswer),
    '/health/ready': () => ({
      status: 200,
      body: { version: '3.0.0', build: 'abcdef1234567', channel: 'staging', builtAt: '2026-10-11T00:00:00Z' },
    }),
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
  fetchSpy = vi.fn(async (input: RequestInfo | URL) => {
    const path = String(input).split('?')[0]!;
    const route = routes[path];
    if (!route) {
      return new Response(JSON.stringify({ error: { code: 'notFound', message: 'not found' } }), {
        status: 404,
        headers: { 'content-type': 'application/json' },
      });
    }
    const { status, body } = route();
    return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
  });
  vi.stubGlobal('fetch', fetchSpy);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

type Card = {
  id: string;
  order: number;
  visible: (ctx: unknown) => boolean;
  Component: (p: { ctx: unknown }) => JSX.Element;
};

function card(id: string, order: number): Card {
  return { id, order, visible: () => true, Component: () => <p data-testid={`body-${id}`}>{id}</p> };
}

async function mount(cards: Card[] = []) {
  const { StaffPage } = await page();
  const navigate = vi.fn();
  const api = new ApiClient(fetchSpy as unknown as typeof fetch);
  dispose = render(() => <StaffPage api={api} cards={cards} navigate={navigate} />, host);
  return navigate;
}

/** The shell's main area: AppShell renders `<main class="xc-shell-main">`, or `<div class="xc-content">` with ownsMain. */
function mainArea(): Element | null {
  return host.querySelector('.xc-shell .xc-shell-main, .xc-shell .xc-content');
}

async function signedIn(permissions: string[], cards: Card[] = [], floor: StaffFloorView = FLOOR_WITH_HOURS) {
  server(ok(staffMe(permissions)), floor);
  const navigate = await mount(cards);
  await vi.waitFor(() => expect(host.querySelector('[data-testid="staff-hours"]')).not.toBeNull());
  await vi.waitFor(() => expect(host.querySelector('.account-bar')).not.toBeNull());
  return navigate;
}

// ── #1281 §9 ────────────────────────────────────────────────────────────────────────────────

describe('S-11: /staff inside AppShell (#1281 §9)', () => {
  it('signed out: the S1 redirect, and no .xc-shell is ever rendered', async () => {
    server(SIGNED_OUT);
    let shellSeen = false;
    const watch = new MutationObserver(() => {
      if (host.querySelector('.xc-shell')) shellSeen = true;
    });
    watch.observe(host, { childList: true, subtree: true });
    const navigate = await mount([card('floor', 10)]);
    if (host.querySelector('.xc-shell')) shellSeen = true;
    await vi.waitFor(() => expect(navigate).toHaveBeenCalledWith('/login?next=%2Fstaff'));
    await new Promise((r) => setTimeout(r, 0));
    watch.disconnect();
    expect(shellSeen, 'a .xc-shell was rendered for a signed-out visitor').toBe(false);
    expect(host.querySelector('.xc-shell')).toBeNull();
    expect(host.querySelector('[data-card]')).toBeNull();
  });

  it('signed in: exactly one .xc-shell and one account bar', async () => {
    const navigate = await signedIn([]);
    expect(navigate).not.toHaveBeenCalled();
    expect(host.querySelectorAll('.xc-shell')).toHaveLength(1);
    expect(host.querySelectorAll('.account-bar')).toHaveLength(1);
    expect(host.querySelector('.xc-shell .account-bar'), 'the account bar is the shell\'s').not.toBeNull();
    expect(host.querySelector('.account-bar')?.textContent ?? '').toContain('Ploy');
  });

  it("exactly one <h1> on the page, and it is the shell's, with the staff title", async () => {
    await signedIn([]);
    const h1s = host.querySelectorAll('h1');
    expect(h1s, 'one <h1>').toHaveLength(1);
    const h1 = h1s[0]!;
    expect(h1.closest('.xc-shell'), 'inside the shell').not.toBeNull();
    expect(h1.classList.contains('xc-shell-title'), "the shell's own title element").toBe(true);
    expect((h1.textContent ?? '').trim()).toBe(staffTitle());
  });

  it('the hours line, the refresh button and the report link keep their S-3 conditions, in the shell\'s main area', async () => {
    // with hours and a report permission: hours as 10:00–20:00 Bangkok, refresh, the report link
    await signedIn(['FinanceReconcile']);
    const area = mainArea();
    expect(area, "the shell's main area").not.toBeNull();
    const hours = area!.querySelector('[data-testid="staff-hours"]');
    expect(hours, 'the hours line in the main area').not.toBeNull();
    expect(hours!.textContent ?? '').toContain('10:00');
    expect(hours!.textContent ?? '').toContain('20:00');
    expect(area!.querySelector('[data-testid="staff-refresh"]'), 'refresh in the main area').not.toBeNull();
    expect(area!.querySelector('a[href="/staff/report"]'), 'the report link in the main area').not.toBeNull();

    dispose?.();
    dispose = undefined;
    host.innerHTML = '';

    // no hours and no report permission: the hours sentence without a time, refresh, no report link
    await signedIn([], [], FLOOR_BASE);
    const area2 = mainArea();
    expect(area2, "the shell's main area").not.toBeNull();
    const hours2 = area2!.querySelector('[data-testid="staff-hours"]');
    expect(hours2).not.toBeNull();
    expect(hours2!.textContent ?? '').not.toMatch(/\d\d:\d\d/);
    expect(area2!.querySelector('[data-testid="staff-refresh"]')).not.toBeNull();
    expect(host.querySelector('a[href="/staff/report"]'), 'no report link without the permission').toBeNull();
  });

  it('the cards are still mounted from the registry, in order, inside the shell', async () => {
    await signedIn([], [card('shift', 20), card('floor', 10)]);
    await vi.waitFor(() => expect(host.querySelectorAll('[data-card]')).toHaveLength(2));
    const area = mainArea();
    expect(area, "the shell's main area").not.toBeNull();
    const mounted = Array.from(area!.querySelectorAll('[data-card]')).map((e) => e.getAttribute('data-card'));
    expect(mounted).toEqual(['floor', 'shift']);
    expect(area!.querySelector('[data-testid="body-floor"]')).not.toBeNull();
  });
});
