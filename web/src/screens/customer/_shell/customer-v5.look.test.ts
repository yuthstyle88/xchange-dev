// OL-1R-v5 (#6528): rate freshness colour and header title gap at 375px
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

type Rule = { atRule: string; selector: string; decls: Map<string, string> };

function parseCss(text: string): Rule[] {
  const raw = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const block = (body: string, atRule: string) => {
    const re = /([^{}]+)\{([^{}]*)\}/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(body)) !== null) {
      const decls = new Map<string, string>();
      for (const d of m[2]!.split(';')) {
        const k = d.indexOf(':');
        if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
      }
      for (const sel of m[1]!.split(',')) out.push({ atRule, selector: normSel(sel), decls });
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
  return out;
}

function value(rules: Rule[], selector: string, prop: string, atRule = ''): string | undefined {
  const sel = selector.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const atNorm = atRule.replace(/\s+/g, '');
  for (const r of rules) {
    if (r.selector === sel && (atRule === '' ? r.atRule === '' : r.atRule.replace(/\s+/g, '').includes(atNorm)) && r.decls.has(prop)) {
      return r.decls.get(prop);
    }
  }
  return undefined;
}

describe('customer-v5.look (OL-1R-v5 #6528)', () => {
  let tokenRules: Rule[] = [];
  let shellRules: Rule[] = [];

  beforeAll(async () => {
    const fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
    const path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
    const here = path.dirname(new URL(import.meta.url).pathname);
    tokenRules = parseCss(fs.readFileSync(path.join(here, 'xc-customer-tokens.css'), 'utf8'));
    shellRules = parseCss(fs.readFileSync(path.join(here, 'shell.css'), 'utf8'));
  });

  it('row 1: .xc-customer .xc-rate-freshness sets color: #6b7280 in xc-customer-tokens.css', () => {
    expect(value(tokenRules, '.xc-customer .xc-rate-freshness', 'color')).toBe('#6b7280');
  });

  it('row 2: <= 760px customer title restores margin-left: 10px while column-gap remains 0 in shell.css', () => {
    const media = '(max-width: 760px)';
    expect(value(shellRules, '.xc-customer .xc-shell-header .xc-shell-title', 'margin-left', media)).toBe('10px');
    expect(value(shellRules, '.xc-customer .xc-shell-header', 'column-gap', media)).toBe('0');
  });
});
