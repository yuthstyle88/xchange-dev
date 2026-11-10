// Pins the shared staff nav (one list, each item shown only with its permission), the phone rows, and L4's shell
// follow-ups: the XC mark on the brand, the long title, the brand on /devices, the branch shown once, the notice toast.
// OL-6 L5 (#6542), docs/design/xchange-one-look-layout.md §1 D2, §2 Q2, §3 L5; the map is #6542 issuecomment-6689849443.
// Written before the implementation by claude/f6a909ac for sub-controller claude/1cc51615. Do not edit, delete or weaken.
//
// One test per row of #6542 §9, with the notice-toast row. Restated for #6552: the 5-item map adds ตั้งค่า
// (`/staff/settings`, shown when any of `settingsRead`, `AlertSettingsSave`, `RateAlertSettingsSave` is held).
// `staff-nav.ts` loads through import.meta.glob, so tsc passes before it exists and the tests go red on a wrong value.
//
// jsdom has no layout and no media queries, so what shows at a width comes from shell.css's own rules: display,
// visibility and position only, base rules first and then the 760px ones. Nothing here measures a browser. The shared
// session is a page singleton, so every test loads fresh modules, and nothing mounts as JSX (as AppShell.lock.test.tsx does).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { meOf } from '../../test-support/me';
import { makeBranchRef } from '../testing/fixtures';
import { warmModules } from '../testing/warm-modules';

// `host.innerHTML` captured on the base, 36f743a4.
const CUSTOMER_MARKUP =
  '<div class="xc-shell xc-theme"><header class="xc-shell-header" data-menu-open="false"><div id="xc-shell-menu" class="xc-shell-panel" data-open="false"></div><h1 class="xc-shell-title">คีย์ย้อนหลัง</h1><div role="group" class="xc-lang-switch"><button type="button" aria-pressed="true" aria-label="เปลี่ยนภาษาเป็น ไทย">ไทย</button><button type="button" aria-pressed="false" aria-label="เปลี่ยนภาษาเป็น English">EN</button></div></header><fieldset class="xc-lock" style="border:0;padding:0;margin:0;min-width:0"><main class="xc-shell-main"><section id="the-content"></section></main></fieldset><div class="xc-shell-footer"><footer><div><span>v1.0.0<!----> · 0123456<!----> · staging</span> <span>2026-10-08T01:02:03Z</span></div></footer></div></div>';

type StaffNavItem = {
  key: string;
  href: string;
  permission?: string;
  visible?: (permissions: readonly string[]) => boolean;
  icon: string;
};
type StaffNavModule = {
  STAFF_NAV?: StaffNavItem[];
  visibleStaffNav?: (permissions: readonly string[]) => StaffNavItem[];
  hasVisibleSettings?: (permissions: readonly string[]) => boolean;
};

const staffNavModule = import.meta.glob<StaffNavModule>('./staff-nav.ts', { eager: true })['./staff-nav.ts'];

const SETTINGS_PERMISSIONS = ['settingsRead', 'AlertSettingsSave', 'RateAlertSettingsSave'] as const;

const FOUR_ITEMS = [
  { key: 'shell.nav.counter', href: '/desk', permission: 'ExchangeTransact', icon: '⇄', th: 'เคาน์เตอร์' },
  { key: 'shell.nav.backdated', href: '/desk/backdated', permission: 'exchangeBackdate', icon: '↺', th: 'คีย์ย้อนหลัง' },
  { key: 'shell.nav.vault', href: '/#cash', permission: 'CashManage', icon: '▣', th: 'ตู้เซฟ' },
  { key: 'shell.nav.reports', href: '/#reports', permission: 'CompatRead', icon: '≡', th: 'รายงาน' },
];
const SETTINGS_ITEM = {
  key: 'shell.nav.settings',
  href: '/staff/settings',
  permission: undefined,
  icon: '⚙',
  th: 'ตั้งค่า',
};
const MAP = [...FOUR_ITEMS, SETTINGS_ITEM];
const ALL_PERMISSIONS = ['SignedIn', ...FOUR_ITEMS.map((i) => i.permission), ...SETTINGS_PERMISSIONS];

