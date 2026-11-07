import { describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(path: string, encoding: string): string;
}

interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

const PHONE = '(max-width:760px)';

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

async function readSibling(name: string): Promise<string> {
  const fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
  const path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
  const here = path.dirname(new URL(import.meta.url).pathname);
  return fs.readFileSync(path.join(here, name), 'utf8');
}

async function sheet(): Promise<Rule[]> {
  return parseCss(await readSibling('shell.css'));
}

function value(rules: Rule[], selector: string, prop: string, media = ''): string | undefined {
  let v: string | undefined;
  for (const r of rules) if (r.selector === selector && r.media === media && r.decls.has(prop)) v = r.decls.get(prop);
  return v;
}

describe('AppShell branch name overflow at phone widths (V3-3, #6511)', () => {
  it('at <= 760px, .account-bar button has min-width: 0, overflow: hidden, and text-overflow: ellipsis', async () => {
    const rules = await sheet();
    expect(value(rules, '.account-bar button', 'min-width', PHONE), 'min-width on phone').toBe('0');
    expect(value(rules, '.account-bar button', 'overflow', PHONE), 'overflow on phone').toBe('hidden');
    expect(value(rules, '.account-bar button', 'text-overflow', PHONE), 'text-overflow on phone').toBe('ellipsis');
    expect(value(rules, '.account-bar button', 'max-width', PHONE), 'max-width on phone').toBe('100%');
  });

  it('above 760px, .account-bar button overflow rules remain unchanged', async () => {
    const rules = await sheet();
    expect(value(rules, '.account-bar button', 'overflow', ''), 'overflow above 760px').toBeUndefined();
    expect(value(rules, '.account-bar button', 'text-overflow', ''), 'text-overflow above 760px').toBeUndefined();
    expect(value(rules, '.account-bar button', 'min-width', ''), 'min-width above 760px').toBeUndefined();
    expect(value(rules, '.account-bar button', 'white-space', ''), 'white-space above 760px').toBe('nowrap');
  });
});
