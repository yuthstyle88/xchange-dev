// Up-front tests: the admin mobile menu folds back like v2's (admin-workspace.js, xchange-v2-source@4d0bdbb)
//
// Owner 2026-10-10: once the menu is open on a narrow screen, tapping the work area must fold it
// back to the menu button. v2 does this with a shade button after the sidebar
// (admin-workspace.js offset ~4253), closes on a nav link click (~5700), on leaving
// `(max-width:900px)` (~4720) and on Escape (~5228), and returns focus to the menu button (~4640).
//
// The harness (fake ApiClient, fetch stub, settle/mount helpers) is copied from AdminShell.test.tsx;
// that file is not edited. matchMedia is a controllable fake here, so a test can say which side of
// 900px the screen is on and fire the `change` event.

//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the `Me` fixtures (`kind`/`role`/`username`) → `meOf(…)` (`web/src/test-support/me.ts`, design §3.3). Nothing else changes. The fixture menus' `roles: 'all'` → `needs: 'all'` (#1030 §3.10).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { ApiClient, ViewAnswer, ViewParams } from '../../shared/api/client';
import type { Me } from '../../shared/api/generated/common';
import { setLang } from '../../shared/i18n';
import { AdminShell } from './AdminShell';
import type { AdminMenu } from './menus';
import { meOf } from '../../test-support/me';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../shared/testing/page-wide';

const MENU_BUTTON = '☰ เมนู'; // admin-workspace.js:60, i18n.ts menuButton
const SHADE_TH = 'ปิดเมนู'; // admin-workspace.js ~4253: shade.setAttribute('aria-label','ปิดเมนู')
const SHADE_EN = 'Close menu';
const SEARCH_PLACEHOLDER = 'ค้นหาเมนู…';

// ── fixture menus ────────────────────────────────────────────────────────────

function fixtureMenu(id: string, order: number, label: string, group: AdminMenu['group']): AdminMenu {
  const Component = (): JSX.Element => <section data-fixture-menu={id}>{`body of ${id}`}</section>;
  return {
    id,
    order,
    label: () => label,
    icon: '▦',
    group,
    needs: 'all',
    heading: { eyebrow: () => 'EYEBROW', title: () => label, subtitle: () => 'SUBTITLE' },
    dated: false,
    Component,
  };
}

function menus(): AdminMenu[] {
  return [fixtureMenu('overview', 1, 'ภาพรวม', 'desk'), fixtureMenu('transactions', 2, 'รายการแลกเงิน', 'money')];
}

// ── the fake ApiClient (as AdminShell.test.tsx) ─────────────────────────────

const BRANCHES = [
  { id: 'MAIN', name: 'สำนักงานใหญ่' },
  { id: 'B1', name: 'สาขาหนึ่ง' },
];

function meFor(named: string | undefined): Me {
  return meOf({ id: 'owner', role: 'OWNER', branch: named || 'MAIN', name: 'Owner', branchName: named || 'MAIN', branches: [] });
}

const fakeApi = {
  async view<T>(name: string, q: ViewParams = {}): Promise<ViewAnswer<T>> {
    if (name === 'me') return { version: 3, changed: true, data: meFor(q.branch) as T };
    if (name === 'admin_branches') {
      return { version: 3, changed: true, data: { branches: BRANCHES.map((b) => ({ ...b })) } as T };
    }
    throw new Error(`the shell reads no view ${name}`);
  },
  async page<R>(name: string): Promise<R[]> {
    throw new Error(`the shell reads no page ${name}`);
  },
  async act<B, R>(area: string, action: string, _body: B): Promise<R> {
    throw new Error(`the shell calls no v3 action ${area}/${action}`);
  },
  async post<B, R>(_path: string, _body: B): Promise<R> {
    throw new Error('the menu tests post nothing');
  },
};

const api = fakeApi as unknown as ApiClient;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

// ── a controllable matchMedia ────────────────────────────────────────────────

type Listener = (e: { matches: boolean; media: string }) => void;

interface FakeMql {
  media: string;
  matches: boolean;
  listeners: Set<Listener>;
  onchange: Listener | null;
}

let narrow = true;
let mqls: FakeMql[] = [];

function is900(query: string): boolean {
  return /max-width:\s*900px/.test(query);
}

/** Moves the screen across 900px and fires `change` on every 900px query the page made. */
function setNarrow(next: boolean): void {
  narrow = next;
  for (const m of mqls) {
    if (!is900(m.media)) continue;
    m.matches = next;
    const e = { matches: next, media: m.media };
    for (const l of Array.from(m.listeners)) l(e);
    m.onchange?.(e);
  }
}

