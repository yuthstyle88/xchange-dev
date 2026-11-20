// Dark mode for customer pages (design xchange-one-look.md §2.6; owner 2026-11-20 "ธีม ไม่สมบูรณ์", "ไม่มี dark mode").
// xc-theme-dark.css mirrors the value ramps on every .xc-theme, so customer rules that name a token follow. This file pins
// the customer side of it: the alias block, the traps (wine text, white text on a fill that turns light) and the contrast
// of every text/background pair the customer pages rely on, in both modes.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  readdirSync(dir: string, opts: { recursive: boolean }): string[];
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

let fs: NodeFs;
let path: NodePath;
let customerDir = '';
let tokens = '';
let theme = '';
let dark = '';

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
  const here = path.dirname(new URL(import.meta.url).pathname);
  customerDir = path.join(here, '..');
  const ui = path.join(here, '..', '..', '..', 'shared', 'ui');
  tokens = fs.readFileSync(path.join(here, 'xc-customer-tokens.css'), 'utf8');
  theme = fs.readFileSync(path.join(ui, 'xc-theme.css'), 'utf8');
  dark = fs.readFileSync(path.join(ui, 'xc-theme-dark.css'), 'utf8');
});

const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '');

/** The body of the first rule whose selector list starts with `head` (whitespace-insensitive). */
function body(css: string, head: string): string {
  const src = strip(css);
  const at = src.replace(/\s+/g, ' ').indexOf(head.replace(/\s+/g, ' '));
  if (at < 0) return '';
  const flat = src.replace(/\s+/g, ' ');
  const open = flat.indexOf('{', at);
  return flat.slice(open + 1, flat.indexOf('}', open));
}

const decl = (b: string, prop: string) => new RegExp(`(?:^|;)\\s*${prop}\\s*:\\s*([^;]+)`).exec(b)?.[1]?.trim();

const LIGHT_BLOCK = '.xc-customer.xc-theme, .xc-customer .xc-theme {';
const DARK_BLOCK = ':root[data-theme="dark"] .xc-customer.xc-theme, :root[data-theme="dark"] .xc-customer .xc-theme {';

/** Customer stylesheets of pages that carry the switch (the TV boards load no theme mode and stay light). */
function customerRules(): Array<[string, string]> {
  const out: Array<[string, string]> = [];
  for (const rel of fs.readdirSync(customerDir, { recursive: true })) {
    if (!rel.endsWith('.css') || rel.includes('rate-display') || rel.includes('queue-display')) continue;
    for (const m of strip(fs.readFileSync(path.join(customerDir, rel), 'utf8')).matchAll(/\{([^{}]*)\}/g)) {
      out.push([rel, m[1]!]);
    }
  }
  return out;
}

