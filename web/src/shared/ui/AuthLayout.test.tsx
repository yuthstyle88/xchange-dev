// V1-auth-layout (#531; design docs/design/xchange-v3-web.md §12.21): one frame for signed-out pages, so the login
// page looks like v2's (xchange-v2-source@4d0bdbb:src/devices-public/login.html:1) and the next signed-out page reuses it.
// Written before the implementation by the lane controller (claude/608dff). Do not edit, delete or weaken.
//
// What this test pins, in `web/src/shared/ui/AuthLayout.tsx`:
//
//   export function AuthLayout(props: { title: string; help?: JSX.Element; children?: JSX.Element }): JSX.Element;
//
// - One `<main class="xc-auth-card">` (the card). Inside it a `<header>` holding the brand line `XCHANGE × T.K.`,
//   then the `<h1>` title, and `LangSwitch`; then the help line when given; then the children.
// - `VersionFooter`'s `<footer>` after the card, outside it, from GET /health/ready.
// - No AccountBar, no session notice, no `fieldset.xc-lock`, and no read of the `me` view: nobody is signed in.
// - `web/src/shared/ui/auth.css` (read from disk: vitest returns CSS imports empty) is v2's card under `.xc-auth`,
//   and every rule in it is scoped under `.xc-auth` so no other screen is restyled.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { lang, setLang } from '../i18n';

type AuthLayoutFn = (props: { title: string; help?: JSX.Element; children?: JSX.Element }) => JSX.Element;

const modules = import.meta.glob<{ AuthLayout?: AuthLayoutFn }>('./AuthLayout.tsx', { eager: true });

function AuthLayoutOrFail(): AuthLayoutFn {
  const mod = modules['./AuthLayout.tsx'];
  expect(mod?.AuthLayout, 'web/src/shared/ui/AuthLayout.tsx exports AuthLayout (§12.21)').toBeTypeOf('function');
  return mod!.AuthLayout!;
}

const READY = {
  status: 'ok',
  database: true,
  version: '1.0.0',
  build: '0123456789abcdef0123456789abcdef01234567',
  builtAt: '2026-10-08T01:02:03Z',
  channel: 'staging',
  ui: 'built',
};

const ME = { user: { id: 'owner', username: 'owner1', name: 'Somchai', role: 'OWNER', branch: 'XC-B01' } };

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
  existsSync(file: string): boolean;
}

let authCss: string | undefined;
let dispose: (() => void) | undefined;
let style: HTMLStyleElement | undefined;
let fetched: string[];

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const dir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  authCss = fs.existsSync(dir + 'auth.css') ? fs.readFileSync(dir + 'auth.css', 'utf8') : undefined;
});

beforeEach(() => {
  fetched = [];
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string) => {
      fetched.push(String(url));
      if (/\/health\/ready$/.test(String(url))) return new Response(JSON.stringify(READY), { status: 200 });
      // Answers a signed-in user to anything else, so an AccountBar would show if the layout had one.
      return new Response(JSON.stringify({ changed: true, version: 1, data: ME }), { status: 200 });
    }),
  );
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  style?.remove();
  style = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

async function mount(help?: string) {
  const AuthLayout = AuthLayoutOrFail();
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <AuthLayout title="เข้าสู่ระบบพนักงาน" help={help === undefined ? undefined : <span class="the-help">{help}</span>}>
        <form class="the-children">
          <input name="x" />
        </form>
      </AuthLayout>
    ),
    host,
  );
  await settle();
  return host;
}

function cardOf(host: HTMLElement): HTMLElement {
  const mains = host.querySelectorAll('main');
  expect(mains, 'one <main> landmark: the card').toHaveLength(1);
  const card = mains[0] as HTMLElement;
  expect(card.classList.contains('xc-auth-card'), 'the card is main.xc-auth-card').toBe(true);
  return card;
}

