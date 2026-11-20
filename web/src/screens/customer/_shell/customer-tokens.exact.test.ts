import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { selector: string; decls: Map<string, string> };

function parseCss(text: string): { rules: Rule[]; line1: string } {
  const line1 = text.split('\n')[0]?.trim() ?? '';
  const raw = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(raw)) !== null) {
    const decls = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const k = d.indexOf(':');
      if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
    }
    for (const sel of m[1]!.split(',')) out.push({ selector: normSel(sel), decls });
  }
  return { rules: out, line1 };
}

function value(rules: Rule[], selector: string, prop: string): string | undefined {
  const sel = selector.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  for (const r of rules) if (r.selector === sel && r.decls.has(prop)) return r.decls.get(prop);
  return undefined;
}

describe('xc-customer-tokens.css exact values (A1-b #6518)', () => {
  let rules: Rule[] = [];
  let line1 = '';

  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    const cssText = fs.readFileSync(here + 'xc-customer-tokens.css', 'utf8');
    const parsed = parseCss(cssText);
    rules = parsed.rules;
    line1 = parsed.line1;
  });

  it('line 1 has legacy marker', () => {
    expect(line1).toBe('/* xc-look: legacy-colours */');
  });

  const selectors = ['.xc-customer.xc-theme', '.xc-customer .xc-theme'];

  it.each(selectors)('%s pins exact card radius and shadow', (sel) => {
    expect(value(rules, sel, '--xc-radius-card')).toBe('22px');
    expect(value(rules, sel, '--xc-shadow-card')).toBe('0 12px 32px #54001e1f');
  });

  it.each(selectors)('%s pins exact header and muted text colours', (sel) => {
    expect(value(rules, sel, '--xc-header')).toBe('#fffdf8');
    // Restated 2026-11-20 (design §2.6, dark mode): #7a7178 read 4.34:1 on the customer page (gray-50); the warm grey
    // gray-600 (#6c6268) is the same role at 5.3:1 and follows dark mode.
    expect(value(rules, sel, '--xc-text-muted')).toBe('var(--xc-gray-600)');
  });

  it.each(selectors)('%s pins info background and teal steps', (sel) => {
    expect(value(rules, sel, '--xc-info-bg')).toBe('var(--xc-teal-50)');
    expect(value(rules, sel, '--xc-teal-50')).toBe('#eef4f6');
    expect(value(rules, sel, '--xc-teal-200')).toBe('#c6d6dd');
  });
});
