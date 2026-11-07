// OL-6 wave B (cust-shell-rail, audit #564 issuecomment-6702970101): chip rail scroll cue; /app/verification marks no bottom tab.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { CustomerShell } from './Shell';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

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

let rules: Rule[] = [];

/** The value the stylesheet's last rule for this exact selector gives the property, inside `media` when named. */
function value(selector: string, prop: string, media = ''): string | undefined {
  const m = media.replace(/\s+/g, '');
  for (let i = rules.length - 1; i >= 0; i -= 1) {
    const r = rules[i]!;
    if (r.selector === selector && r.media === m && r.decls.has(prop)) return r.decls.get(prop);
  }
  return undefined;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  rules = parseCss(fs.readFileSync(here + 'shell.css', 'utf8'));
});

const RAIL = '.xc-customer-bar:has(.xc-customer-name)';
const PHONE = '(max-width: 760px)';
const FADE = 'linear-gradient(to right, black calc(100% - 32px), transparent)';

describe('cust-shell-rail: the chip rail shows it scrolls', () => {
  it('RL-1 signed in on a phone the rail fades at its right edge', () => {
    expect(value(RAIL, 'mask-image', PHONE)).toBe(FADE);
    expect(value(RAIL, '-webkit-mask-image', PHONE)).toBe(FADE);
  });

  it('RL-2 the rail ends past the fade, so the last chip is fully visible when scrolled to the end', () => {
    expect(value(RAIL, 'padding-right', PHONE)).toBe('40px');
  });

  it('RL-3 the rail still scrolls and hides its scrollbar', () => {
    expect(value('.xc-customer-bar', 'overflow-x')).toBe('auto');
    expect(value('.xc-customer-bar', 'scrollbar-width')).toBe('none');
  });
});

describe('cust-shell-rail: /app/verification marks no tab', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    vi.spyOn(globalThis, 'fetch').mockImplementation(
      async () => new Response(JSON.stringify({ error: { code: 'signIn', message: 'x' } }), { status: 401 }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.restoreAllMocks();
  });

  it('RL-4 the screen passes its own route to the shell', async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    const main = fs.readFileSync(here + '../verification/main.tsx', 'utf8');
    expect(main).toContain('<CustomerShell route="/app/verification">');
    const shell = fs.readFileSync(here + 'Shell.tsx', 'utf8');
    expect(shell).toContain("'/app/verification': undefined,");
  });

  it('RL-5 with that route no bottom tab is current', () => {
    dispose = render(() => <CustomerShell route="/app/verification" />, host);
    expect(host.querySelectorAll('.xc-customer-nav a').length).toBe(5);
    expect(host.querySelector('.xc-customer-nav a[aria-current="page"]')).toBeNull();
  });

  it('RL-6 home still marks the home tab', () => {
    dispose = render(() => <CustomerShell route="/app" />, host);
    expect(host.querySelector('.xc-customer-nav a[aria-current="page"]')?.getAttribute('href')).toBe('/app');
  });
});