function fakeMatchMedia(query: string) {
  const m: FakeMql = { media: query, matches: is900(query) ? narrow : true, listeners: new Set(), onchange: null };
  mqls.push(m);
  return {
    get matches() {
      return m.matches;
    },
    media: query,
    get onchange() {
      return m.onchange;
    },
    set onchange(v: Listener | null) {
      m.onchange = v;
    },
    addEventListener: (_type: string, l: Listener) => m.listeners.add(l),
    removeEventListener: (_type: string, l: Listener) => m.listeners.delete(l),
    addListener: (l: Listener) => m.listeners.add(l),
    removeListener: (l: Listener) => m.listeners.delete(l),
    dispatchEvent: () => false,
  };
}

// ── rendering ────────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;
let container: HTMLDivElement;

const realSetTimeout = globalThis.setTimeout;

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

async function mount(url = '/ui/admin/#overview'): Promise<void> {
  window.history.replaceState(null, '', url);
  container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(() => <AdminShell api={api} menus={menus()} />, container);
  await settle();
}

function menuButton(): HTMLButtonElement {
  const found = Array.from(document.body.querySelectorAll('button')).filter(
    (b) => (b.textContent ?? '').trim() === MENU_BUTTON,
  );
  expect(found, `one ${MENU_BUTTON} button`).toHaveLength(1);
  return found[0]!;
}

function sidebar(): HTMLElement {
  const id = menuButton().getAttribute('aria-controls');
  expect(id, 'the menu button names the sidebar in aria-controls').toBeTruthy();
  const el = document.getElementById(id!);
  expect(el, 'the controlled sidebar exists').not.toBeNull();
  return el!;
}

function shades(): HTMLElement[] {
  return Array.from(document.body.querySelectorAll<HTMLElement>('.workspace-shade'));
}

/** The one shade, asserted present (so a missing shade fails on an assertion, not a TypeError). */
function shade(): HTMLElement {
  const found = shades();
  expect(found, 'one shade while the menu is open').toHaveLength(1);
  return found[0]!;
}

function isOpen(): boolean {
  const expanded = menuButton().getAttribute('aria-expanded');
  const open = sidebar().classList.contains('open');
  expect(open, 'the sidebar `open` class (admin.css .admin-sidebar.open) follows aria-expanded').toBe(
    expanded === 'true',
  );
  return open;
}

async function openMenu(): Promise<void> {
  menuButton().click();
  await settle();
  expect(isOpen(), 'the menu button opens the menu').toBe(true);
}

function navLink(href: string): HTMLAnchorElement {
  const a = sidebar().querySelector<HTMLAnchorElement>(`a[href="${href}"]`);
  expect(a, `a sidebar link to ${href}`).not.toBeNull();
  return a!;
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-10-08T05:00:00Z'));
  setLang('th');
  narrow = true;
  mqls = [];
  try {
    window.localStorage.clear();
  } catch {
    // storage blocked
  }
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: string) => {
      const url = String(input);
      if (url.includes('/health/ready')) {
        return jsonResponse({ version: '3.0.0', build: 'abcdef1234567', channel: 'staging', builtAt: '2026-10-08T00:00:00Z' });
      }
      if (url.startsWith('/api/v3/views/me')) return jsonResponse({ version: 1, changed: true, data: meFor(undefined) });
      if (url.startsWith(QUEUE_CARD_PATH)) return jsonResponse(queueCardUnchanged());
      return jsonResponse({ error: { code: 'notFound', message: 'not found' } }, 404);
    }),
  );
  vi.stubGlobal('matchMedia', vi.fn(fakeMatchMedia));
});

afterEach(async () => {
  await settle();
  if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(10_000);
  dispose?.();
  dispose = undefined;
  container?.remove();
  document.body.innerHTML = '';
  vi.useRealTimers();
  vi.unstubAllGlobals();
  setLang('th');
  window.history.replaceState(null, '', '/');
});

