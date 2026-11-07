// Pins the staff sign-in subtitle (AuthLayout's `subtitle` prop, LoginScreen's th and en text, its auth.css look) and the
// version line 20px under the card (`.xc-auth-card`'s margin above 560px).
// OL-6 L1 (#6532), docs/design/xchange-one-look-layout.md §1 D4
// Written before the implementation by claude/f6a909ac for sub-controller claude/1cc51615. Do not edit, delete or weaken.
//
// One test per row of #6532 §9. `subtitle` goes through a loose props cast, so tsc passes before the prop exists and
// the tests go red on a wrong value. auth.css is read as text (vitest returns CSS imports empty), and every rule that
// matches a selector is checked, not just the first, so a later rule cannot undo the value.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { render } from 'solid-js/web';
import type { AuthApi } from '../../screens/login/auth-api';
import { LoginScreen } from '../../screens/login/LoginScreen';
import { setLang } from '../i18n';
import { AuthLayout } from './AuthLayout';

const LooseAuthLayout = AuthLayout as unknown as (props: {
  title: string;
  subtitle?: string;
  children?: JSX.Element;
}) => JSX.Element;

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let authCss = '';
let dispose: (() => void) | undefined;

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const dir = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  authCss = fs.readFileSync(dir + 'auth.css', 'utf8');
});

beforeEach(() => {
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => new Response('{}', { status: 503, headers: { 'Content-Type': 'application/json' } })),
  );
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

async function mountLayout(subtitle?: string) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <LooseAuthLayout title="the title" subtitle={subtitle}>
        <form class="the-children" />
      </LooseAuthLayout>
    ),
    host,
  );
  await settle();
  return host;
}

async function mountLogin() {
  const auth = {
    session: async () => ({ user: null }),
    login: async () => {
      throw new Error('login was not expected in this test');
    },
  } as unknown as AuthApi;
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <LoginScreen auth={auth} navigate={() => {}} />, host);
  await settle();
  return host;
}

function subtitleText(host: HTMLElement): (string | undefined)[] {
  return Array.from(host.querySelectorAll('p.xc-auth-subtitle')).map((p) => p.textContent?.trim());
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

/** Flat rules of a stylesheet: one entry per selector of each rule, inside at most one @media. */
function parseCss(text: string): Rule[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const block = (body: string, media: string) => {
    const re = /([^{}]+)\{([^{}]*)\}/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(body)) !== null) {
      const decls = new Map<string, string>();
      for (const d of m[2]!.split(';')) {
        const k = d.indexOf(':');
        if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
      }
      for (const sel of m[1]!.split(',')) out.push({ media, selector: sel.trim().replace(/\s+/g, ' '), decls });
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
    block(src.slice(open + 1, j - 1), src.slice(at + 6, open).replace(/\s+/g, ''));
    i = j;
  }
  block(rest, '');
  return out;
}

/** The rules whose selector targets `className` itself, not something inside it. */
function rulesFor(className: string): Rule[] {
  return parseCss(authCss).filter((r) => r.selector.endsWith(`.${className}`));
}

/** Every value of `prop` across `rules`, plus the name of any margin longhand that could undo a `margin`. */
function declared(rules: Rule[], prop: string): string[] {
  const out: string[] = [];
  for (const r of rules) {
    for (const [k, v] of r.decls) {
      if (k === prop) out.push(v);
      else if (prop === 'margin' && k.startsWith('margin-')) out.push(`${k}: ${v}`);
    }
  }
  return out;
}

describe('AuthLayout — the subtitle (#6532 §9)', () => {
  it('with subtitle="x", header.xc-auth-header holds the h1 and then p.xc-auth-subtitle with text x', async () => {
    const host = await mountLayout('x');
    const header = host.querySelector('header.xc-auth-header');
    expect(header, 'the card header').not.toBeNull();
    expect(subtitleText(header as HTMLElement)).toEqual(['x']);
    const h1 = header!.querySelector('h1');
    expect(h1?.nextElementSibling?.matches('p.xc-auth-subtitle'), 'the subtitle comes right after the h1').toBe(true);
  });

  it('without subtitle, there is no .xc-auth-subtitle and the rest is unchanged', async () => {
    const host = await mountLayout();
    expect(host.querySelector('.xc-auth-subtitle')).toBeNull();
    const header = host.querySelector('header.xc-auth-header')!;
    expect(Array.from(header.children).map((e) => e.className || e.tagName)).toEqual(['xc-auth-header-top', 'H1']);
    const card = host.querySelector('main.xc-auth-card')!;
    expect(Array.from(card.children).map((e) => e.tagName)).toEqual(['HEADER', 'FORM']);
  });
});

describe('LoginScreen — the subtitle text (#6532 §9)', () => {
  it('/login in th reads ใช้อีเมล เบอร์โทรศัพท์ หรือชื่อผู้ใช้', async () => {
    setLang('th');
    const host = await mountLogin();
    expect(subtitleText(host)).toEqual(['ใช้อีเมล เบอร์โทรศัพท์ หรือชื่อผู้ใช้']);
  });

  it('/login in en reads Use your email, phone number or username', async () => {
    setLang('en');
    const host = await mountLogin();
    expect(subtitleText(host)).toEqual(['Use your email, phone number or username']);
  });
});

describe('auth.css (#6532 §9)', () => {
  it('.xc-auth-card above 560px has margin: 8vh auto 20px in every rule that sets it', () => {
    const above560 = rulesFor('xc-auth-card').filter((r) => !r.media.includes('max-width'));
    expect(declared(above560, 'margin')).toEqual(['8vh auto 20px']);
  });

  it('.xc-auth-subtitle has color: var(--xc-muted) and margin: 0 0 24px in every rule that sets them', () => {
    const rules = rulesFor('xc-auth-subtitle');
    expect(declared(rules, 'color')).toEqual(['var(--xc-muted)']);
    expect(declared(rules, 'margin')).toEqual(['0 0 24px']);
  });
});
