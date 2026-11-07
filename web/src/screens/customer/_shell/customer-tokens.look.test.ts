import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

/** Flat rules of a stylesheet: one entry per selector of each rule, inside at most one @media or @layer. */
function parseCss(text: string): { rules: Rule[]; raw: string } {
  const raw = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
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
  while (i < raw.length) {
    const atMedia = raw.indexOf('@media', i);
    const atLayer = raw.indexOf('@layer', i);
    let at = -1;
    if (atMedia >= 0 && atLayer >= 0) at = Math.min(atMedia, atLayer);
    else if (atMedia >= 0) at = atMedia;
    else if (atLayer >= 0) at = atLayer;

    if (at < 0) {
      rest += raw.slice(i);
      break;
    }
    rest += raw.slice(i, at);
    const open = raw.indexOf('{', at);
    if (open < 0) {
      rest += raw.slice(at);
      break;
    }
    let depth = 1;
    let j = open + 1;
    while (j < raw.length && depth > 0) {
      if (raw[j] === '{') depth += 1;
      if (raw[j] === '}') depth -= 1;
      j += 1;
    }
    const header = raw.slice(at, open).trim();
    block(raw.slice(open + 1, j - 1), header);
    i = j;
  }
  block(rest, '');
  return { rules: out, raw };
}

function value(rules: Rule[], selector: string, prop: string, media = ''): string | undefined {
  const sel = selector.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const med = media.replace(/\s+/g, '');
  let v: string | undefined;
  for (const r of rules) if (r.selector === sel && (media === '' || r.media.replace(/\s+/g, '') === med) && r.decls.has(prop)) v = r.decls.get(prop);
  return v;
}

describe('xc-customer-tokens.css (OL-5d #6456)', () => {
  let rules: Rule[] = [];
  let raw = '';

  beforeAll(async () => {
    const specifier = 'node:fs';
    const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
    const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
    const cssText = fs.readFileSync(here + 'xc-customer-tokens.css', 'utf8');
    const parsed = parseCss(cssText);
    rules = parsed.rules;
    raw = parsed.raw;
  });

  it('(a) .xc-customer.xc-theme, .xc-customer .xc-theme sets --xc-bg: var(--xc-gray-50), --xc-radius-md: 10px, --xc-radius-lg: 12px', () => {
    for (const sel of ['.xc-customer.xc-theme', '.xc-customer .xc-theme']) {
      expect(value(rules, sel, '--xc-bg'), `${sel} --xc-bg`).toBe('var(--xc-gray-50)');
      expect(value(rules, sel, '--xc-radius-md'), `${sel} --xc-radius-md`).toBe('10px');
      expect(value(rules, sel, '--xc-radius-lg'), `${sel} --xc-radius-lg`).toBe('12px');
    }
  });

  it('(b) .xc-c-primary, .xc-c-signin-needed and body:has(.xc-customer) exist', () => {
    expect(rules.some((r) => r.selector === '.xc-c-primary'), '.xc-c-primary exists').toBe(true);
    expect(rules.some((r) => r.selector === '.xc-c-signin-needed'), '.xc-c-signin-needed exists').toBe(true);
    expect(rules.some((r) => r.selector === 'body:has(.xc-customer)'), 'body:has(.xc-customer) exists').toBe(true);
  });

  it('(c) @layer xc-customer-base exists', () => {
    expect(raw).toMatch(/@layer\s+xc-customer-base/);
    expect(rules.some((r) => r.media.includes('xc-customer-base')), 'rules inside @layer xc-customer-base').toBe(true);
  });
});