describe('Admin mobile menu folds back like v2 (owner 2026-10-10)', () => {
  it(`renders the shade only while the menu is open, as a button "${SHADE_TH}" inside .xc-content .workspace-menu-open (admin.css)`, async () => {
    await mount();
    expect(isOpen()).toBe(false);
    expect(shades(), 'no shade while the menu is closed').toHaveLength(0);

    await openMenu();
    const found = shades();
    expect(found, 'one shade while the menu is open').toHaveLength(1);
    const shade = found[0]!;
    expect(shade.tagName, 'the shade is a button, as v2').toBe('BUTTON');
    expect(shade.getAttribute('aria-label')).toBe(SHADE_TH);
    expect(
      shade.closest('.xc-content .workspace-menu-open'),
      'admin.css shows the shade only under `.xc-content .workspace-menu-open`',
    ).not.toBeNull();
  });

  it(`names the shade "${SHADE_EN}" in English (W10)`, async () => {
    await mount();
    setLang('en');
    await settle();
    const b = Array.from(document.body.querySelectorAll('button')).find(
      (x) => (x.textContent ?? '').trim() === '☰ Menu',
    );
    expect(b, 'the menu button').toBeDefined();
    b!.click();
    await settle();
    const found = shades();
    expect(found).toHaveLength(1);
    expect(found[0]!.getAttribute('aria-label')).toBe(SHADE_EN);
  });

  it('closes the menu when the shade (the page behind it) is tapped, and returns focus to the menu button', async () => {
    await mount();
    await openMenu();
    shade().click();
    await settle();
    expect(isOpen(), 'tapping the shade folds the menu').toBe(false);
    expect(menuButton().getAttribute('aria-expanded')).toBe('false');
    expect(shades(), 'the shade goes away with the menu').toHaveLength(0);
    expect(document.activeElement, 'focus returns to the menu button (admin-workspace.js openMenu(false))').toBe(
      menuButton(),
    );
  });

  it('returns focus to the menu button when Escape closes the menu', async () => {
    await mount();
    await openMenu();
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await settle();
    expect(isOpen()).toBe(false);
    expect(shades()).toHaveLength(0);
    expect(document.activeElement).toBe(menuButton());
  });

  it('closes the menu when a menu link is clicked, and the page follows the link', async () => {
    await mount();
    await openMenu();
    navLink('#transactions').click();
    window.location.hash = '#transactions';
    await settle();
    expect(isOpen(), 'a nav link click folds the menu (admin-workspace.js nav click)').toBe(false);
    expect(shades()).toHaveLength(0);
    expect(container.querySelector('[data-fixture-menu="transactions"]'), 'the chosen menu is shown').not.toBeNull();
  });

  it('keeps the menu open on a click inside the sidebar that is not a link (the menu search)', async () => {
    await mount();
    await openMenu();
    const search = sidebar().querySelector<HTMLInputElement>(`input[placeholder="${SEARCH_PLACEHOLDER}"]`);
    expect(search, 'the menu search').not.toBeNull();
    search!.click();
    await settle();
    expect(isOpen(), 'only a link click closes; the search box does not').toBe(true);
    expect(shades()).toHaveLength(1);
  });

  it('closes the menu when the screen grows past 900px (matchMedia change, admin-workspace.js)', async () => {
    await mount();
    await openMenu();
    setNarrow(false);
    await settle();
    expect(isOpen(), 'leaving (max-width: 900px) folds the menu').toBe(false);
    expect(shades()).toHaveLength(0);
  });

  it('never renders the shade at desktop width, even if the menu button is pressed', async () => {
    narrow = false;
    await mount();
    menuButton().click();
    await settle();
    expect(shades(), 'no shade above 900px (admin.css shows it only at max-width: 900px)').toHaveLength(0);
  });

  it('keeps aria-expanded in step with the menu over open, shade close, open, link close', async () => {
    await mount();
    const states: string[] = [];
    const read = () => states.push(menuButton().getAttribute('aria-expanded') ?? 'missing');
    read();
    menuButton().click();
    await settle();
    read();
    shade().click();
    await settle();
    read();
    menuButton().click();
    await settle();
    read();
    navLink('#transactions').click();
    await settle();
    read();
    expect(states).toEqual(['false', 'true', 'false', 'true', 'false']);
  });

  it('marks the sidebar inert while closed on mobile (<=900px), and removes inert when open or at desktop width', async () => {
    await mount();
    expect(isOpen()).toBe(false);
    expect(sidebar().hasAttribute('inert') || (sidebar() as any).inert === true).toBe(true);

    await openMenu();
    expect(sidebar().hasAttribute('inert') || (sidebar() as any).inert === true).toBe(false);

    shade().click();
    await settle();
    expect(sidebar().hasAttribute('inert') || (sidebar() as any).inert === true).toBe(true);

    setNarrow(false);
    await settle();
    expect(sidebar().hasAttribute('inert') || (sidebar() as any).inert === true).toBe(false);
  });

  it('moves focus to the menu search input when the mobile menu opens', async () => {
    await mount();
    const search = sidebar().querySelector<HTMLInputElement>(`input[placeholder="${SEARCH_PLACEHOLDER}"]`);
    expect(search).not.toBeNull();
    expect(document.activeElement).not.toBe(search);

    await openMenu();
    expect(document.activeElement).toBe(search);
  });

  it('traps Tab and Shift+Tab inside the sidebar while the mobile menu is open', async () => {
    await mount();
    await openMenu();

    const focusables = Array.from(
      sidebar().querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    );
    expect(focusables.length).toBeGreaterThan(1);
    const first = focusables[0]!;
    const last = focusables[focusables.length - 1]!;

    first.focus();
    expect(document.activeElement).toBe(first);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true }));
    expect(document.activeElement).toBe(last);

    last.focus();
    expect(document.activeElement).toBe(last);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey: false, bubbles: true }));
    expect(document.activeElement).toBe(first);
  });
});
