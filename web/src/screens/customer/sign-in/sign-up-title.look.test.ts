// OL-7 row 129 (verification ~/xc-verify-138 row 129, shot 076): on /app/sign-up the title "สมัครสมาชิก" sits 12 px
// over the first label, the same gap the form uses between fields, so the title reads as one more field label.
// Sign-in is fine: its title is followed by the mode tabs (margin-bottom 1rem). `.xc-sign-in > h2 { margin: 0 0 12px }`
// is pinned by sign-in.look.test.ts, so the extra room goes on the form that follows the title directly (sign-up).
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { selector: string; decls: Map<string, string> };

function parseCss(text: string): Rule[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/@media[^{]*\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, '');
  const out: Rule[] = [];
  const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    const decls = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const k = d.indexOf(':');
      if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
    }
    for (const sel of m[1]!.split(',')) out.push({ selector: normSel(sel), decls });
  }
  return out;
}

let rules: Rule[] = [];

function value(selector: string, prop: string): string | undefined {
  for (let i = rules.length - 1; i >= 0; i -= 1) {
    const r = rules[i]!;
    if (r.selector === selector && r.decls.has(prop)) return r.decls.get(prop);
  }
  return undefined;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  rules = parseCss(fs.readFileSync(here + 'sign-in.css', 'utf8'));
});

describe('ST sign-up title spacing (row 129)', () => {
  it('ST-1 a form that follows the title directly sits 20px under it (block margins collapse: max(12px, 20px))', () => {
    expect(value('.xc-sign-in>h2+.xc-sign-in-form', 'margin-top')).toBe('20px');
  });

  it('ST-2 the shared title rule and the form gap stay as they are', () => {
    expect(value('.xc-sign-in>h2', 'margin')).toBe('0 0 12px');
    expect(value('.xc-sign-in-form', 'gap')).toBe('0.75rem');
  });
});