describe('customer dark aliases (xc-customer-tokens.css)', () => {
  it('CDK-1 the dark block is scoped to customer surfaces in dark mode, on screen only', () => {
    const src = strip(tokens);
    const at = src.indexOf(':root[data-theme="dark"] .xc-customer.xc-theme');
    expect(at).toBeGreaterThan(0);
    expect(src.slice(0, at)).toMatch(/@media screen \{\s*$/);
  });

  it('CDK-2 every alias that holds a literal colour in light mode is re-pointed for dark', () => {
    const light = body(tokens, LIGHT_BLOCK);
    const literal = [...light.matchAll(/(--xc-[a-z0-9-]+)\s*:\s*[^;]*#[0-9a-fA-F]{3,8}/g)].map((m) => m[1]!);
    expect(literal.length).toBeGreaterThan(0);
    const darkBody = body(tokens, DARK_BLOCK);
    for (const name of literal) expect(decl(darkBody, name), name).toMatch(/^(var\(--xc-|0 )/);
    expect(darkBody).not.toMatch(/#[0-9a-fA-F]{3,8}/);
  });

  it('CDK-3 the dark page is the near-black step, under dark grey cards', () => {
    const darkBody = body(tokens, DARK_BLOCK);
    expect(decl(darkBody, '--xc-canvas')).toBe('var(--xc-gray-75)');
    expect(decl(darkBody, '--xc-bg')).toBe('var(--xc-gray-75)');
  });
});

describe('customer stylesheets avoid the dark-mode traps', () => {
  it('CDK-4 brand-coloured text reads --xc-brand-text, never the --xc-brand / --xc-primary fill', () => {
    const bad = customerRules().filter(([, b]) => /(^|[^-\w])color:\s*var\(--(xc-brand|xc-primary)\)/.test(b));
    expect(bad.map(([f, b]) => `${f}: ${b.trim().slice(0, 60)}`)).toEqual([]);
  });

  it('CDK-5 no rule paints white text on a fill that turns light in dark mode', () => {
    const bad = customerRules().filter(([, b]) => {
      const white = /(^|[^-])color:\s*var\(--xc-(on-brand|gray-0|brand-ink)\)/.test(b);
      const lightInDark =
        /background(-color)?:\s*var\(--(xc-ink|xc-text|xc-danger|xc-warn|xc-warning|xc-success|xc-ok|xc-gray-[5-9]\d{2}|xc-gray-1000|xc-brand-950|xc-red-700|xc-green-[5-9]00|xc-amber-[5-9]00)\)/.test(
          b,
        );
      return white && lightInDark;
    });
    expect(bad.map(([f]) => f)).toEqual([]);
  });
});

describe('customer text pairs read at 4.5:1 in both modes', () => {
  const hexOf = (name: string): string => {
    const m = theme.match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`));
    if (!m) throw new Error(`no value for --${name}`);
    return m[1]!.toLowerCase();
  };
  /** A token's value: customer dark block, then the staff dark mapping, then the light chain (customer, then theme). */
  const resolve = (token: string, mode: 'light' | 'dark'): string => {
    if (mode === 'dark') {
      const c = decl(body(tokens, DARK_BLOCK), `--${token}`);
      if (c) return chase(c, mode);
      const d = dark.match(new RegExp(`--${token}:\\s*var\\(--(xc-dk-[a-z0-9-]+)\\)`));
      if (d) return hexOf(d[1]!);
    }
    const own = decl(body(tokens, LIGHT_BLOCK), `--${token}`);
    if (own) return chase(own, mode);
    const v = theme.match(new RegExp(`^\\s*--${token}:\\s*([^;]+);`, 'm'));
    if (!v) throw new Error(`no --${token}`);
    return chase(v[1]!.trim(), mode);
  };
  const chase = (val: string, mode: 'light' | 'dark'): string => {
    const ref = val.match(/^var\(--([a-z0-9-]+)\)/);
    return ref ? resolve(ref[1]!, mode) : val.toLowerCase();
  };
  const lum = (h: string): number => {
    const c = [1, 3, 5]
      .map((i) => parseInt(h.slice(i, i + 2), 16) / 255)
      .map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
    return 0.2126 * c[0]! + 0.7152 * c[1]! + 0.0722 * c[2]!;
  };
  const ratio = (a: string, b: string): number => {
    const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
    return (x! + 0.05) / (y! + 0.05);
  };

  const PAIRS: Array<[string, string, string]> = [
    ['text on a card', 'xc-text', 'xc-card'],
    ['text on the page', 'xc-text', 'xc-canvas'],
    ['text on the header', 'xc-text', 'xc-header'],
    ['text in a field', 'xc-text', 'xc-field'],
    ['muted text on a card', 'xc-text-muted', 'xc-card'],
    ['muted text on the header', 'xc-text-muted', 'xc-header'],
    ['muted text on the page', 'xc-text-muted', 'xc-canvas'],
    ['brand text on a card', 'xc-brand-text', 'xc-card'],
    ['brand text on the brand tint', 'xc-brand-text', 'xc-brand-soft'],
    ['white on the primary fill', 'xc-on-brand', 'xc-primary'],
    ['white on the primary hover', 'xc-on-brand', 'xc-primary-hover'],
    ['on-danger text on the danger fill', 'xc-on-danger', 'xc-danger'],
    ['danger text on the danger tint', 'xc-danger-text', 'xc-danger-bg'],
    ['success text on the success tint', 'xc-success-text', 'xc-success-bg'],
    ['warning text on the warning tint', 'xc-warning-text', 'xc-warning-bg'],
    ['disabled text on the disabled fill', 'xc-disabled-text', 'xc-disabled-bg'],
  ];

  for (const mode of ['light', 'dark'] as const) {
    it.each(PAIRS)(`${mode}: %s reads at 4.5:1 or better`, (_label, fg, bg) => {
      expect(ratio(resolve(fg, mode), resolve(bg, mode))).toBeGreaterThanOrEqual(4.5);
    });
  }
});
