// OL-7 rows 118 + 122 + 124 (verification ~/xc-verify-138, shots 063 / 068 / 070): customer disabled buttons ("ส่ง",
// "อ่านทั้งหมดแล้ว", "บันทึก") are grey on grey at opacity 0.5, about 2.0:1. The screens already paint grey-600 on
// grey-100; the half opacity comes from the shared base rule `.xc-customer :where(button:disabled) { opacity: 0.5 }`
// in xc-customer-tokens.css (inside @layer xc-customer-base), and five screens repeat it (home, bookings, book,
// queue, branches: opacity 0.5/0.6 over a brand fill). Root fix: ONE unlayered customer rule, on the customer's own
// tokens, and no opacity on any customer :disabled rule. Customer colours stay (ruling A1).
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
  readdirSync(dir: string, opts: { recursive: true }): string[];
}

type Rule = { layer: string; selector: string; decls: Map<string, string> };

/** Flat list of style rules with the @layer they sit in ('' = unlayered); @media bodies are walked through. */
function parseCss(text: string): Rule[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const walk = (body: string, layer: string) => {
    let i = 0;
    while (i < body.length) {
      const open = body.indexOf('{', i);
      if (open < 0) return;
      const head = body.slice(i, open).replace(/^[\s;}]+/, '').trim();
      let depth = 1;
      let j = open + 1;
      while (j < body.length && depth > 0) {
        if (body[j] === '{') depth += 1;
        if (body[j] === '}') depth -= 1;
        j += 1;
      }
      const inner = body.slice(open + 1, j - 1);
      if (head.startsWith('@layer')) walk(inner, head.slice(6).trim());
      else if (head.startsWith('@')) walk(inner, layer);
      else {
        const decls = new Map<string, string>();
        for (const d of inner.split(';')) {
          const k = d.indexOf(':');
          if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
        }
        for (const sel of head.split(/,(?![^(]*\))/)) out.push({ layer, selector: normSel(sel), decls });
      }
      i = j;
    }
  };
  walk(src, '');
  return out;
}

let fs: NodeFs;
let here = '';
let shared: Rule[] = [];
let themeCss = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  shared = parseCss(fs.readFileSync(here + 'xc-customer-tokens.css', 'utf8'));
  themeCss = fs.readFileSync(here + '../../../shared/ui/xc-theme.css', 'utf8');
});

const DISABLED = '.xc-customer button:disabled';

function lastValue(rules: Rule[], selector: string, prop: string, layer = ''): string | undefined {
  for (let i = rules.length - 1; i >= 0; i -= 1) {
    const r = rules[i]!;
    if (r.selector === selector && r.layer === layer && r.decls.has(prop)) return r.decls.get(prop);
  }
  return undefined;
}

function token(name: string): string | undefined {
  for (let i = shared.length - 1; i >= 0; i -= 1) {
    const r = shared[i]!;
    if (r.selector === '.xc-customer .xc-theme' && r.layer === '' && r.decls.has(name)) return r.decls.get(name);
  }
  return undefined;
}

function hex(name: string): string {
  const v = new RegExp(`${name}\\s*:\\s*(#[0-9a-fA-F]{6})\\b`).exec(themeCss)?.[1];
  if (!v) throw new Error(`${name} not found as a 6-digit hex in xc-theme.css`);
  return v;
}

function contrast(a: string, b: string): number {
  const lum = (h: string) => {
    const c = [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
    const l = c.map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
    return 0.2126 * l[0]! + 0.7152 * l[1]! + 0.0722 * l[2]!;
  };
  const [x, y] = [lum(a), lum(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

describe('CD customer disabled buttons: one root rule on customer tokens (rows 118, 122, 124)', () => {
  it('CD-1 the customer token block names the disabled colours', () => {
    expect(token('--xc-disabled-bg')).toBe('var(--xc-gray-100)');
    expect(token('--xc-disabled-text')).toBe('var(--xc-gray-600)');
    expect(token('--xc-disabled-border')).toBe('var(--xc-border)');
  });

  it('CD-2 the pair reads at ≥ 4.5:1', () => {
    expect(contrast(hex('--xc-gray-600'), hex('--xc-gray-100'))).toBeGreaterThanOrEqual(4.5);
  });

  it('CD-3 the layered base rule no longer fades disabled buttons', () => {
    const layered = shared.filter((r) => r.layer !== '' && /:disabled/.test(r.selector));
    for (const r of layered) expect(r.decls.has('opacity'), r.selector).toBe(false);
  });

  it('CD-4 one UNLAYERED rule paints disabled buttons with the tokens (beats each screen’s fill)', () => {
    expect(lastValue(shared, DISABLED, 'background')).toBe('var(--xc-disabled-bg)');
    expect(lastValue(shared, DISABLED, 'border-color')).toBe('var(--xc-disabled-border)');
    expect(lastValue(shared, DISABLED, 'color')).toBe('var(--xc-disabled-text)');
    expect(lastValue(shared, DISABLED, 'box-shadow')).toBe('none');
    expect(lastValue(shared, DISABLED, 'cursor')).toBe('not-allowed');
    expect(lastValue(shared, DISABLED, 'opacity')).toBeUndefined();
  });

  it('CD-5 no customer stylesheet fades a disabled control with opacity', () => {
    const root = here + '../';
    const files = fs.readdirSync(root, { recursive: true }).filter((f) => f.endsWith('.css'));
    expect(files.length).toBeGreaterThan(10);
    const faded: string[] = [];
    for (const f of files) {
      for (const r of parseCss(fs.readFileSync(root + f, 'utf8'))) {
        if (/:disabled|\[disabled\]|aria-disabled/.test(r.selector) && !/:not\(:disabled\)/.test(r.selector)) {
          const o = r.decls.get('opacity');
          if (o !== undefined && o !== '1') faded.push(`${f} ${r.selector} opacity:${o}`);
        }
      }
    }
    expect(faded).toEqual([]);
  });
});