function follows(a: Node, b: Node): boolean {
  return (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
}

function applyAuthCss(): CSSStyleRule[] {
  expect(authCss, 'web/src/shared/ui/auth.css exists (§12.21.1)').toBeTypeOf('string');
  style = document.createElement('style');
  style.textContent = authCss!;
  document.head.appendChild(style);
  const rules: CSSStyleRule[] = [];
  const walk = (list: CSSRuleList) => {
    for (const rule of Array.from(list)) {
      if (rule.type === CSSRule.STYLE_RULE) rules.push(rule as CSSStyleRule);
      else if ('cssRules' in rule) walk((rule as CSSMediaRule).cssRules);
    }
  };
  walk(style.sheet!.cssRules);
  return rules;
}

describe('AuthLayout — the card (§12.21.1)', () => {
  it('the card is the one <main>; its <header> holds the brand line, then the <h1> title', async () => {
    const host = await mount();
    const card = cardOf(host);
    const header = card.querySelector('header');
    expect(header, 'a <header> inside the card').not.toBeNull();
    const h1 = header!.querySelector('h1');
    expect(h1?.textContent?.trim()).toBe('เข้าสู่ระบบพนักงาน');
    expect(host.querySelectorAll('h1')).toHaveLength(1);
    const brand = Array.from(header!.querySelectorAll('*')).find((e) => e.textContent?.trim() === 'XCHANGE × T.K.');
    expect(brand, 'the brand line XCHANGE × T.K. inside the card header').toBeDefined();
    expect(follows(brand!, h1!), 'the brand line comes before the title, as in v2').toBe(true);
  });

  it('every <header> on the page is inside the card: nothing sits above it', async () => {
    const host = await mount();
    const card = cardOf(host);
    for (const header of Array.from(host.querySelectorAll('header'))) expect(card.contains(header)).toBe(true);
  });

  it('LangSwitch is inside the card header and switches the language', async () => {
    const host = await mount();
    const header = cardOf(host).querySelector('header')!;
    const button = Array.from(header.querySelectorAll('button')).find((b) => b.textContent?.trim() === 'EN');
    expect(button, 'the EN switch inside the card header').toBeDefined();
    button!.click();
    await settle();
    expect(lang()).toBe('en');
    expect(button!.textContent?.trim()).toBe('TH');
  });

  it('the help line, when given, is inside the card after the title and before the children', async () => {
    const host = await mount('ตั้งรหัสผ่านอย่างน้อย 12 ตัวอักษร');
    const card = cardOf(host);
    const help = card.querySelector('.the-help');
    expect(help?.textContent).toBe('ตั้งรหัสผ่านอย่างน้อย 12 ตัวอักษร');
    expect(follows(card.querySelector('h1')!, help!)).toBe(true);
    expect(follows(help!, card.querySelector('.the-children')!)).toBe(true);
  });

  it('the children are inside the card, after its header', async () => {
    const host = await mount();
    const card = cardOf(host);
    const children = card.querySelector('form.the-children');
    expect(children, 'the children inside the card').not.toBeNull();
    expect(follows(card.querySelector('header')!, children!)).toBe(true);
    expect(card.textContent ?? '').not.toContain('undefined');
  });
});

describe('AuthLayout — the version footer (§12.21.1)', () => {
  it('shows the build line from /health/ready in a <footer> after the card, outside it', async () => {
    const host = await mount();
    const card = cardOf(host);
    const footers = host.querySelectorAll('footer');
    expect(footers).toHaveLength(1);
    const footer = footers[0]!;
    expect(card.contains(footer), 'the footer is outside the card').toBe(false);
    expect(follows(card, footer), 'the footer comes after the card').toBe(true);
    expect(footer.textContent).toContain('v1.0.0 · 0123456 · staging');
    expect(footer.textContent).toContain('2026-10-08T01:02:03Z');
    expect(fetched.some((u) => /\/health\/ready$/.test(u))).toBe(true);
  });
});

describe('AuthLayout — signed-out only (§12.21.1)', () => {
  it('has no AccountBar, no session notice, no lock, and never reads the me view', async () => {
    const host = await mount();
    expect(host.querySelector('.account-bar')).toBeNull();
    expect(host.querySelector('fieldset')).toBeNull();
    expect(host.textContent).not.toContain('ออกจากระบบ');
    expect(host.textContent).not.toContain('Somchai');
    for (const url of fetched) expect(url).toMatch(/\/health\/ready$/);
  });
});

describe('AuthLayout — auth.css (§12.21.1, v2 login.html:1)', () => {
  it("every rule is scoped under .xc-auth (or is body:has(.xc-auth)), so other screens are not restyled", () => {
    const rules = applyAuthCss();
    expect(rules.length).toBeGreaterThan(0);
    for (const rule of rules) {
      for (const part of rule.selectorText.split(',')) {
        expect(part, `selector "${part.trim()}" is scoped under .xc-auth`).toContain('.xc-auth');
      }
    }
  });

  it("the card is v2's card: 420px wide at most, 32px padding, 16px corners, centred", async () => {
    applyAuthCss();
    const host = await mount();
    const card = cardOf(host);
    expect(card.closest('.xc-auth'), 'the card sits inside the layout root .xc-auth').not.toBeNull();
    const s = getComputedStyle(card);
    expect(s.maxWidth).toBe('420px');
    expect(s.paddingTop).toBe('32px');
    expect([s.borderRadius, s.borderTopLeftRadius]).toContain('16px');
    expect(s.marginLeft).toBe('auto');
    expect(s.marginRight).toBe('auto');
  });
});
