// What this pins: the staff shell on a phone. The branch menu and the bell stay reachable at 760px or less, the panel's
// sign-out cannot run twice, one Esc handler closes the menu and returns focus, and the panel follows the menu button.
// OL-1R-b (#6451), docs/design/xchange-one-look.md §2.3
// Written before the implementation by claude/f6a909ac for sub-controller claude/1cc51615. Do not edit, delete or weaken.
//
// One test per row of the ticket's §9 behaviour table. Rows 2 and 8 ("unchanged from 4e48ae24") pass today and stay green;
// the others are red today.
//
// jsdom has no layout, so the width rows read `shell.css` as text (as QueueCard.float.test.tsx does) and assert the
// declarations under `@media (max-width: 760px)`, and the Tab row asserts document order. Neither measures a real
// browser.
//
// The shared session is a page singleton, so every test loads fresh modules. Nothing here is JSX: the test builds the
// shell with the freshly loaded Solid, so both sides share one Solid instance (as AppShell.lock.test.tsx does).
//
// Restated for OL-6 L4 (#6537), issuecomment-6688832431, docs/design/xchange-one-look-layout.md §2 Q1: rows 1 and 2 pinned
// the wrapping bar before the header. The bar now sits inside the one header, and only the brand, the language switch and
// the menu button show in the header row at 760px or less. Rows 3 to 8 are unchanged.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Me } from '../api/generated/common';
import { meOf } from '../../test-support/me';
import { makeBranchRef } from '../testing/fixtures';
import { warmModules } from '../testing/warm-modules';

const PHONE = '(max-width:760px)';
const VIEWPORT_PX = 375;

const READY = {
  status: 'ok',
  database: true,
  version: '1.0.0',
  build: '0123456789abcdef0123456789abcdef01234567',
  builtAt: '2026-10-08T01:02:03Z',
  channel: 'staging',
  ui: 'built',
};
const OWNER: Me = meOf({
  id: 'owner',
  role: 'OWNER',
  branch: 'MAIN',
  name: 'Somchai',
  branchName: 'สำนักงานใหญ่',
  branches: [
    makeBranchRef({ id: 'MAIN', code: 'XC-01', name: 'สำนักงานใหญ่' }),
    makeBranchRef({ id: 'B2', code: 'XC-02', name: 'สาขาสอง' }),
  ],
});

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

/** /health/ready, the me view as the owner, 401 for any other view read; the logout answers when the test says so. */
function stubNetwork() {
  const logouts: Array<(res: Response) => void> = [];
  const urls: string[] = [];
  const fetch = vi.fn(async (input: string) => {
    const url = String(input);
    urls.push(url);
    if (url.startsWith('/health/ready')) return json(200, READY);
    if (url.startsWith('/api/v3/views/me')) return json(200, { version: 3, changed: true, data: OWNER });
    if (url.startsWith('/api/auth/logout')) return new Promise<Response>((resolve) => logouts.push(resolve));
    return json(401, { error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } });
  });
  vi.stubGlobal('fetch', fetch);
  return { logouts, urls };
}

let cleanup: Array<() => void> = [];

async function load() {
  vi.resetModules();
  const web = await import('solid-js/web');
  const solid = await import('solid-js');
  const { AppShell } = await import('./AppShell');
  const session = await import('../session/session');
  const { th } = await import('../i18n/th');
  return { web, solid, AppShell, session, th };
}

type Loaded = Awaited<ReturnType<typeof load>>;

