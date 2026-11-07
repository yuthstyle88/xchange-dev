// OL-1 L3 (#6360; docs/design/xchange-one-look.md §2.3 "Mobile menu"): at 760px or less the shell header keeps the
// brand and one menu button. The button expands a panel holding the nav and the account content with sign-out, and
// collapses it again. Written before the implementation by the lane controller. Do not edit, delete or weaken.
//
// What this test pins, in `web/src/shared/ui/AppShell.tsx`:
// - exactly one `button[aria-controls]` in the shell, starting with `aria-expanded="false"`;
// - its `aria-controls` names an element in the page (the panel) that holds the nav and the sign-out button;
// - a click expands it (`aria-expanded="true"`), a second click collapses it;
// - Esc collapses it, and choosing a nav link collapses it.
// The width breakpoint is CSS; jsdom has no layout, so this pins the behaviour only.
// It does not pin where the panel sits relative to `<header>`: AppShell.lock.test.tsx still pins `.account-bar`
// before the header and outside it.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Me } from '../api/generated/common';
import { meOf } from '../../test-support/me';
import { warmModules } from '../testing/warm-modules';

const SIGN_OUT_TH = 'ออกจากระบบ';

const READY = {
  status: 'ok',
  database: true,
  version: '1.0.0',
  build: '0123456789abcdef0123456789abcdef01234567',
  builtAt: '2026-10-08T01:02:03Z',
  channel: 'staging',
  ui: 'built',
};
const OWNER: Me = meOf({ id: 'owner', role: 'OWNER', branch: 'MAIN', name: 'Somchai', branchName: 'MAIN', branches: [] });

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

/** /health/ready, the me view as the owner, and 401 for any other view read. */
function stubNetwork() {
  const fetch = vi.fn(async (input: string) => {
    const url = String(input);
    if (url.startsWith('/health/ready')) return json(200, READY);
    if (url.startsWith('/api/v3/views/me')) return json(200, { version: 3, changed: true, data: OWNER });
    return json(401, { error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } });
  });
  vi.stubGlobal('fetch', fetch);
  return fetch;
}

let cleanup: Array<() => void> = [];

async function load() {
  vi.resetModules();
  const web = await import('solid-js/web');
  const solid = await import('solid-js');
  const { AppShell } = await import('./AppShell');
  const session = await import('../session/session');
  return { web, solid, AppShell, session };
}

type Loaded = Awaited<ReturnType<typeof load>>;

function mount(m: Loaded): HTMLElement {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const nav = (): HTMLElement => {
    const n = document.createElement('nav');
    n.innerHTML = '<a href="#one" id="nav-one">หนึ่ง</a><a href="#two" id="nav-two">สอง</a>';
    return n;
  };
  const dispose = m.web.render(
    () =>
      m.solid.createComponent(m.AppShell, {
        title: 'คีย์ย้อนหลัง',
        get nav() {
          return nav();
        },
        get children() {
          return document.createElement('section');
        },
      }),
    host,
  );
  cleanup.push(dispose, () => m.session.sharedSession().dispose());
  return host;
}

async function signedIn(host: HTMLElement) {
  await vi.waitFor(() => {
    expect(host.querySelector('.account-bar')).not.toBeNull();
  });
}

function menuButton(host: HTMLElement): HTMLButtonElement {
  const buttons = host.querySelectorAll<HTMLButtonElement>('button[aria-controls]');
  expect(buttons, 'the shell has exactly one menu button with aria-controls').toHaveLength(1);
  return buttons[0]!;
}

function panelOf(button: HTMLButtonElement): HTMLElement {
  const id = button.getAttribute('aria-controls');
  expect(id, 'aria-controls names the panel').toBeTruthy();
  const panel = document.getElementById(id!);
  expect(panel, `the panel #${id} is in the page`).not.toBeNull();
  return panel!;
}

function expanded(button: HTMLButtonElement): string | null {
  return button.getAttribute('aria-expanded');
}

beforeAll(
  () =>
    warmModules(
      () => import('solid-js/web'),
      () => import('solid-js'),
      () => import('./AppShell'),
      () => import('../session/session'),
    ),
  60_000,
);

beforeEach(() => {
  try {
    window.localStorage.clear();
  } catch {
    // storage blocked: the language starts as th anyway
  }
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
});

afterEach(() => {
  for (const c of cleanup.reverse()) c();
  cleanup = [];
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
});

describe('AppShell mobile menu (OL-1 L3, #6360)', () => {
  it('has one menu button with aria-controls, collapsed at first', async () => {
    stubNetwork();
    const host = mount(await load());
    await signedIn(host);
    const button = menuButton(host);
    expect(button.getAttribute('type')).toBe('button');
    expect(expanded(button)).toBe('false');
  });

  it('the panel holds the nav and the sign-out button', async () => {
    stubNetwork();
    const host = mount(await load());
    await signedIn(host);
    const panel = panelOf(menuButton(host));
    expect(panel.querySelector('#nav-one'), 'the nav is in the panel').not.toBeNull();
    expect(panel.querySelector('#nav-two'), 'every nav item is in the panel').not.toBeNull();
    const signOut = Array.from(panel.querySelectorAll('button')).find((b) => b.textContent?.trim() === SIGN_OUT_TH);
    expect(signOut, 'the sign-out button is in the panel').toBeDefined();
  });

  it('a click expands it and a second click collapses it', async () => {
    stubNetwork();
    const host = mount(await load());
    await signedIn(host);
    const button = menuButton(host);
    button.click();
    expect(expanded(button)).toBe('true');
    button.click();
    expect(expanded(button)).toBe('false');
  });

  it('Esc collapses it', async () => {
    stubNetwork();
    const host = mount(await load());
    await signedIn(host);
    const button = menuButton(host);
    button.click();
    expect(expanded(button)).toBe('true');
    const link = panelOf(button).querySelector<HTMLAnchorElement>('#nav-one')!;
    link.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(expanded(button)).toBe('false');
  });

  it('choosing a nav link collapses it', async () => {
    stubNetwork();
    const host = mount(await load());
    await signedIn(host);
    const button = menuButton(host);
    button.click();
    expect(expanded(button)).toBe('true');
    panelOf(button).querySelector<HTMLAnchorElement>('#nav-two')!.click();
    expect(expanded(button)).toBe('false');
  });
});
