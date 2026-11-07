// Pins the one white staff header: nothing before it, the account area inside it, the title first in the content root,
// the customer markup unchanged, and the admin, receipt-sample and staff-guide rules that reach it.
// OL-6 L4 (#6537), docs/design/xchange-one-look-layout.md §1 D1, §2 Q1, §3 L4 (Q1 confirmed on #564 issuecomment-6688832431)
// Written before the implementation by claude/f6a909ac for sub-controller claude/1cc51615. Do not edit, delete or weaken.
//
// Rows 1, 2, 4, 6, 7, 8 and 9 of #6537 §9. Row 3 is in AppShell.phone.test.tsx, row 5 in AppShell.lock.test.tsx, and
// row 10 in AppShell.branch-*.test.ts. The brand's selector is `.xc-shell-brand`, the class shell.css already styles;
// its text is not pinned.
//
// CUSTOMER_* are `host.innerHTML` captured on the base, 1e451f2c. The CSS is read as text (vitest returns CSS imports
// empty), and every matching rule is checked, not just the first. The shared session is a page singleton, so every test
// loads fresh modules, and nothing mounts as JSX (as AppShell.lock.test.tsx does).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Me } from '../api/generated/common';
import { meOf } from '../../test-support/me';
import { makeBranchRef } from '../testing/fixtures';
import { warmModules } from '../testing/warm-modules';

const CUSTOMER_NAV =
  '<div class="xc-shell xc-theme"><header class="xc-shell-header" data-menu-open="false"><div id="xc-shell-menu" class="xc-shell-panel" data-open="false"></div><div class="xc-shell-nav"><nav><a href="#one" id="nav-one">หนึ่ง</a></nav></div><h1 class="xc-shell-title">คีย์ย้อนหลัง</h1><div role="group" class="xc-lang-switch"><button type="button" aria-pressed="true" aria-label="เปลี่ยนภาษาเป็น ไทย">ไทย</button><button type="button" aria-pressed="false" aria-label="เปลี่ยนภาษาเป็น English">EN</button></div></header><fieldset class="xc-lock" style="border:0;padding:0;margin:0;min-width:0"><main class="xc-shell-main"><section id="the-content"></section></main></fieldset><div class="xc-shell-footer"><footer><div><span>v1.0.0<!----> · 0123456<!----> · staging</span> <span>2026-10-08T01:02:03Z</span></div></footer></div></div>';
const CUSTOMER_OWNS_MAIN =
  '<div class="xc-shell xc-theme"><header class="xc-shell-header" data-menu-open="false"><div id="xc-shell-menu" class="xc-shell-panel" data-open="false"></div><h1 class="xc-shell-title">คีย์ย้อนหลัง</h1><div role="group" class="xc-lang-switch"><button type="button" aria-pressed="true" aria-label="เปลี่ยนภาษาเป็น ไทย">ไทย</button><button type="button" aria-pressed="false" aria-label="เปลี่ยนภาษาเป็น English">EN</button></div></header><fieldset class="xc-lock" style="border:0;padding:0;margin:0;min-width:0"><div class="xc-content"><section id="the-content"></section></div></fieldset><div class="xc-shell-footer"><footer><div><span>v1.0.0<!----> · 0123456<!----> · staging</span> <span>2026-10-08T01:02:03Z</span></div></footer></div></div>';
const CUSTOMER_AUDIENCE =
  '<div class="xc-shell xc-theme"><header class="xc-shell-header" data-menu-open="false"><div id="xc-shell-menu" class="xc-shell-panel" data-open="false"></div><div class="xc-shell-nav"><nav><a href="#one" id="nav-one">หนึ่ง</a></nav></div><h1 class="xc-shell-title">คีย์ย้อนหลัง</h1><button type="button" aria-pressed="true" aria-label="เปลี่ยนภาษาเป็น ไทย">TH</button><button type="button" aria-pressed="false" aria-label="เปลี่ยนภาษาเป็น English">EN</button><button type="button" aria-pressed="false" aria-label="เปลี่ยนภาษาเป็น 中文">中文</button></header><fieldset class="xc-lock" style="border:0;padding:0;margin:0;min-width:0"><main class="xc-shell-main"><section id="the-content"></section></main></fieldset><div class="xc-shell-footer"><footer><div><span>v1.0.0<!----> · 0123456<!----> · staging</span> <span>2026-10-08T01:02:03Z</span></div></footer></div></div>';

