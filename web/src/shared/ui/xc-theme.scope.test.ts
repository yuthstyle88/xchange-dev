// A1-a (#6517): limit xc-theme.css element rules to staff roots, keeping customer pages unchanged.
// Pins rows of the behaviour table:
// 1. Every rule in xc-theme.css that sets a non-custom property starts with .xc-theme:where(:not(.xc-customer *))
// 2. Every prefixed selector contains :where(
// 3. Every custom property is declared on plain .xc-theme
import { describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

function splitSelectors(selectorText: string): string[] {
  const result: string[] = [];
  let depth = 0;
  let current = '';
  for (let i = 0; i < selectorText.length; i++) {
    const ch = selectorText[i];
    if (ch === '(') depth++;
    else if (ch === ')') depth--;
    if (ch === ',' && depth === 0) {
      result.push(current);
      current = '';
    } else {
      current += ch;
    }
  }
  if (current) result.push(current);
  return result;
}

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
      for (const sel of splitSelectors(m[1]!)) out.push({ media, selector: normSel(sel), decls });
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

let fs: NodeFs;
let path: NodePath;
let here: string;

async function readSibling(name: string): Promise<string | undefined> {
  if (!fs) {
    fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
    path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
    here = path.dirname(new URL(import.meta.url).pathname);
  }
  const file = path.join(here, name);
  return fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : undefined;
}

async function sheet(): Promise<Rule[]> {
  const text = await readSibling('xc-theme.css');
  expect(text, 'web/src/shared/ui/xc-theme.css exists').toBeDefined();
  return parseCss(text!);
}

describe('xc-theme.css selector scope (A1-a #6517)', () => {
  it('row 1: every rule in xc-theme.css that sets a non-custom property starts with .xc-theme:where(:not(.xc-customer *))', async () => {
    const rules = await sheet();
    const elementRules = rules.filter((r) => [...r.decls.keys()].some((k) => !k.startsWith('--')));
    expect(elementRules.length).toBeGreaterThan(0);
    for (const r of elementRules) {
      expect(
        r.selector.startsWith('.xc-theme:where(:not(.xc-customer *))'),
        `Selector "${r.selector}" should start with ".xc-theme:where(:not(.xc-customer *))"`,
      ).toBe(true);
    }
  });

  it('every prefixed selector contains :where(', async () => {
    const rules = await sheet();
    const elementRules = rules.filter((r) => [...r.decls.keys()].some((k) => !k.startsWith('--')));
    expect(elementRules.length).toBeGreaterThan(0);
    for (const r of elementRules) {
      expect(r.selector.includes(':where('), `Selector "${r.selector}" should contain ":where("`).toBe(true);
    }
  });

  it('row 2: every custom property is declared on plain .xc-theme', async () => {
    const rules = await sheet();
    const tokenRules = rules.filter((r) => [...r.decls.keys()].some((k) => k.startsWith('--')));
    expect(tokenRules.length).toBeGreaterThan(0);
    for (const r of tokenRules) {
      expect(r.selector, `Custom property rules must be on plain .xc-theme`).toBe('.xc-theme');
    }
  });
});