const READY = {
  status: 'ok',
  database: true,
  version: '1.0.0',
  build: '0123456789abcdef0123456789abcdef01234567',
  builtAt: '2026-10-08T01:02:03Z',
  channel: 'staging',
  ui: 'built',
};
const BRANCH_TEXT = 'สำนักงานใหญ่';

function meWith(permissions: readonly string[], branches: 'one' | 'two' = 'two') {
  const me = meOf({
    id: 'owner',
    role: 'OWNER',
    branch: 'MAIN',
    name: 'Somchai',
    branchName: BRANCH_TEXT,
    branches:
      branches === 'two'
        ? [
            makeBranchRef({ id: 'MAIN', code: 'XC-01', name: BRANCH_TEXT }),
            makeBranchRef({ id: 'B2', code: 'XC-02', name: 'สาขาสอง' }),
          ]
        : [],
  });
  return { ...me, permissions: [...permissions] };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function stubNetwork(me: ReturnType<typeof meWith> = meWith(ALL_PERMISSIONS)) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: string) => {
      const url = String(input);
      if (url.startsWith('/health/ready')) return json(200, READY);
      if (url.startsWith('/api/v3/views/me')) return json(200, { version: 3, changed: true, data: me });
      return json(401, { error: { code: 'signIn', message: 'กรุณาเข้าสู่ระบบ' } });
    }),
  );
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