const READY = {
  status: 'ok',
  database: true,
  version: '1.0.0',
  build: '0123456789abcdef0123456789abcdef01234567',
  builtAt: '2026-10-08T01:02:03Z',
  channel: 'staging',
  ui: 'built',
};
const TWO_BRANCHES: Me = meOf({
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
const ONE_BRANCH: Me = meOf({
  id: 'owner',
  role: 'OWNER',
  branch: 'MAIN',
  name: 'Somchai',
  branchName: 'สำนักงานใหญ่',
  branches: [],
});

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function stubNetwork(me: Me = TWO_BRANCHES) {
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
  const i18n = await import('../i18n');
  const { th } = await import('../i18n/th');
  return { web, solid, AppShell, session, i18n, th };
}

type Loaded = Awaited<ReturnType<typeof load>>;

function mount(m: Loaded, props: { staffSession?: boolean; ownsMain?: boolean; nav?: boolean } = {}): HTMLElement {
  const { nav: withNav = true, ...shell } = props;
  const host = document.createElement('div');
  document.body.appendChild(host);
  const nav = (): HTMLElement => {
    const n = document.createElement('nav');
    n.innerHTML = '<a href="#one" id="nav-one">หนึ่ง</a>';
    return n;
  };
  const content = (): HTMLElement => {
    const s = document.createElement('section');
    s.id = 'the-content';
    return s;
  };
  const dispose = m.web.render(
    () =>
      m.solid.createComponent(m.AppShell, {
        title: 'คีย์ย้อนหลัง',
        ...shell,
        ...(withNav
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

function follows(a: Node, b: Node): boolean {
  return (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
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

async function sheet(relative: string): Promise<Rule[]> {
  const fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
  const path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
  const here = path.dirname(new URL(import.meta.url).pathname);
  return parseCss(fs.readFileSync(path.join(here, relative), 'utf8'));
}

/** Every value of `prop` in the rules that pass `which`. */
function values(rules: Rule[], which: (r: Rule) => boolean, prop: string): string[] {
  return rules.filter((r) => which(r) && r.decls.has(prop)).map((r) => r.decls.get(prop)!);
}

// ── setup ────────────────────────────────────────────────────────────────────

beforeAll(
  () =>
    warmModules(
      () => import('solid-js/web'),
      () => import('solid-js'),
      () => import('./AppShell'),
      () => import('../session/session'),
      () => import('../i18n'),
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
});

// ── §9 ───────────────────────────────────────────────────────────────────────

describe('the one staff header (OL-6 L4, #6537 §9)', () => {
  it('row 1: nothing renders before header.xc-shell-header in .xc-shell, and .account-bar is inside the header', async () => {
    stubNetwork();
    const host = mount(await load());
    await signedIn(host);
    const shellRoot = host.querySelector('.xc-shell')!;
    const header = host.querySelector('header.xc-shell-header')!;
    expect(shellRoot.firstElementChild?.tagName, 'the first thing in the shell').toBe('HEADER');
    expect(header.contains(host.querySelector('.account-bar')), 'the account area is in the header').toBe(true);
    expect(host.querySelectorAll('.account-bar'), 'one account area').toHaveLength(1);
  });

  it.each([
    ['a branch switch', TWO_BRANCHES, 'button[aria-haspopup="menu"]'],
    ['a single-branch span', ONE_BRANCH, ':scope > span'],
  ])(
    'row 2: the header holds, in DOM order, the brand, the nav, the language switch, then .account-bar with the user, %s, the bell and sign-out',
    async (_name, me, branchSelector) => {
      stubNetwork(me);
      const m = await load();
      const host = mount(m);
      await signedIn(host);
      const header = host.querySelector('header.xc-shell-header')!;
      const bar = host.querySelector('.account-bar')!;
      expect(header.contains(bar), 'the account area is in the header').toBe(true);

      const brand = header.querySelector('.xc-shell-brand');
      const nav = header.querySelector('#nav-one');
      const lang = header.querySelector('.xc-lang-switch');
      expect(brand, 'the brand').not.toBeNull();
      expect(nav, 'the nav').not.toBeNull();
      expect(lang, 'the language switch').not.toBeNull();
      expect(follows(brand!, nav!), 'the brand, then the nav').toBe(true);
      expect(follows(nav!, lang!), 'the nav, then the language switch').toBe(true);
      expect(follows(lang!, bar), 'the language switch, then the account area').toBe(true);

      const label = m.th['shell.account.branch'].replace('{branch}', 'สำนักงานใหญ่');
      const user = Array.from(bar.querySelectorAll('span')).find((e) => e.textContent?.includes('Somchai'));
      const branch = Array.from(bar.querySelectorAll(branchSelector)).find((e) => e.textContent === label);
      const bell = bar.querySelector('.notice-bell');
      const signOut = Array.from(bar.querySelectorAll('button')).find(
        (b) => b.textContent?.trim() === m.th['shell.account.signOut'],
      );
      expect(user, 'the user').toBeDefined();
      expect(branch, 'the branch').toBeDefined();
      expect(bell, 'the bell').not.toBeNull();
      expect(signOut, 'the sign-out').toBeDefined();
      expect(follows(user!, branch!), 'the user, then the branch').toBe(true);
      expect(follows(branch!, bell!), 'the branch, then the bell').toBe(true);
      expect(follows(bell!, signOut!), 'the bell, then the sign-out').toBe(true);
    },
  );

  it("row 2: the white header replaces the dark bar's look (§3 item 4)", async () => {
    const rules = await sheet('shell.css');
    const bar = (r: Rule) => r.selector.startsWith('.account-bar') && !r.selector.includes(' button');
    for (const v of values(rules, bar, 'background')) expect(v, '.account-bar background').not.toContain('--xc-gray-900');
    expect(values(rules, bar, 'color'), '.account-bar has no on-brand text on the white header').not.toContain(
      'var(--xc-on-brand)',
    );
    expect(values(rules, bar, 'border-bottom'), 'the header has the one bottom line').toEqual([]);
    const header = (r: Rule) => r.selector === '.xc-shell-header';
    expect(values(rules, header, 'background')).toEqual(['var(--xc-surface)']);
    expect(values(rules, header, 'border-bottom')).toEqual(['1px solid var(--xc-line)']);
  });

  it.each([
    ['main.xc-shell-main', false],
    ['.xc-content', true],
  ])('row 4: h1.xc-shell-title is the first child of %s and is visible', async (root, ownsMain) => {
    stubNetwork();
    const host = mount(await load(), { ownsMain });
    await signedIn(host);
    const titles = host.querySelectorAll('h1.xc-shell-title');
    expect(titles, 'one title').toHaveLength(1);
    const h1 = titles[0]!;
    expect(h1.textContent).toBe('คีย์ย้อนหลัง');
    expect(h1.closest('header'), 'the title is not in the header').toBeNull();
    expect(host.querySelector(root)!.firstElementChild?.matches('h1.xc-shell-title'), 'first in the content root').toBe(
      true,
    );
    const applied = (await sheet('shell.css')).filter((r) => !r.selector.includes('::') && h1.matches(r.selector));
    for (const prop of ['display', 'visibility', 'clip']) {
      for (const v of values(applied, () => true, prop)) expect(v, `.xc-shell-title { ${prop} }`).not.toMatch(/none|hidden|rect/);
    }
  });

  it.each([
    ['with a nav', CUSTOMER_NAV, { nav: true }, 'staff'],
    ['with ownsMain and no nav', CUSTOMER_OWNS_MAIN, { nav: false, ownsMain: true }, 'staff'],
    ['with the customer audience', CUSTOMER_AUDIENCE, { nav: true }, 'customer'],
  ] as const)('row 6: staffSession={false} %s renders the markup of 1e451f2c', async (_name, want, props, audience) => {
    stubNetwork();
    const m = await load();
    m.i18n.setAudience(audience);
    const host = mount(m, { staffSession: false, ...props });
    await vi.waitFor(() => {
      expect(host.querySelector('footer')?.textContent).toContain('v1.0.0');
    });
    expect(host.innerHTML).toBe(want);
  });
});

describe('the rules that reach the header (OL-6 L4, #6537 §9)', () => {
  it('row 7: admin above the breakpoint: the header has margin-left 248px once, the account area none, the title 248px', async () => {
    const rules = await sheet('../../screens/admin/admin.css');
    const above = (r: Rule) => !r.media.includes('max-width');
    expect(
      values(rules, (r) => above(r) && r.selector === '.xc-admin .xc-shell-header', 'margin-left'),
      'the header',
    ).toEqual(['248px']);
    expect(
      values(rules, (r) => r.selector.startsWith('.xc-admin') && r.selector.endsWith('.account-bar'), 'margin-left').filter(
        (v) => v !== '0',
      ),
      'the account area',
    ).toEqual([]);
    const title = (r: Rule) => r.selector.startsWith('.xc-admin') && r.selector.endsWith('.xc-shell-title');
    expect(values(rules, (r) => above(r) && title(r), 'margin-left'), 'the title').toEqual(['248px']);
    // The offset ends where the sidebar stops being fixed: wherever the header's goes to 0, the title's does too.
    const resets = rules.filter(
      (r) => r.media !== '' && r.selector === '.xc-admin .xc-shell-header' && r.decls.get('margin-left') === '0',
    );
    expect(resets.length, 'the header has a reset').toBeGreaterThan(0);
    for (const reset of resets) {
      expect(values(rules, (r) => r.media === reset.media && title(r), 'margin-left'), `the title in ${reset.media}`).toEqual(['0']);
    }
  });

  it('row 8: receipt-sample hides .xc-shell-title on screen and in print', async () => {
    const rules = await sheet('../../screens/receipt-sample/print.css');
    const title = (r: Rule) => r.selector.endsWith('.xc-shell-title');
    expect(values(rules, (r) => r.media === '' && title(r), 'display'), 'on screen').toEqual(['none']);
    expect(values(rules, (r) => r.media === 'print' && title(r), 'display'), 'in print').toEqual(['none']);
  });

  // Restated after #6596 (OL-6 wave B, staff guide uses the shared shell): the guide no longer overrides the shell
  // title at all (pinned by staff-guide.look.test.tsx SG-1), so v2's 2em title is dropped with the other overrides.
  it('row 9: the staff guide has no .xc-shell-header .account-bar rule and does not override the shell title', async () => {
    const rules = await sheet('../../screens/staff-guide/guide.css');
    expect(rules.map((r) => r.selector).filter((s) => s.includes('.xc-shell-header .account-bar'))).toEqual([]);
    expect(values(rules, (r) => r.selector === '.xc-shell .xc-shell-title', 'font-size')).toEqual([]);
    expect(rules.map((r) => r.selector).filter((s) => s.includes('.xc-shell-header .xc-shell-title')), 'the old rule moved').toEqual([]);
  });
});