function mount(m: Loaded, props: { staffSession?: boolean } = {}): HTMLElement {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const nav = (): HTMLElement => {
    const n = document.createElement('nav');
    n.innerHTML = '<a href="#one" id="nav-one">หนึ่ง</a><a href="#two" id="nav-two">สอง</a>';
    return n;
  };
  const content = (): HTMLElement => {
    const s = document.createElement('section');
    s.innerHTML = '<a href="#content" id="content-link">เนื้อหา</a>';
    return s;
  };
  const dispose = m.web.render(
    () =>
      m.solid.createComponent(m.AppShell, {
        title: 'คีย์ย้อนหลัง',
        ...props,
        get nav() {
          return nav();
        },
        get children() {
          return content();
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

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

function menuButton(host: HTMLElement): HTMLButtonElement {
  const buttons = host.querySelectorAll<HTMLButtonElement>('button[aria-controls]');
  expect(buttons, 'the shell has exactly one menu button with aria-controls').toHaveLength(1);
  return buttons[0]!;
}

function panelOf(button: HTMLButtonElement): HTMLElement {
  const panel = document.getElementById(button.getAttribute('aria-controls')!);
  expect(panel, 'aria-controls names the panel').not.toBeNull();
  return panel!;
}

function panelSignOut(m: Loaded, panel: HTMLElement): HTMLButtonElement {
  const label = m.th['shell.account.signOut'];
  const button = Array.from(panel.querySelectorAll('button')).find((b) => b.textContent?.trim() === label);
  expect(button, 'the sign-out button is in the panel').toBeDefined();
  return button!;
}

function pressEscape(target: EventTarget) {
  target.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
}

// ── the stylesheet and the source, read as text ──────────────────────────────

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

/** Flat rules of a stylesheet: one entry per selector of each rule, inside at most one @media. */
function parseCss(text: string): Rule[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const norm = (s: string) => s.replace(/\s+/g, '');
  const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const block = (body: string, media: string) => {
    const re = /([^{}]+)\{([^{}]*)\}/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(body)) !== null) {
      const decls = new Map<string, string>();
      for (const d of m[2]!.split(';')) {
        const k = d.indexOf(':');
        if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
      }
      for (const sel of m[1]!.split(',')) out.push({ media, selector: normSel(sel), decls });
    }
  };
  let rest = '';
  let i = 0;
  while (i < src.length) {
    const at = src.indexOf('@media', i);
    if (at < 0) {
      rest += src.slice(i);
      break;
    }
    rest += src.slice(i, at);
    const open = src.indexOf('{', at);
    let depth = 1;
    let j = open + 1;
    while (j < src.length && depth > 0) {
      if (src[j] === '{') depth += 1;
      if (src[j] === '}') depth -= 1;
      j += 1;
    }
    block(src.slice(open + 1, j - 1), norm(src.slice(at + 6, open)));
    i = j;
  }
  block(rest, '');
  return out;
}

/** The value of `prop` for exactly `selector` under `media` ('' = no media), the last one winning. */
function value(rules: Rule[], selector: string, prop: string, media = ''): string | undefined {
  let v: string | undefined;
  for (const r of rules) if (r.selector === selector && r.media === media && r.decls.has(prop)) v = r.decls.get(prop);
  return v;
}

/** What applies at 760px or less: the media rule when it sets `prop`, else the base rule. */
function onPhone(rules: Rule[], selector: string, prop: string): string | undefined {
  return value(rules, selector, prop, PHONE) ?? value(rules, selector, prop);
}

// Restated for OL-6 L4 (#6537): what shows at a width comes from shell.css's own rules, because jsdom has no layout and
// no media queries. Base rules cascade first, then the 760px ones, as in the file. Only `display` and `hidden` count.
type Width = 'phone' | 'wide';

function specificity(selector: string): number {
  const s = selector.replace(/"[^"]*"/g, '');
  const ids = (s.match(/#[\w-]+/g) ?? []).length;
  const classes = (s.match(/\.[\w-]+|\[[^\]]*\]|(?<!:):(?!not\b)[\w-]+/g) ?? []).length;
  const types = (s.replace(/:[\w-]+|\.[\w-]+|\[[^\]]*\]|#[\w-]+/g, ' ').match(/[a-z][\w-]*/gi) ?? []).length;
  return ids * 10000 + classes * 100 + types;
}

function matches(el: Element, selector: string): boolean {
  if (selector.includes('::')) return false;
  try {
    return el.matches(selector);
  } catch {
    return false;
  }
}

function displayOf(rules: Rule[], el: Element, width: Width): string {
  const applies = (r: Rule) => r.media === '' || (width === 'phone' && r.media === PHONE);
  const ordered = [...rules.filter((r) => r.media === ''), ...rules.filter((r) => r.media !== '')];
  let best: { important: boolean; spec: number; value: string } | undefined;
  for (const r of ordered) {
    const raw = r.decls.get('display');
    if (raw === undefined || !applies(r) || !matches(el, r.selector)) continue;
    const important = raw.includes('!important');
    const spec = specificity(r.selector);
    if (!best || (important && !best.important) || (important === best.important && spec >= best.spec)) {
      best = { important, spec, value: raw.replace('!important', '').trim() };
    }
  }
  return best?.value ?? '';
}

// Restated for OL-6 L5 (#6542, added 23:26 ICT): the closed account area may hide with `visibility: hidden` out of flow,
// so a fixed toast inside it can stay visible. `visibility` inherits, so the nearest element that sets it decides; a
// `visibility: hidden` element still in flow would leave a gap in the row, and it does not count as hidden.
function visibilityOf(rules: Rule[], el: Element, width: Width): { visibility: string; outOfFlow: boolean } {
  const applies = (r: Rule) => r.media === '' || (width === 'phone' && r.media === PHONE);
  const last = (e: Element, prop: string) => {
    let best: { spec: number; value: string } | undefined;
    for (const r of [...rules.filter((x) => x.media === ''), ...rules.filter((x) => x.media !== '')]) {
      const raw = r.decls.get(prop);
      if (raw === undefined || !applies(r) || !matches(e, r.selector)) continue;
      const spec = specificity(r.selector);
      if (!best || spec >= best.spec) best = { spec, value: raw };
    }
    return best?.value;
  };
  for (let e: Element | null = el; e; e = e.parentElement) {
    const visibility = last(e, 'visibility');
    if (visibility !== undefined) {
      return { visibility, outOfFlow: ['absolute', 'fixed'].includes(last(e, 'position') ?? '') };
    }
  }
  return { visibility: 'visible', outOfFlow: false };
}

function shown(rules: Rule[], el: Element, width: Width): boolean {
  for (let e: Element | null = el; e; e = e.parentElement) {
    if ((e as HTMLElement).hidden || displayOf(rules, e, width) === 'none') return false;
  }
  const { visibility, outOfFlow } = visibilityOf(rules, el, width);
  return !(visibility === 'hidden' && outOfFlow);
}

function shownUserNames(rules: Rule[], host: HTMLElement, width: Width, name: string): number {
  const leaves = Array.from(host.querySelectorAll('*')).filter((e) => e.children.length === 0 && e.textContent?.includes(name));
  return leaves.filter((e) => shown(rules, e, width)).length;
}

function shownSignOuts(m: Loaded, rules: Rule[], host: HTMLElement, width: Width): number {
  const label = m.th['shell.account.signOut'];
  const buttons = Array.from(host.querySelectorAll('button')).filter((b) => b.textContent?.trim() === label);
  return buttons.filter((b) => shown(rules, b, width)).length;
}

function expectDecls(rules: Rule[], selector: string, media: string, want: Record<string, string>) {
  for (const [prop, v] of Object.entries(want)) {
    expect(value(rules, selector, prop, media), `${media || '(no media)'} ${selector} { ${prop} }`).toBe(v);
  }
}

async function readSibling(name: string): Promise<string> {
  const fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
  const path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
  const here = path.dirname(new URL(import.meta.url).pathname);
  return fs.readFileSync(path.join(here, name), 'utf8');
}

async function sheet(): Promise<Rule[]> {
  return parseCss(await readSibling('shell.css'));
}

// ── setup ────────────────────────────────────────────────────────────────────

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

// ── §9 ───────────────────────────────────────────────────────────────────────

describe('AppShell on a phone (OL-1R-b, #6451)', () => {
  // Restated for OL-6 L4 (#6537), §2 Q1: the bar no longer shows and wraps in the page at 760px or less. The header row
  // shows the brand, the language switch and the menu button; the open panel reaches the branch menu and the bell.
  it('row 1: staff session at 375px shows brand, language switch and menu button in the header row; the open panel reaches the branch menu and bell; one user name, one sign-out; nothing wider than the viewport', async () => {
    stubNetwork();
    const m = await load();
    const host = mount(m);
    await signedIn(host);
    const bar = host.querySelector<HTMLElement>('.account-bar')!;
    expect(bar.querySelector('button[aria-haspopup="menu"]'), 'the branch menu is in the bar').not.toBeNull();
    expect(bar.querySelector('.notice-bell'), 'the bell is in the bar').not.toBeNull();

    const rules = await sheet();
    const header = host.querySelector<HTMLElement>('header.xc-shell-header')!;
    const branchMenu = bar.querySelector('button[aria-haspopup="menu"]')!;
    const bell = bar.querySelector('.notice-bell')!;
    const inRow = Array.from(header.children).filter((c) => shown(rules, c, 'phone'));
    expect(inRow.map((c) => c.classList[0] ?? c.tagName).sort(), 'the header row with the menu closed').toEqual([
      'xc-lang-switch',
      'xc-shell-brand',
      'xc-shell-menu-btn',
    ]);
    expect(shown(rules, branchMenu, 'phone'), 'the branch menu is not in the closed header').toBe(false);
    expect(shown(rules, bell, 'phone'), 'the bell is not in the closed header').toBe(false);

    menuButton(host).click();
    await settle();
    expect(shown(rules, branchMenu, 'phone'), 'the open panel reaches the branch menu').toBe(true);
    expect(shown(rules, bell, 'phone'), 'the open panel reaches the bell').toBe(true);
    expect(shownUserNames(rules, host, 'phone', 'Somchai'), 'one visible user name').toBe(1);
    expect(shownSignOuts(m, rules, host, 'phone'), 'one visible sign-out').toBe(1);

    expect(
      onPhone(rules, '.account-bar button', 'max-width'),
      'a nowrap button with a long branch name stays inside the bar',
    ).toBe('100%');
    for (const r of rules) {
      if (!r.selector.startsWith('.account-bar') || (r.media !== '' && r.media !== PHONE)) continue;
      for (const prop of ['width', 'min-width']) {
        const px = /^(\d+(?:\.\d+)?)px$/.exec(r.decls.get(prop) ?? '');
        if (px) {
          expect(Number(px[1]), `${r.selector} { ${prop} } fits ${VIEWPORT_PX}px`).toBeLessThanOrEqual(VIEWPORT_PX);
        }
      }
      expect(r.decls.get('width'), `${r.selector} { width } does not add a scrollbar's width`).not.toBe('100vw');
    }
  });

  it('row 2: staff session at 1280px has the bar inside the header, one user name and one sign-out, and the header rules unchanged', async () => {
    stubNetwork();
    const m = await load();
    const host = mount(m);
    await signedIn(host);
    const bar = host.querySelector<HTMLElement>('.account-bar')!;
    const header = host.querySelector<HTMLElement>('header')!;
    // Restated for OL-6 L4 (#6537), §2 Q1: the bar is inside the one header, not before it.
    expect(header.contains(bar), 'the bar is inside the header').toBe(true);
    expect(header.querySelector('#nav-one'), 'the nav is in the header').not.toBeNull();

    const rules = await sheet();
    // Restated for OL-6 L4 (#6537), §2 Q1: the bar's own 10px 24px padding was the dark bar's; the header has the padding now.
    expectDecls(rules, '.account-bar', '', {
      display: 'flex',
      'flex-wrap': 'wrap',
      gap: '16px',
    });
    // Restated for OL-6 L4 (#6537), §3 item 3: one user name and one sign-out at any width.
    expect(shownUserNames(rules, host, 'wide', 'Somchai'), 'one visible user name').toBe(1);
    expect(shownSignOuts(m, rules, host, 'wide'), 'one visible sign-out').toBe(1);
    expectDecls(rules, '.account-bar button', '', { 'white-space': 'nowrap', 'min-height': '36px' });
    expectDecls(rules, '.xc-shell-header', '', { display: 'flex', padding: '14px 24px' });
    expectDecls(rules, '.xc-shell-menu-btn', '', { display: 'none', order: '4' });
    expectDecls(rules, '.xc-shell-panel', '', { display: 'flex', order: '2' });
    expectDecls(rules, '.xc-shell-panel-account', '', { display: 'none' });
    expectDecls(rules, '.xc-shell-panel-footer', '', { display: 'none' });
  });

  it('row 3: panel sign-out tapped twice while the logout is pending sends one logout, and the button is disabled', async () => {
    const net = stubNetwork();
    const m = await load();
    const host = mount(m);
    await signedIn(host);
    const button = menuButton(host);
    button.click();
    const signOut = panelSignOut(m, panelOf(button));

    signOut.click();
    signOut.click();
    await settle();

    expect(
      net.urls.filter((u) => u.startsWith('/api/auth/logout')),
      'exactly one logout call',
    ).toHaveLength(1);
    expect(signOut.disabled, 'disabled while the logout is pending').toBe(true);

    // A failed logout stays on this page, so the test does not navigate jsdom.
    for (const answer of net.logouts) answer(json(500, { error: { code: 'serverError', message: 'x' } }));
    await settle();
  });

  it('row 4: panel sign-out when the logout answers 500 keeps the panel open, shows the failure, and enables the button again', async () => {
    const net = stubNetwork();
    const m = await load();
    const host = mount(m);
    await signedIn(host);
    const button = menuButton(host);
    button.click();
    const panel = panelOf(button);
    const signOut = panelSignOut(m, panel);

    signOut.click();
    await settle();
    expect(signOut.disabled, 'disabled while the logout is pending').toBe(true);

    expect(net.logouts).toHaveLength(1);
    net.logouts[0]!(json(500, { error: { code: 'serverError', message: 'x' } }));
    await settle();

    expect(panel.getAttribute('data-open'), 'the panel stays open').toBe('true');
    expect(button.getAttribute('aria-expanded')).toBe('true');
    expect(panel.querySelector('[role="alert"]')?.textContent).toBe(m.th['shell.account.signOutFailed']);
    expect(signOut.disabled, 'enabled again after the failure').toBe(false);
  });

  it('row 5: Esc with the menu open closes it once and puts focus on the menu button', async () => {
    stubNetwork();
    const host = mount(await load());
    await signedIn(host);
    const button = menuButton(host);
    const link = host.querySelector<HTMLAnchorElement>('#nav-one')!;

    // Focus inside the panel.
    button.click();
    link.focus();
    pressEscape(link);
    expect(button.getAttribute('aria-expanded'), 'Esc from inside the panel closes the menu').toBe('false');
    expect(document.activeElement, 'focus is back on the menu button').toBe(button);

    // Focus outside the header.
    button.click();
    (document.activeElement as HTMLElement | null)?.blur();
    pressEscape(document.body);
    expect(button.getAttribute('aria-expanded'), 'Esc from outside the header closes the menu').toBe('false');
    expect(document.activeElement, 'focus is back on the menu button').toBe(button);

    // A closed menu takes no focus.
    const content = host.querySelector<HTMLAnchorElement>('#content-link')!;
    content.focus();
    pressEscape(content);
    expect(document.activeElement, 'Esc with the menu closed leaves focus alone').toBe(content);

    const source = await readSibling('AppShell.tsx');
    expect(source.match(/['"]Escape['"]/g) ?? [], 'AppShell.tsx has one Esc handler').toHaveLength(1);
  });

  it('row 6: Tab from the open menu button enters the panel', async () => {
    stubNetwork();
    const host = mount(await load());
    await signedIn(host);
    const button = menuButton(host);
    button.click();
    const panel = panelOf(button);

    const focusable = Array.from(
      host.querySelectorAll<HTMLElement>('a[href], button:not([disabled]), input, select, textarea'),
    );
    const next = focusable[focusable.indexOf(button) + 1];
    expect(next, 'something follows the menu button').toBeDefined();
    expect(panel.contains(next!), `the next stop is in the panel, not ${next!.outerHTML.slice(0, 80)}`).toBe(true);
    expect(button.nextElementSibling, 'the panel comes right after the menu button').toBe(panel);
  });

  it('row 7: body:has(.xc-shell) sets only margin: 0', async () => {
    const declared: Record<string, string> = {};
    for (const r of await sheet()) {
      if (r.selector === 'body:has(.xc-shell)') for (const [k, v] of r.decls) declared[k] = v;
    }
    expect(declared).toEqual({ margin: '0' });
  });

  it('row 8: staffSession={false} is unchanged', async () => {
    const net = stubNetwork();
    const host = mount(await load(), { staffSession: false });
    await settle();

    expect(host.querySelector('.account-bar'), 'no account bar').toBeNull();
    expect(host.querySelector('button[aria-controls]'), 'no menu button').toBeNull();
    expect(
      net.urls.some((u) => u.startsWith('/api/v3/views/me')),
      'the staff me view is not read',
    ).toBe(false);

    const header = host.querySelector<HTMLElement>('header')!;
    const shown = Array.from(header.children).filter((c) => !c.classList.contains('xc-shell-panel'));
    expect(shown[0]?.className, 'the nav stays first in the header').toBe('xc-shell-nav');
    expect(shown[0]?.querySelector('#nav-one'), 'the nav stays in the header').not.toBeNull();
    expect(shown[1]?.tagName, 'then the title').toBe('H1');
    expect(shown[1]?.textContent).toBe('คีย์ย้อนหลัง');
    expect(shown).toHaveLength(3);
  });
});