function mount(
  m: Loaded,
  props: { staffSession?: boolean; staffNav?: boolean; look?: 'devices'; title?: string } = {},
  nav?: () => HTMLElement,
): HTMLElement {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const content = (): HTMLElement => {
    const s = document.createElement('section');
    s.id = 'the-content';
    return s;
  };
  const dispose = m.web.render(
    () =>
      m.solid.createComponent(m.AppShell as (p: Record<string, unknown>) => ReturnType<typeof m.AppShell>, {
        title: 'คีย์ย้อนหลัง',
        ...props,
        ...(nav
          ? {
              get nav() {
                return nav();
              },
            }
          : {}),
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

function navLinks(host: HTMLElement): HTMLAnchorElement[] {
  return Array.from(host.querySelectorAll<HTMLAnchorElement>('header .xc-shell-nav a'));
}

function hrefs(host: HTMLElement): (string | null)[] {
  return navLinks(host).map((a) => a.getAttribute('href'));
}

// ── the stylesheets, read as text (as QueueCard.float.test.tsx does) ─────────

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };
type Width = 'phone' | 'wide';

const PHONE = '(max-width:760px)';

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

async function readHere(relative: string): Promise<string> {
  const fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
  const path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
  const here = path.dirname(new URL(import.meta.url).pathname);
  return fs.readFileSync(path.join(here, relative), 'utf8');
}

async function shellCss(): Promise<Rule[]> {
  return parseCss(await readHere('shell.css'));
}

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

/** The value of `prop` set on `el` itself by the rules that apply at `width`. */
function declared(rules: Rule[], el: Element, prop: string, width: Width): string | undefined {
  const ordered = [...rules.filter((r) => r.media === ''), ...rules.filter((r) => r.media === PHONE && width === 'phone')];
  let best: { important: boolean; spec: number; value: string } | undefined;
  for (const r of ordered) {
    const raw = r.decls.get(prop);
    if (raw === undefined || !matches(el, r.selector)) continue;
    const important = raw.includes('!important');
    const spec = specificity(r.selector);
    if (!best || (important && !best.important) || (important === best.important && spec >= best.spec)) {
      best = { important, spec, value: raw.replace('!important', '').trim() };
    }
  }
  return best?.value;
}

/** `visibility` inherits: the nearest element that sets it decides. */
function visibilityOf(rules: Rule[], el: Element, width: Width): string {
  for (let e: Element | null = el; e; e = e.parentElement) {
    const v = declared(rules, e, 'visibility', width);
    if (v !== undefined) return v;
  }
  return 'visible';
}

function shown(rules: Rule[], el: Element, width: Width): boolean {
  for (let e: Element | null = el; e; e = e.parentElement) {
    if ((e as HTMLElement).hidden || declared(rules, e, 'display', width) === 'none') return false;
  }
  return visibilityOf(rules, el, width) !== 'hidden';
}

/** Every value of `prop` in the rules with exactly this selector, at any media. */
function everyValue(rules: Rule[], selector: string, prop: string, media?: string): string[] {
  return rules
    .filter((r) => r.selector === selector && r.decls.has(prop) && (media === undefined || r.media === media))
    .map((r) => r.decls.get(prop)!);
}

// ── setup ────────────────────────────────────────────────────────────────────

beforeAll(
  () =>
    warmModules(
      () => import('solid-js/web'),
      () => import('solid-js'),
      () => import('./AppShell'),
      () => import('../session/session'),
      () => import('../i18n/th'),
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
  window.history.pushState({}, '', '/');
});

// ── §9 ───────────────────────────────────────────────────────────────────────

describe('StaffNav (OL-6 L5, #6542 §2b, §3 item 1, restated for #6552)', () => {
  it('STAFF_NAV is the §2b map, in order', () => {
    const list = (staffNavModule?.STAFF_NAV ?? []).map(({ key, href, permission, icon }) => ({ key, href, permission, icon }));
    expect(list).toEqual(MAP.map(({ key, href, permission, icon }) => ({ key, href, permission, icon })));
    expect(staffNavModule?.STAFF_NAV?.[4]?.visible).toBeTypeOf('function');
    expect(staffNavModule?.STAFF_NAV?.[4]?.visible).toBe(staffNavModule?.hasVisibleSettings);
  });

  it('visibleStaffNav returns the items whose permission is held, in order', () => {
    const visible = (held: readonly string[]) => (staffNavModule?.visibleStaffNav?.(held) ?? []).map((i) => i.href);
    expect(visible(ALL_PERMISSIONS)).toEqual(MAP.map((i) => i.href));
    expect(visible(['CompatRead', 'ExchangeTransact'])).toEqual(['/desk', '/#reports']);
    for (const perm of SETTINGS_PERMISSIONS) {
      expect(visible([perm]), perm).toEqual(['/staff/settings']);
    }
    expect(visible([])).toEqual([]);
  });

  it('th has the mock-up labels, and en and zh have their own', async () => {
    const m = await load();
    const tables = m.th as unknown as Record<string, string>;
    const { en } = (await import('../i18n/en')) as unknown as { en: Record<string, string> };
    const { zh } = (await import('../i18n/zh')) as unknown as { zh: Record<string, string> };
    for (const item of MAP) {
      expect(tables[item.key], `th ${item.key}`).toBe(item.th);
      for (const [lang, table] of [['en', en], ['zh', zh]] as const) {
        expect(table[item.key], `${lang} ${item.key}`).toBeTypeOf('string');
        expect(table[item.key], `${lang} ${item.key} is not Thai`).not.toMatch(/[฀-๿]/);
      }
    }
  });
});

describe('the staff nav in AppShell (OL-6 L5, #6542 §9, restated for #6552)', () => {
  it.each([
    ['/desk/backdated', '/desk/backdated'],
    ['/desk', '/desk'],
    ['/staff/settings', '/staff/settings'],
    ['/desk/queue', null],
  ])('staff session, no nav, all permissions, on %s: the items in order, and the current page has aria-current', async (path, current) => {
    window.history.pushState({}, '', path);
    stubNetwork();
    const m = await load();
    const host = mount(m);
    await signedIn(host);
    await settle();
    expect(hrefs(host), 'the items, in order').toEqual(MAP.map((i) => i.href));
    navLinks(host).forEach((a, i) => expect(a.textContent, MAP[i]!.key).toContain(MAP[i]!.th));
    const marked = navLinks(host).filter((a) => a.getAttribute('aria-current') === 'page');
    expect(marked.map((a) => a.getAttribute('href'))).toEqual(current === null ? [] : [current]);
    const rules = await shellCss();
    expect(shown(rules, navLinks(host)[0]!, 'wide'), 'inline above 760px').toBe(true);
  });

  it.each(FOUR_ITEMS.map((i) => [i.th, i] as const))('permissions missing the one for %s: that item is absent', async (_th, item) => {
    const held = ALL_PERMISSIONS.filter((p) => p !== item.permission);
    stubNetwork(meWith(held));
    const host = mount(await load());
    await signedIn(host);
    await settle();
    expect(hrefs(host)).toEqual(MAP.filter((i) => i !== item).map((i) => i.href));
  });

  it('permissions missing all settings permissions: ตั้งค่า is absent; any single settings permission shows ตั้งค่า', async () => {
    const withoutSettings = ['SignedIn', ...FOUR_ITEMS.map((i) => i.permission)];
    stubNetwork(meWith(withoutSettings));
    const host = mount(await load());
    await signedIn(host);
    await settle();
    expect(hrefs(host)).toEqual(FOUR_ITEMS.map((i) => i.href));

    for (const perm of SETTINGS_PERMISSIONS) {
      stubNetwork(meWith(['SignedIn', perm]));
      const h = mount(await load());
      await signedIn(h);
      await settle();
      expect(hrefs(h), perm).toEqual(['/staff/settings']);
    }
  });

  it('a screen passes nav: its nav is used, no StaffNav', async () => {
    stubNetwork();
    const host = mount(await load(), {}, () => {
      const n = document.createElement('nav');
      n.innerHTML = '<a href="#own" id="own">own</a>';
      return n;
    });
    await signedIn(host);
    await settle();
    expect(hrefs(host)).toEqual(['#own']);
  });

  it('staffNav={false}: no nav', async () => {
    stubNetwork();
    const host = mount(await load(), { staffNav: false });
    await signedIn(host);
    await settle();
    expect(host.querySelector('.xc-shell-nav')).toBeNull();
    expect(navLinks(host)).toEqual([]);
  });

  it.each([
    ['../../screens/kiosk-currencies/main.tsx'],
    ['../../screens/kiosk-queue/main.tsx'],
  ])('%s passes staffNav={false}', async (file) => {
    expect(await readHere(file)).toMatch(/<AppShell\b[^>]*\bstaffNav=\{false\}/);
  });

  it.each([
    ['a branch switch', 'two'],
    ['a single branch', 'one'],
  ] as const)('at 760px or less, menu open, with %s: each item is a 48px row with its icon, and the branch appears once', async (_name, branches) => {
    stubNetwork(meWith(ALL_PERMISSIONS, branches));
    const m = await load();
    const host = mount(m);
    await signedIn(host);
    host.querySelector<HTMLButtonElement>('.xc-shell-menu-btn')!.click();
    await settle();
    const rules = await shellCss();
    const links = navLinks(host);
    expect(links.map((a) => a.getAttribute('href')), 'the items').toEqual(MAP.map((i) => i.href));
    links.forEach((a, i) => {
      expect(shown(rules, a, 'phone'), `${MAP[i]!.key} shows`).toBe(true);
      expect(declared(rules, a, 'min-height', 'phone'), `${MAP[i]!.key} row height`).toBe('48px');
      expect(a.textContent, `${MAP[i]!.key} icon`).toContain(MAP[i]!.icon);
    });
    const branchText = Array.from(host.querySelectorAll('*')).filter(
      (e) => e.children.length === 0 && e.textContent?.includes(BRANCH_TEXT),
    );
    expect(branchText.filter((e) => shown(rules, e, 'phone')).length, 'the branch appears once').toBe(1);
  });
});

describe('the shell follow-ups from L4 (OL-6 L5, #6542 §9)', () => {
  it('brand: .xc-shell-brand::before shows the XC mark', async () => {
    const rules = await shellCss();
    expect(everyValue(rules, '.xc-shell-brand::before', 'content', '')).toEqual(["'XC'"]);
    expect(everyValue(rules, '.xc-shell-brand::before', 'width', '')).toEqual(['32px']);
    expect(everyValue(rules, '.xc-shell-brand::before', 'height', '')).toEqual(['32px']);
    expect(everyValue(rules, '.xc-shell-brand::before', 'background', '')).toEqual(['var(--xc-brand)']);
  });

  it.each(['wide', 'phone'] as const)('long title, %s: a block, cut with an ellipsis', async (width) => {
    stubNetwork();
    const host = mount(await load());
    await signedIn(host);
    const rules = await shellCss();
    const h1 = host.querySelector('h1.xc-shell-title')!;
    expect(declared(rules, h1, 'display', width), 'display').toBe('block');
    expect(declared(rules, h1, 'overflow', width), 'overflow').toBe('hidden');
    expect(declared(rules, h1, 'text-overflow', width), 'text-overflow').toBe('ellipsis');
    expect(declared(rules, h1, 'white-space', width), 'white-space').toBe('nowrap');
  });

  it('the customer title above 760px keeps its look: the long-title rules stay under staff roots', async () => {
    stubNetwork();
    const host = mount(await load(), { staffSession: false });
    host.classList.add('xc-customer');
    const rules = await shellCss();
    const h1 = host.querySelector('h1.xc-shell-title')!;
    expect(declared(rules, h1, 'display', 'wide'), 'display').toBe('inline-flex');
    expect(declared(rules, h1, 'overflow', 'wide'), 'overflow').toBeUndefined();
    expect(declared(rules, h1, 'text-overflow', 'wide'), 'text-overflow').toBeUndefined();
    expect(declared(rules, h1, 'white-space', 'wide'), 'white-space').toBeUndefined();
  });

  it('/devices at 760px or less: a brand shows in the closed header row; above 760px exactly one shows', async () => {
    stubNetwork();
    const host = mount(await load(), { look: 'devices' }, () => {
      const box = document.createElement('div');
      box.innerHTML = '<a class="xc-shell-brand" href="/devices">XCHANGE</a><nav><a href="/">admin</a></nav>';
      return box;
    });
    await signedIn(host);
    const rules = await shellCss();
    const header = host.querySelector('header.xc-shell-header')!;
    const brands = Array.from(header.querySelectorAll('.xc-shell-brand'));
    expect(brands.filter((b) => shown(rules, b, 'phone')).length, 'one brand in the closed header row').toBe(1);
    expect(brands.filter((b) => shown(rules, b, 'wide')).length, 'one brand above 760px').toBe(1);
  });

  it('at 760px or less, menu closed, a new notice: the toast is visible, and the closed account area is not', async () => {
    stubNetwork();
    const host = mount(await load());
    await signedIn(host);
    const bell = host.querySelector('.account-bar .notice-bell');
    expect(bell, 'the bell is in the account area').not.toBeNull();
    // NoticeBell.tsx:122-133's markup, which renders inside the bell when a notice arrives.
    bell!.insertAdjacentHTML(
      'beforeend',
      '<div class="notice-bell-toasts"><div class="notice-toast" data-testid="notice-toast">new</div></div>',
    );
    const toast = bell!.querySelector('.notice-toast')!;
    const shell = await shellCss();
    const rules = [...parseCss(await readHere('notice-bell/notice-bell.css')), ...shell];
    expect(shown(rules, toast, 'phone'), 'the toast is visible').toBe(true);
    expect(declared(rules, bell!.querySelector('.notice-bell-toasts')!, 'position', 'phone'), 'a fixed overlay').toBe('fixed');
    const bar = host.querySelector('.account-bar')!;
    const branchMenu = bar.querySelector('button[aria-haspopup="menu"]')!;
    expect(shown(shell, branchMenu, 'phone'), 'the rest of the closed account area stays hidden').toBe(false);
    if (declared(shell, bar, 'display', 'phone') !== 'none') {
      expect(['absolute', 'fixed'], 'the closed account area is out of flow').toContain(declared(shell, bar, 'position', 'phone'));
    }
  });

  it('staffSession={false}: the markup of 36f743a4, and no StaffNav', async () => {
    stubNetwork();
    const host = mount(await load(), { staffSession: false });
    await vi.waitFor(() => {
      expect(host.querySelector('footer')?.textContent).toContain('v1.0.0');
    });
    expect(host.innerHTML).toBe(CUSTOMER_MARKUP);
  });
});
